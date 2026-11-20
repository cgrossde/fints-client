//! End-to-end behaviour against the synthetic test bank
//! (`fints-client-testbank`) through `jobs`: system ID reuse and re-sync,
//! decoupled login, touchdown paging with a TAN on every page, the tolerant
//! quirks against answers broken on purpose, and the 9075 retry.

use std::sync::Arc;
use std::time::Duration;

use fints_client::codec::defs::{HkIdn2, HkTan7};
use fints_client::jobs::{self, BankClient, Credentials, Format, Opened, Range, Step};
use fints_client::quirks::QuirkChain;
use fints_client::return_code::Scope;
use fints_client::transport::{HttpTransport, SharedTransport};
use fints_client::{Blz, NoticeDialog, Pin, ProductId, SessionState, SystemId, UserId};
use fints_client_testbank::{
    self as testbank, BankModel, Booking, HttpBank, LoginSca, TanRule, TestBank, date, tamper,
};
use rust_decimal::Decimal;

fn product() -> ProductId {
    ProductId::new("TEST-123")
}

fn creds(m: &BankModel) -> Credentials {
    Credentials::new(UserId::new(&m.customer.user_id), Pin::new(&m.customer.pin))
}

fn fresh(m: &BankModel) -> SessionState {
    SessionState::new(&Blz::new(&m.blz), &UserId::new(&m.customer.user_id))
}

fn account(bank: &HttpBank) -> fints_client::Account {
    let m = bank.model();
    fints_client::Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap()
}

fn done<T: std::fmt::Debug>(step: Step<T>) -> T {
    match step {
        Step::Done(d) => d.value,
        other => panic!("expected Done, got {other:?}"),
    }
}

/// Open a dialog with `state`; the login must need no TAN.
async fn open_with(
    client: &BankClient,
    bank: &HttpBank,
    state: &SessionState,
) -> fints_client::Result<(jobs::Dialog, Opened)> {
    let (dialog, step) = jobs::open(client, &creds(&bank.model()), state).await?;
    match step {
        Step::Done(d) => Ok((dialog, d.value)),
        other => panic!("the login is exempt, got {other:?}"),
    }
}

/// A fresh session, login exempt from SCA.
async fn open_fresh(bank: &HttpBank) -> (jobs::Dialog, Opened) {
    open_with(&bank.client(&product()), bank, &fresh(&bank.model()))
        .await
        .expect("open")
}

/// Answers every `NeedTan` of a decoupled challenge by polling at once.
async fn poll_to_done<T: jobs::JobValue + std::fmt::Debug>(
    dialog: &mut jobs::Dialog,
    mut step: Step<T>,
) -> T {
    loop {
        match step {
            Step::Done(d) => return d.value,
            Step::NeedTan(_, resume) => step = dialog.poll(resume).await.expect("poll"),
            other => panic!("{other:?}"),
        }
    }
}

// ── System ID ───────────────────────────────────────────────────────────

#[tokio::test]
async fn second_run_reuses_system_id() {
    let bank = TestBank::new(BankModel::default()).serve().await;

    let (dialog, _) = open_fresh(&bank).await;
    let first = dialog.end().await.expect("end");
    assert_eq!(first.system_id.as_str(), "TESTSYS0001");
    let before = bank.requests().len();

    let (dialog, _) = open_with(&bank.client(&product()), &bank, &first)
        .await
        .expect("second run");
    let second = dialog.end().await.expect("end");
    assert_eq!(second.system_id, first.system_id);

    let requests = bank.requests();
    let second_run = &requests[before..];
    assert!(second_run.iter().all(|r| !r.has("HKSYN")), "no sync dialog");
    let idn: HkIdn2 = second_run[0].get().expect("HKIDN");
    assert_eq!(idn.system_id.as_str(), "TESTSYS0001");
    assert_eq!(bank.issued_system_ids(), ["TESTSYS0001"]);
    bank.assert_clean();
}

#[tokio::test]
async fn rejected_system_id_triggers_a_new_sync() {
    let bank = TestBank::new(BankModel::default()).serve().await;

    // A complete session (BPD, UPD) whose system ID the bank never issued.
    let (dialog, _) = open_fresh(&bank).await;
    let stale = dialog
        .end()
        .await
        .expect("end")
        .with_system_id(&SystemId::new("STALESYS0001"));
    let before = bank.requests().len();

    let (dialog, _) = open_with(&bank.client(&product()), &bank, &stale)
        .await
        .expect("re-sync after 9391");
    let state = dialog.end().await.expect("end");
    assert_eq!(
        state.system_id.as_str(),
        "TESTSYS0002",
        "a new ID from HKSYN"
    );

    let requests = bank.requests();
    let rerun = &requests[before..];
    let idn: HkIdn2 = rerun[0].get().expect("HKIDN");
    assert_eq!(
        idn.system_id.as_str(),
        "STALESYS0001",
        "first try uses the stored ID"
    );
    assert!(
        rerun.iter().any(|r| r.has("HKSYN")),
        "then the sync dialog runs"
    );
    assert_eq!(bank.issued_system_ids(), ["TESTSYS0001", "TESTSYS0002"]);
    bank.assert_clean();
}

// ── Decoupled login ─────────────────────────────────────────────────────

/// The bank advertises 30 s waits; with `bank_waits` off the run polls
/// after the options' 10 ms, so three polls neither sleep 90 s nor run
/// into the timeout.
#[tokio::test]
async fn replay_run_ignores_the_banks_decoupled_waits() {
    use fints_client_testbank::replay::{RunOptions, standard_run};
    let m = BankModel {
        tan_methods: vec![testbank::TanMethod::decoupled(902, "Test-App", 3)],
        hitans_version: 7,
        login_sca: LoginSca::Tan,
        decoupled_wait_secs: 30,
        ..BankModel::default()
    };
    let bank = TestBank::new(m).serve().await;
    let m = bank.model();
    let (user, pin) = (UserId::new(&m.customer.user_id), Pin::new(&m.customer.pin));
    let opts = RunOptions {
        bank_waits: false,
        first_wait: Duration::from_millis(10),
        next_wait: Duration::from_millis(10),
        timeout: Duration::from_secs(5),
        ..RunOptions::new(30)
    };
    let started = std::time::Instant::now();
    let report = standard_run(&bank.client(&product()), &user, &pin, &opts)
        .await
        .expect("run");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(report.errors().is_empty());
    let polls = bank
        .requests()
        .iter()
        .flat_map(|r| r.all::<HkTan7>())
        .filter(|t| t.process.as_str() == "S")
        .count();
    assert_eq!(polls, 3, "the bank approves on the third poll");
    bank.assert_clean();
}

#[tokio::test]
async fn decoupled_login_is_approved_after_three_polls() {
    let m = BankModel {
        tan_methods: vec![testbank::TanMethod::decoupled(902, "Test-App", 3)],
        hitans_version: 7,
        login_sca: LoginSca::Tan,
        ..BankModel::default()
    };
    let bank = TestBank::new(m).serve().await;
    let m = bank.model();

    let (mut dialog, step) = jobs::open(&bank.client(&product()), &creds(&m), &fresh(&m))
        .await
        .expect("open");
    assert!(
        matches!(&step, Step::NeedTan(c, _) if matches!(c.kind, jobs::TanKind::Decoupled { .. })),
        "{step:?}"
    );
    poll_to_done(&mut dialog, step).await;

    let balance = done(
        jobs::balance(&mut dialog, &account(&bank))
            .await
            .expect("balance"),
    );
    assert_eq!(
        balance.map(|b| b.amount.to_string()).as_deref(),
        Some("1654.33")
    );
    dialog.end().await.expect("end");

    let polls: Vec<HkTan7> = bank
        .requests()
        .iter()
        .flat_map(|r| r.all::<HkTan7>())
        .filter(|t| t.process.as_str() == "S")
        .collect();
    assert_eq!(polls.len(), 3, "the bank approves on the third poll");
    assert!(
        polls
            .iter()
            .all(|t| t.order_reference.as_ref().map(|r| r.as_str()) == Some("TASK0001"))
    );
    bank.assert_clean();
}

// ── Touchdown ───────────────────────────────────────────────────────────

#[tokio::test]
async fn touchdown_pages_carry_hktan_and_the_last_point() {
    let mut m = BankModel::default();
    let bookings = (1..=7)
        .map(|i| {
            Booking::new(
                date(2026, 2, i),
                &format!("-{i}.25"),
                &format!("SVWZ+Testbuchung {i}"),
            )
        })
        .collect();
    let number = m.accounts[0].number.clone();
    m.accounts[0] = testbank::Account::giro(&m.blz, &number, "Testkundin Eins")
        .with_bookings(Decimal::new(80_000, 2), bookings);
    let kaz = m.job_mut("HKKAZ");
    kaz.page_size = Some(3);
    kaz.hipins_needs_tan = true;
    let bank = TestBank::new(m).serve().await;

    let (mut dialog, _) = open_fresh(&bank).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&bank), years(), Format::Mt940)
            .await
            .expect("transactions"),
    );
    dialog.end().await.expect("end");
    assert_eq!(statement.booked.len(), 7);
    assert_eq!(statement.booked[6].amount.to_string(), "-7.25");

    let pages: Vec<_> = bank
        .requests()
        .into_iter()
        .filter(|r| r.has("HKKAZ"))
        .collect();
    assert_eq!(pages.len(), 3, "7 bookings, 3 per page");
    assert!(pages.iter().all(|p| p.has("HKTAN")), "HKTAN on every page");
    assert!(pages[1].text().contains("TD-3"));
    assert!(pages[2].text().contains("TD-6"));
    bank.assert_clean();
}

/// Ten years back to today (a window that holds the test bank's bookings).
fn years() -> Range {
    let today = jiff::ZonedDateTime::now().date();
    Range::new(today - jiff::Span::new().days(3650), today)
}

#[tokio::test]
async fn in_process_bank_needs_no_server() {
    let handle = TestBank::new(BankModel::default()).handle();
    let m = handle.model();
    let account =
        fints_client::Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap();
    let client = handle.client(&product());

    let (mut dialog, step) = jobs::open(&client, &creds(&m), &fresh(&m))
        .await
        .expect("open");
    done(step);
    let balance = done(jobs::balance(&mut dialog, &account).await.expect("balance"));
    let statement = done(
        jobs::transactions(&mut dialog, &account, years(), Format::Mt940)
            .await
            .expect("transactions"),
    );
    dialog.end().await.expect("end");
    assert!(balance.is_some(), "balance");
    assert!(!statement.booked.is_empty(), "transactions");
    assert!(handle.requests().iter().any(|r| r.has("HKSAL")));
    assert!(handle.requests().iter().any(|r| r.has("HKKAZ")));
    handle.assert_clean();
}

// ── Tolerant quirks against answers broken on purpose ───────────────────

/// Opens a dialog (through `quirks` when given), fetches the balance and
/// ends the dialog.
async fn balance(
    bank: &HttpBank,
    quirks: Option<fn() -> QuirkChain>,
) -> fints_client::Result<Decimal> {
    let client = match quirks {
        Some(q) => bank.client(&product()).with_quirks(q),
        None => bank.client(&product()),
    };
    let (mut dialog, _) = open_with(&client, bank, &fresh(&bank.model())).await?;
    let amount = done(jobs::balance(&mut dialog, &account(bank)).await?)
        .expect("a balance")
        .amount;
    dialog.end().await?;
    Ok(amount)
}

#[tokio::test]
async fn default_quirks_read_a_missing_message_reference() {
    let bank = TestBank::new(BankModel::default())
        .tamper_raw(|_, msg| tamper::drop_message_reference(msg))
        .serve()
        .await;
    assert_eq!(balance(&bank, None).await.unwrap().to_string(), "1654.33");
    bank.assert_clean();
}

#[tokio::test]
async fn default_quirks_read_an_empty_header_version() {
    let bank = TestBank::new(BankModel::default())
        .tamper_raw(|_, msg| tamper::header_version(msg, ""))
        .serve()
        .await;
    assert_eq!(balance(&bank, None).await.unwrap().to_string(), "1654.33");
    bank.assert_clean();
}

#[tokio::test]
async fn without_quirks_an_empty_header_version_breaks_the_dialog() {
    let bank = TestBank::new(BankModel::default())
        .tamper_raw(|_, msg| tamper::header_version(msg, ""))
        .serve()
        .await;
    // Without the header-version repair the client does not take the
    // dialog ID from the answer, so its next message claims a new dialog
    // ("0") with message number 2. The bank refuses it.
    let result = balance(&bank, Some(QuirkChain::none)).await;
    assert!(result.is_err(), "{result:?}");
    let violations = bank.violations();
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(
        violations[0].message.contains("starts with message 1"),
        "{violations:?}"
    );
}

#[tokio::test]
async fn default_quirks_read_status_after_data_and_odd_numbering() {
    let bank = TestBank::new(BankModel::default())
        .tamper_clear(|_, segs| tamper::renumber(tamper::status_after_data(segs), 5))
        .serve()
        .await;
    assert_eq!(balance(&bank, None).await.unwrap().to_string(), "1654.33");
    bank.assert_clean();
}

// ── Retry with HKTAN on 9075 ───────────────────────────────────

/// The bank needs HKTAN for HKSAL although HIPINS says it does not; it
/// answers the order without HKTAN with 9075 (as Consorsbank does).
fn consors_like() -> BankModel {
    let mut m = BankModel::default();
    let sal = m.job_mut("HKSAL");
    sal.hipins_needs_tan = false;
    sal.server_tan = TanRule::RequiredExempt;
    m
}

fn sal_requests(bank: &HttpBank) -> Vec<testbank::Request> {
    bank.requests()
        .into_iter()
        .filter(|r| r.has("HKSAL"))
        .collect()
}

/// Login, one balance job, end: the job's result and the stored state.
async fn balance_run(
    bank: &HttpBank,
    client: &BankClient,
    state: Option<&SessionState>,
) -> (
    fints_client::Result<Option<fints_client::AccountBalance>>,
    SessionState,
) {
    let fresh_state = fresh(&bank.model());
    let (mut dialog, _) = open_with(client, bank, state.unwrap_or(&fresh_state))
        .await
        .expect("open");
    let r = jobs::balance(&mut dialog, &account(bank)).await.map(done);
    (r, dialog.end().await.expect("end"))
}

#[tokio::test]
async fn order_is_retried_with_hktan_after_9075() {
    let bank = TestBank::new(consors_like()).serve().await;
    let (r, state) = balance_run(&bank, &bank.client(&product()), None).await;
    assert!(r.expect("retried with HKTAN").is_some());
    let sal = sal_requests(&bank);
    assert_eq!(sal.len(), 2, "one try, one retry");
    assert!(!sal[0].has("HKTAN"));
    assert!(sal[1].has("HKTAN"), "retry carries HKTAN");
    assert_eq!(
        state.tan_required_learned,
        vec![fints_client::SegmentType::new("HKSAL")]
    );
    bank.assert_clean();
}

#[tokio::test]
async fn learned_tan_requirement_is_sent_at_once_next_run() {
    let bank = TestBank::new(consors_like()).serve().await;
    let client = bank.client(&product());
    let (_, state) = balance_run(&bank, &client, None).await;
    let json = state.to_json().unwrap();
    let state = SessionState::from_json(&json).unwrap();
    let (r, _) = balance_run(&bank, &client, Some(&state)).await;
    assert!(r.expect("second run").is_some());
    let sal = sal_requests(&bank);
    assert_eq!(sal.len(), 3, "two in run 1, exactly one in run 2");
    assert!(sal[2].has("HKTAN"), "learned: HKTAN at once");
    bank.assert_clean();
}

#[tokio::test]
async fn no_second_retry_when_hktan_does_not_help() {
    let bank = TestBank::new(BankModel::default())
        .on("HKSAL", |_, _| {
            Some(testbank::Answer::error("9075", "Starke Authentifizierung"))
        })
        .serve()
        .await;
    let (r, state) = balance_run(&bank, &bank.client(&product()), None).await;
    assert!(r.is_err());
    assert_eq!(sal_requests(&bank).len(), 2, "exactly one retry");
    assert!(state.tan_required_learned.is_empty(), "nothing learned");
}

#[tokio::test]
async fn sca_required_with_dialog_abort_is_not_retried() {
    let bank = TestBank::new(BankModel::default())
        .on("HKSAL", |_, _| {
            Some(testbank::Answer {
                message: vec![testbank::Feedback::new("9800", "Dialog abgebrochen")],
                ..testbank::Answer::error("9075", "Starke Authentifizierung")
            })
        })
        .serve()
        .await;
    let (r, state) = balance_run(&bank, &bank.client(&product()), None).await;
    assert!(r.is_err());
    assert_eq!(sal_requests(&bank).len(), 1, "no retry");
    assert_eq!(
        state.tan_required_learned,
        vec![fints_client::SegmentType::new("HKSAL")]
    );
}

/// A client for `bank` with per-order HKTAN overrides, as a profile in
/// `banks` would give them.
fn client_with(
    bank: &HttpBank,
    rules: &[(&str, fints_client::protocol::TanOverride)],
) -> BankClient {
    let config = rules.iter().fold(bank.bank_config(), |c, (code, rule)| {
        c.with_tan_override(code, *rule)
    });
    BankClient::new(config, product()).with_now(fints_client_testbank::TESTBANK_NOW)
}

#[tokio::test]
async fn override_never_is_not_retried_on_9075() {
    use fints_client::protocol::TanOverride;
    let bank = TestBank::new(consors_like()).serve().await;
    let client = client_with(&bank, &[("HKSAL", TanOverride::Never)]);
    let (r, state) = balance_run(&bank, &client, None).await;
    assert!(r.is_err());
    assert_eq!(sal_requests(&bank).len(), 1);
    assert!(state.tan_required_learned.is_empty());
}

// ── Profile overrides beat HIPINS ───────────────────────────────────────

#[tokio::test]
async fn override_always_sends_hktan_although_hipins_says_no() {
    use fints_client::protocol::TanOverride;
    let mut m = BankModel::default();
    m.job_mut("HKSAL").hipins_needs_tan = false;
    let bank = TestBank::new(m).serve().await;
    let client = client_with(&bank, &[("HKSAL", TanOverride::Always)]);

    let (r, _) = balance_run(&bank, &client, None).await;
    r.expect("balance");
    let sal = sal_requests(&bank);
    assert!(!sal.is_empty());
    assert!(
        sal.iter().all(|r| r.has("HKTAN")),
        "override Always sends HKTAN"
    );
    bank.assert_clean();
}

#[tokio::test]
async fn without_an_override_hipins_decides() {
    let mut m = BankModel::default();
    m.job_mut("HKSAL").hipins_needs_tan = false;
    let bank = TestBank::new(m).serve().await;
    let (r, _) = balance_run(&bank, &bank.client(&product()), None).await;
    r.expect("balance");
    assert!(sal_requests(&bank).iter().all(|r| !r.has("HKTAN")));
}

#[tokio::test]
async fn override_never_sends_no_hktan_although_hipins_says_yes() {
    use fints_client::protocol::TanOverride;
    let mut m = BankModel::default();
    m.accounts
        .push(testbank::Account::depot(&m.blz, "9999", "Test Holder", "-"));
    let wpd = m.job_mut("HKWPD");
    wpd.hipins_needs_tan = true;
    wpd.server_tan = TanRule::Exempt;
    let bank = TestBank::new(m).serve().await;
    let m = bank.model();
    let client = client_with(&bank, &[("HKWPD", TanOverride::Never)]);
    let depot = fints_client::Account::new(m.accounts[1].iban.as_deref().unwrap(), &m.bic).unwrap();

    let (mut dialog, _) = open_with(&client, &bank, &fresh(&m)).await.expect("open");
    let _ = jobs::holdings(&mut dialog, &depot, None).await;
    let wpd: Vec<_> = bank
        .requests()
        .into_iter()
        .filter(|r| r.has("HKWPD"))
        .collect();
    assert!(!wpd.is_empty(), "HKWPD was sent");
    assert!(
        wpd.iter().all(|r| !r.has("HKTAN")),
        "override Never sends no HKTAN"
    );
}

// ── Faults ──────────────────────────────────────────────────────────────

use fints_client::FinTSError;
use fints_client_testbank::Fault;

/// A complete session from a healthy first run.
async fn healthy_state(bank: &HttpBank) -> SessionState {
    let (dialog, _) = open_fresh(bank).await;
    dialog.end().await.expect("end")
}

/// Runs the login again with `state`; the error if it fails.
async fn login(bank: &HttpBank, state: &SessionState) -> fints_client::Result<SessionState> {
    login_with(bank, state, None).await
}

/// `login` through the given transport (default: the library's own).
async fn login_with(
    bank: &HttpBank,
    state: &SessionState,
    transport: Option<SharedTransport>,
) -> fints_client::Result<SessionState> {
    let client = match transport {
        Some(t) => bank.client(&product()).with_transport(t),
        None => bank.client(&product()),
    };
    let (dialog, _) = open_with(&client, bank, state).await?;
    dialog.end().await
}

/// Faults the first request with an HKIDN of the next login, expects an
/// error, then checks that the same stored state logs in at the healthy
/// bank (no new sync dialog).
async fn login_fault(fault: Fault) -> FinTSError {
    login_fault_with(fault, None).await
}

/// `login_fault` with the faulted login going through `transport`.
async fn login_fault_with(fault: Fault, transport: Option<SharedTransport>) -> FinTSError {
    let bank = TestBank::new(BankModel::default()).serve().await;
    let state = healthy_state(&bank).await;
    bank.inject(|r| r.has("HKIDN"), fault);
    let err = login_with(&bank, &state, transport)
        .await
        .expect_err("the fault must fail");
    let before = bank.requests().len();
    let again = login(&bank, &state).await.expect("a healthy bank works");
    assert_eq!(again.system_id, state.system_id, "state survives");
    assert!(bank.requests()[before..].iter().all(|r| !r.has("HKSYN")));
    err
}

#[tokio::test]
async fn http_500_is_a_typed_retryable_error() {
    let err = login_fault(Fault::Status(500)).await;
    assert!(
        matches!(err, FinTSError::Http { status: 500, .. }),
        "{err:?}"
    );
    assert!(err.is_retryable());
}

#[tokio::test]
async fn http_503_is_a_typed_retryable_error() {
    let err = login_fault(Fault::Status(503)).await;
    assert!(
        matches!(err, FinTSError::Http { status: 503, .. }),
        "{err:?}"
    );
    assert!(err.is_retryable());
}

#[tokio::test]
async fn silent_bank_ends_in_a_timeout_error() {
    let err = login_fault_with(
        Fault::Silence(Duration::from_secs(5)),
        Some(Arc::new(
            HttpTransport::with_timeout(Duration::from_secs(1)).unwrap(),
        )),
    )
    .await;
    assert!(matches!(err, FinTSError::Timeout), "{err:?}");
}

#[tokio::test]
async fn truncated_wire_answer_is_a_typed_error() {
    let err = login_fault(Fault::CutWire(200)).await;
    assert!(
        matches!(
            err,
            FinTSError::Parse(_) | FinTSError::InvalidResponse(_) | FinTSError::Transport { .. }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn answer_cut_before_any_return_code_is_an_error() {
    let err = login_fault(Fault::CutWire(100)).await;
    assert!(
        matches!(err, FinTSError::Parse(_) | FinTSError::InvalidResponse(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn truncated_base64_answer_is_a_typed_error() {
    let err = login_fault(Fault::CutBase64(41)).await;
    assert!(
        matches!(
            err,
            FinTSError::Parse(_) | FinTSError::InvalidResponse(_) | FinTSError::Transport { .. }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn invalid_base64_answer_is_a_typed_error() {
    let err = login_fault(Fault::InvalidBase64).await;
    assert!(
        matches!(
            err,
            FinTSError::Parse(_) | FinTSError::InvalidResponse(_) | FinTSError::Transport { .. }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn utf8_text_where_latin1_is_expected_is_a_typed_error_or_read() {
    // The answers carry "ü" ("Auftrag ausgeführt") as two UTF-8 bytes and
    // the HNHBK size is wrong. Either outcome is fine; a panic is not.
    let bank = TestBank::new(BankModel::default()).serve().await;
    let state = healthy_state(&bank).await;
    bank.inject(|r| r.has("HKIDN"), Fault::Utf8Text);
    let result = login(&bank, &state).await;
    if let Err(e) = &result {
        assert!(
            matches!(
                e,
                FinTSError::Parse(_)
                    | FinTSError::InvalidResponse(_)
                    | FinTSError::Transport { .. }
            ),
            "{e:?}"
        );
    }
    login(&bank, &state).await.expect("a healthy bank works");
}

fn january() -> Range {
    Range::new(date(2026, 1, 1), date(2026, 1, 31))
}

#[tokio::test]
async fn dialog_aborted_on_the_second_touchdown_page() {
    let mut m = BankModel::default();
    m.job_mut("HKKAZ").page_size = Some(1);
    m.accounts[0] = testbank::Account::giro(&m.blz, "1234567", "Testperson Abbruch").with_bookings(
        Decimal::new(10_000, 2),
        vec![
            Booking::new(date(2026, 1, 2), "-4.00", "Seite eins"),
            Booking::new(date(2026, 1, 3), "-6.00", "Seite zwei"),
        ],
    );
    let bank = TestBank::new(m).abort_dialog_on("HKKAZ", 2).serve().await;
    let state = healthy_state(&bank).await;

    let (mut dialog, _) = open_fresh(&bank).await;
    let err = jobs::transactions(&mut dialog, &account(&bank), january(), Format::Mt940)
        .await
        .expect_err("page two is refused");
    assert!(
        matches!(&err, FinTSError::Bank(_)),
        "typed bank error: {err:?}"
    );
    assert_eq!(bank.requests().iter().filter(|r| r.has("HKKAZ")).count(), 2);

    // The stored state still works against the same bank.
    let again = login(&bank, &state).await.expect("new dialog");
    assert_eq!(again.system_id, state.system_id);
    let (mut dialog, _) = open_fresh(&bank).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&bank), january(), Format::Mt940)
            .await
            .expect("all pages now"),
    );
    assert_eq!(statement.booked.len(), 2);
}

#[tokio::test]
async fn retry_with_decoupled_challenge_is_polled() {
    let mut m = BankModel {
        tan_methods: vec![testbank::TanMethod::decoupled(902, "Test-App", 2)],
        hitans_version: 7,
        ..BankModel::default()
    };
    let sal = m.job_mut("HKSAL");
    sal.hipins_needs_tan = false;
    sal.server_tan = TanRule::Required;
    let bank = TestBank::new(m).serve().await;

    let (mut dialog, _) = open_fresh(&bank).await;
    let step = jobs::balance(&mut dialog, &account(&bank))
        .await
        .expect("balance");
    assert!(matches!(step, Step::NeedTan(..)), "{step:?}");
    let balance = poll_to_done(&mut dialog, step).await;
    assert_eq!(
        balance.map(|b| b.amount.to_string()).as_deref(),
        Some("1654.33")
    );
    let state = dialog.end().await.expect("end");

    let sal = sal_requests(&bank);
    assert_eq!(sal.len(), 2, "one try, one retry");
    assert!(!sal[0].has("HKTAN"));
    assert!(sal[1].has("HKTAN"));
    let polls: Vec<HkTan7> = bank
        .requests()
        .iter()
        .flat_map(|r| r.all::<HkTan7>())
        .filter(|t| t.process.as_str() == "S")
        .collect();
    assert_eq!(polls.len(), 2, "approved on the second poll");
    assert_eq!(
        state.tan_required_learned,
        vec![fints_client::SegmentType::new("HKSAL")]
    );
    bank.assert_clean();
}

#[tokio::test]
async fn learned_hkkaz_carries_hktan_on_every_touchdown_page() {
    let mut m = BankModel::default();
    let bookings = (1..=7)
        .map(|i| {
            Booking::new(
                date(2026, 2, i),
                &format!("-{i}.25"),
                &format!("SVWZ+Testbuchung {i}"),
            )
        })
        .collect();
    let number = m.accounts[0].number.clone();
    m.accounts[0] = testbank::Account::giro(&m.blz, &number, "Testkundin Eins")
        .with_bookings(Decimal::new(80_000, 2), bookings);
    let kaz = m.job_mut("HKKAZ");
    kaz.page_size = Some(3);
    kaz.hipins_needs_tan = false;
    kaz.server_tan = TanRule::RequiredExempt;
    let bank = TestBank::new(m).serve().await;

    let run = |state: Option<SessionState>| {
        let bank = &bank;
        async move {
            let fresh_state = fresh(&bank.model());
            let (mut dialog, _) = open_with(
                &bank.client(&product()),
                bank,
                state.as_ref().unwrap_or(&fresh_state),
            )
            .await
            .expect("open");
            let r = done(
                jobs::transactions(&mut dialog, &account(bank), years(), Format::Mt940)
                    .await
                    .expect("transactions"),
            );
            (r, dialog.end().await.expect("end"))
        }
    };
    let (first, state) = run(None).await;
    assert_eq!(first.booked.len(), 7);
    assert_eq!(
        state.tan_required_learned,
        vec![fints_client::SegmentType::new("HKKAZ")]
    );
    let state = SessionState::from_json(&state.to_json().unwrap()).unwrap();
    let before = bank.requests().len();

    let (second, _) = run(Some(state)).await;
    assert_eq!(second.booked.len(), 7);
    let pages: Vec<_> = bank.requests()[before..]
        .iter()
        .filter(|r| r.has("HKKAZ"))
        .cloned()
        .collect();
    assert_eq!(pages.len(), 3, "7 bookings, 3 per page, no retries");
    assert!(pages.iter().all(|p| p.has("HKTAN")), "HKTAN on every page");
    assert!(pages[1].text().contains("TD-3"));
    bank.assert_clean();
}

/// A depot whose UPD gives no Kontoart and allows only HKWPD: the run
/// fetches its holdings and sends neither HKSAL nor HKKAZ for it.
#[tokio::test]
async fn depot_without_kontoart_gets_holdings_only() {
    use fints_client_testbank::replay::{RunOptions, standard_run};
    const MT535: &str = ":16R:GENL\r\n:98A::STAT//20260410\r\n:16S:GENL\r\n\
:16R:FIN\r\n:35B:ISIN XX0000TEST01\r\nTESTFONDS WELT\r\n:93B::AGGR//UNIT/40,\r\n:16S:FIN\r\n-";
    let mut m = BankModel::default();
    let mut depot = testbank::Account::depot(&m.blz, "0007000999", "Test Holder", MT535);
    depot.kind = None;
    m.accounts = vec![depot];
    let bank = TestBank::new(m).serve().await;
    let m = bank.model();
    let (user, pin) = (UserId::new(&m.customer.user_id), Pin::new(&m.customer.pin));

    let report = standard_run(&bank.client(&product()), &user, &pin, &RunOptions::new(30))
        .await
        .expect("run");
    let result = report.fetched[0].as_ref().expect("fetch");
    assert_eq!(result.holdings.len(), 1);
    assert!(result.balance.is_none());
    assert!(result.transactions.is_empty());
    assert_eq!(report.skipped[0], ["HKSAL", "HKKAZ"]);

    let sent = bank.requests();
    assert!(sent.iter().any(|r| r.has("HKWPD")));
    assert!(sent.iter().all(|r| !r.has("HKSAL") && !r.has("HKKAZ")));
    bank.assert_clean();
}

/// What the UPD allows decides the jobs, as `Capabilities` shows it: a
/// giro lists balance and transactions but not holdings; a depot without
/// Kontoart that lists HKWPD alone gets holdings only.
#[tokio::test]
async fn capabilities_follow_the_upd() {
    use fints_client::Allowance;
    use fints_client::jobs::JobKind;
    const MT535: &str = ":16R:GENL\r\n:16S:GENL\r\n-";
    let mut m = BankModel::default();
    let mut depot = testbank::Account::depot(&m.blz, "0007000999", "Test Holder", MT535);
    depot.kind = None;
    m.accounts.push(depot);
    let bank = TestBank::new(m).serve().await;
    let (dialog, opened) = open_fresh(&bank).await;
    assert_eq!(opened.accounts.len(), 2);

    let caps = dialog.capabilities(&[]);
    let allowance = |account: usize, job: JobKind| caps.accounts[account].job(job).unwrap().upd;
    assert_eq!(allowance(0, JobKind::Balance), Allowance::Yes);
    assert_eq!(allowance(0, JobKind::Transactions), Allowance::Yes);
    assert_eq!(allowance(0, JobKind::Holdings), Allowance::No);
    assert_eq!(allowance(1, JobKind::Balance), Allowance::No);
    assert_eq!(allowance(1, JobKind::Transactions), Allowance::No);
    assert_eq!(allowance(1, JobKind::Holdings), Allowance::Yes);
    // The same from the stored state.
    let state = dialog.end().await.expect("end");
    assert_eq!(state.capabilities(&[]), caps);
    bank.assert_clean();
}

/// comdirect gives its clearing account and the depot the same IBAN and
/// account number; only the subaccount differs (banks/comdirect.md
/// "Account list"). Each UPD entry keeps its own key, allowance and camt
/// decision, whether the accounts come from `Opened` or the UPD, and the
/// depot's holdings reach the depot.
#[tokio::test]
async fn accounts_sharing_iban_and_number_stay_apart() {
    use fints_client::Allowance;
    use fints_client::jobs::{JobKind, Support};
    let mut m = testbank::profiles::comdirect();
    let clearing = m.accounts[0].clone();
    let depot = &mut m.accounts[1];
    depot.iban = clearing.iban.clone();
    depot.number = clearing.number.clone();
    depot.subaccount = Some("01".into());
    let bank = TestBank::new(m).serve().await;
    let (mut dialog, opened) = open_fresh(&bank).await;

    let [c, d] = &opened.accounts[..] else {
        panic!("{:?}", opened.accounts)
    };
    assert_eq!(c.iban, d.iban);
    assert_eq!(c.number, d.number);
    assert_eq!(c.key.to_string(), "20041155/301");
    assert_eq!(d.key.to_string(), "20041155/301/01");
    assert!(d.is_depot() && !c.is_depot());

    let accounts = [c.account().unwrap(), d.account().unwrap()];
    for caps in [dialog.capabilities(&accounts), dialog.capabilities(&[])] {
        let [cc, dc] = &caps.accounts[..] else {
            panic!("{caps:?}")
        };
        assert_eq!((&cc.key, &dc.key), (&c.key, &d.key));
        assert!(cc.in_upd && dc.in_upd);
        let job = |a: &fints_client::jobs::AccountCapabilities, j| a.job(j).unwrap().clone();
        assert_eq!(job(cc, JobKind::Balance).upd, Allowance::Yes);
        assert_eq!(job(dc, JobKind::Balance).upd, Allowance::No);
        assert_eq!(job(cc, JobKind::Holdings).upd, Allowance::No);
        assert_eq!(job(dc, JobKind::Holdings).upd, Allowance::Yes);
        // camt for the clearing account, which lists HKCAZ; MT940 for the
        // depot, whose closed UPD does not.
        let code = |a| match job(a, JobKind::Transactions).support {
            Support::Available { code, .. } => code,
            other => panic!("{other:?}"),
        };
        assert_eq!((code(cc), code(dc)), ("HKCAZ", "HKKAZ"));
    }

    let holdings = done(
        jobs::holdings(&mut dialog, &accounts[1], None)
            .await
            .unwrap(),
    );
    assert_eq!(holdings.len(), 1);
    // The login's TAN method is the one the session state keeps.
    let state = dialog.end().await.expect("end");
    let chosen = opened.selected_tan_method.expect("a TAN method");
    assert_eq!(Some(&chosen.security_function), state.tan_method.as_ref());
    assert!(chosen.name.ends_with("TAN-Verfahren"));
    bank.assert_clean();
}

// ── Engine settings observed at Consorsbank ─────────────────────────────

/// `consors_like` with only a depot, which allows HKWPD alone, and the
/// disallowed-job code Consorsbank sends.
fn consors_depot_only() -> BankModel {
    let mut m = consors_like();
    m.not_allowed_code = "9380";
    m.not_allowed_text = "Keine Berechtigung für diese Auftragsart mit diesem Konto";
    m.accounts = vec![testbank::Account::depot(
        &m.blz,
        "0007000999",
        "Test Holder",
        ":16R:GENL\r\n:16S:GENL\r\n-",
    )];
    m
}

/// The kind of the bank error a job ended in.
fn bank_error<T>(r: fints_client::Result<T>) -> fints_client::return_code::Kind {
    match r.err() {
        Some(FinTSError::Bank(e)) => e.code().kind(),
        other => panic!("expected a bank error, got {other:?}"),
    }
}

/// The code a balance request on the bank's first account ends in.
async fn balance_error_code(bank: &HttpBank) -> String {
    let (mut dialog, _) = open_fresh(bank).await;
    let r = jobs::balance(&mut dialog, &account(bank)).await;
    dialog.end().await.expect("end");
    match r {
        Err(FinTSError::Bank(e)) => {
            let code = e.code();
            // The code names the order it refers to (HIRMS reference).
            assert!(
                matches!(&code.scope, fints_client::return_code::Scope::Segment { order: Some(o), .. } if o.as_str() == "HKSAL"),
                "{code:?}"
            );
            code.code.to_string()
        }
        Err(e) => panic!("expected a bank error, got {e:?}"),
        Ok(_) => panic!("expected a bank error, got a balance"),
    }
}

/// HKSAL to the depot: the TAN check comes first (9075), and the retry
/// with HKTAN gets the configured 9380 as a typed error.
#[tokio::test]
async fn disallowed_job_gets_9075_first_then_the_configured_code() {
    let bank = TestBank::new(consors_depot_only()).serve().await;
    assert_eq!(balance_error_code(&bank).await, "9380");
    let sal = sal_requests(&bank);
    assert_eq!(sal.len(), 2, "9075, then the retry with HKTAN");
    assert!(!sal[0].has("HKTAN") && sal[1].has("HKTAN"));
    bank.assert_clean();
}

/// With the permission checked first, the disallowed HKSAL gets 9380 at
/// once: no 9075, no retry.
#[tokio::test]
async fn permission_first_refuses_without_asking_for_a_tan() {
    let bank = TestBank::new(BankModel {
        check_order: testbank::CheckOrder::PermissionFirst,
        ..consors_depot_only()
    })
    .serve()
    .await;
    assert_eq!(balance_error_code(&bank).await, "9380");
    let sal = sal_requests(&bank);
    assert_eq!(sal.len(), 1, "no retry");
    assert!(!sal[0].has("HKTAN"));
    bank.assert_clean();
}

/// A bank that wants the national parts in the KTI (HKKAZ 7) and needs
/// HKTAN for HKKAZ, with HISPAS "national account allowed" as given.
fn kti_national_bank(hispas_national: bool) -> BankModel {
    let mut m = BankModel {
        kti_requires_national: true,
        hispas_national: Some(hispas_national),
        ..BankModel::default()
    };
    let kaz = m.job_mut("HKKAZ");
    kaz.hipins_needs_tan = false;
    kaz.server_tan = TanRule::RequiredExempt;
    m
}

async fn transactions_run(bank: &HttpBank) -> fints_client::Result<fints_client::jobs::Statement> {
    let (mut dialog, _) = open_fresh(bank).await;
    let r = jobs::transactions(&mut dialog, &account(bank), years(), Format::Mt940)
        .await
        .map(done);
    dialog.end().await.expect("end");
    r
}

fn kaz_requests(bank: &HttpBank) -> Vec<testbank::Request> {
    bank.requests()
        .into_iter()
        .filter(|r| r.has("HKKAZ"))
        .collect()
}

/// HISPAS forbids national details: the client sends `IBAN:BIC`, and the
/// bank answers the retry with HKTAN 9010 next to 3076.
#[tokio::test]
async fn kti_without_national_parts_gets_9010() {
    let bank = TestBank::new(kti_national_bank(false)).serve().await;
    assert_eq!(
        bank_error(transactions_run(&bank).await),
        fints_client::return_code::Kind::GeneralError
    );
    let kaz = kaz_requests(&bank);
    assert_eq!(kaz.len(), 2, "9075, then the retry with HKTAN");
    assert!(kaz[1].has("HKTAN"));
    assert!(!kaz[1].text().contains(":280:"), "IBAN:BIC only");
    bank.assert_clean();
}

/// HISPAS allows national details: the client sends the 6-part KTI and
/// gets the transactions.
#[tokio::test]
async fn kti_with_national_parts_gets_the_transactions() {
    let bank = TestBank::new(kti_national_bank(true)).serve().await;
    let r = transactions_run(&bank).await.expect("transactions");
    assert_eq!(r.booked.len(), 2);
    let kaz = kaz_requests(&bank);
    assert!(kaz.last().unwrap().text().contains(":280:12345678"));
    bank.assert_clean();
}

// ── Typed BPD/UPD model ─────────────────────────────────────────────────

/// After a login the session state's typed model agrees with what the bank
/// model serves: versions, HIPINS flags, the HISPAS flag, the accounts.
#[tokio::test]
async fn typed_bpd_and_upd_match_the_bank_model() {
    let model = BankModel {
        hispas_national: Some(true),
        ..BankModel::default()
    };
    let bank = TestBank::new(model).serve().await;
    let m = bank.model();

    let (dialog, _) = open_fresh(&bank).await;
    let state = dialog.end().await.expect("end");

    let bpd = state.bpd().expect("typed BPD");
    assert_eq!(u32::from(bpd.version), m.bpd_version);
    assert_eq!(
        bpd.bank_code.as_ref().map(|b| b.as_str()),
        Some(m.blz.as_str())
    );
    for (code, spec) in &m.jobs {
        assert_eq!(
            bpd.tan_required(code),
            Some(spec.hipins_needs_tan),
            "{code}"
        );
        let offered: Vec<u16> = bpd.job(code).expect(code).offered().collect();
        assert_eq!(offered, spec.versions, "{code} versions");
    }
    let sepa = bpd.sepa().expect("HISPAS");
    assert_eq!(Some(sepa.national_account_allowed), m.hispas_national);
    assert_eq!(bpd.tan_methods().count(), m.tan_methods.len());
    assert!(bpd.unparsed().is_empty(), "{:?}", bpd.unparsed());

    let upd = state.upd().expect("typed UPD");
    assert_eq!(u32::from(upd.version), m.upd_version);
    assert!(!upd.is_dialog_only());
    assert_eq!(upd.accounts().len(), m.accounts.len());
    for (typed, served) in upd.accounts().iter().zip(&m.accounts) {
        assert_eq!(
            typed.iban.as_ref().map(|i| i.as_str()),
            served.iban.as_deref()
        );
        assert_eq!(
            typed.account_number.as_deref(),
            Some(served.number.as_str())
        );
        assert_eq!(typed.account_type, served.kind);
    }
    bank.assert_clean();
}

// ── Dialog-only UPD ─────────────────────────────────────────────────────

/// UPD version 0 is dialog-only (Formals): the bank sends the UPD in every
/// dialog init, the client stores none and asks for it again.
#[tokio::test]
async fn dialog_only_upd_is_resent_and_not_stored() {
    let model = BankModel {
        upd_version: 0,
        ..BankModel::default()
    };
    let bank = TestBank::new(model).serve().await;

    let (dialog, opened) = open_fresh(&bank).await;
    assert!(!opened.accounts.is_empty(), "the UPD came in this dialog");
    let first = dialog.end().await.expect("end");
    assert!(first.upd().is_none(), "dialog-only UPD is not stored");
    assert_eq!(first.upd_version, 0);
    let before = bank.requests().len();

    let (dialog, opened) = open_with(&bank.client(&product()), &bank, &first)
        .await
        .expect("second run");
    assert!(!opened.accounts.is_empty(), "the UPD came again");
    let second = dialog.end().await.expect("end");
    assert!(second.upd().is_none());
    let vvb = bank.requests()[before..]
        .iter()
        .find(|r| r.has("HKVVB"))
        .expect("HKVVB")
        .get::<fints_client::codec::defs::HkVvb3>()
        .expect("HKVVB readable");
    assert_eq!(vvb.upd_version.value(), 0, "the client asks for the UPD");
    bank.assert_clean();
}

// ── Version negotiation ─────────────────────────────────────────────────

/// A bank whose BPD offers HISALS only in 3 and 4 and HICAZS only in 2:
/// versions the client does not implement.
fn old_versions_only() -> BankModel {
    let mut m = BankModel::default();
    *m.job_mut("HKSAL") = testbank::JobSpec::new(&[3, 4]);
    m.jobs.insert("HKCAZ", testbank::JobSpec::new(&[2]));
    m
}

fn requests_with(bank: &HttpBank, code: &str) -> usize {
    bank.requests().iter().filter(|r| r.has(code)).count()
}

#[tokio::test]
async fn balance_without_a_common_version_is_a_typed_error_and_sends_nothing() {
    let bank = TestBank::new(old_versions_only()).serve().await;
    let (mut dialog, _) = open_fresh(&bank).await;
    match jobs::balance(&mut dialog, &account(&bank)).await {
        Err(FinTSError::VersionMismatch(v)) => {
            assert_eq!(v.code.as_str(), "HKSAL");
            assert_eq!(v.offered, [3, 4]);
            assert!(!v.implemented.is_empty() && !v.implemented.contains(&3));
        }
        Ok(_) => panic!("expected VersionMismatch"),
        Err(e) => panic!("expected VersionMismatch, got {e}"),
    }
    assert_eq!(
        requests_with(&bank, "HKSAL"),
        0,
        "no HKSAL reached the bank"
    );
    bank.assert_clean();
}

#[tokio::test]
async fn camt_falls_back_to_mt940_when_hkcaz_has_no_common_version() {
    let bank = TestBank::new(old_versions_only()).serve().await;
    let (mut dialog, _) = open_fresh(&bank).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&bank), years(), Format::Auto)
            .await
            .expect("transactions"),
    );
    dialog.end().await.expect("end");
    assert!(!statement.booked.is_empty(), "MT940 transactions");
    assert_eq!(statement.format, fints_client::jobs::StatementFormat::Mt940);
    assert_eq!(
        requests_with(&bank, "HKCAZ"),
        0,
        "no HKCAZ reached the bank"
    );
    assert!(requests_with(&bank, "HKKAZ") >= 1);
    bank.assert_clean();
}

// ── Return codes: categories and notices ───────────────────────────────

/// A typed TAN the bank rejects (9941) and does not keep the order for is
/// a TAN error that needs the user.
#[tokio::test]
async fn wrong_typed_tan_is_a_tan_error_that_needs_the_user() {
    use fints_client::Category;
    let mut m = BankModel {
        login_sca: LoginSca::Tan,
        ..BankModel::default()
    };
    m.wrong_tan.kept = 0;
    let bank = TestBank::new(m).serve().await;
    let m = bank.model();
    let (mut dialog, step) = jobs::open(&bank.client(&product()), &creds(&m), &fresh(&m))
        .await
        .expect("open");
    let Step::NeedTan(_, resume) = step else {
        panic!("a TAN is required");
    };
    let err = dialog
        .submit_tan(resume, "000000")
        .await
        .expect_err("wrong TAN");
    assert_eq!(err.kind(), Category::Tan, "{err}");
    assert!(err.needs_user());
}

/// Warnings no protocol step consumes (3060, 3072 new access data) reach the
/// caller as notices of the step; 0030 (TAN required) stays internal. The
/// notices of the messages before the TAN come with the `Done` after it.
#[tokio::test]
async fn unconsumed_warnings_arrive_as_notices_on_the_step() {
    let mut m = BankModel {
        login_sca: LoginSca::Tan,
        ..BankModel::default()
    };
    m.customer.changed_user_id = Some("NEWUSER".into());
    let bank = TestBank::new(m).serve().await;
    let m = bank.model();
    let (mut dialog, step) = jobs::open(&bank.client(&product()), &creds(&m), &fresh(&m))
        .await
        .expect("open");
    let Step::NeedTan(_, resume) = step else {
        panic!("a TAN is required");
    };
    let Step::Done(done) = dialog.submit_tan(resume, "123456").await.expect("TAN") else {
        panic!("the TAN opens the dialog");
    };
    let codes: Vec<String> = done
        .notices
        .iter()
        .map(|n| n.code().unwrap().code.to_string())
        .collect();
    // The sync dialog and the login each carry the pair; notices are not
    // deduplicated, each says where it came from.
    assert_eq!(
        codes,
        ["3060", "3072", "3060", "3072"],
        "{:?}",
        done.notices
    );
    let origins: Vec<(NoticeDialog, bool)> = done
        .notices
        .iter()
        .map(|n| (n.dialog(), n.code().unwrap().scope == Scope::Message))
        .collect();
    assert_eq!(
        origins,
        [
            (NoticeDialog::Sync, true),
            (NoticeDialog::Sync, false),
            (NoticeDialog::Job, true),
            (NoticeDialog::Job, false),
        ],
        "{:?}",
        done.notices
    );
    assert_eq!(
        done.notices[1].code().unwrap().parameters,
        ["NEWUSER", "NEWUSER"]
    );
}

// ── State access ────────────────────────────────────────────────────────

#[tokio::test]
async fn state_with_only_a_stored_system_id_opens_without_sync() {
    let bank = TestBank::new(BankModel::default()).serve().await;
    let (dialog, _) = open_fresh(&bank).await;
    let first = dialog.end().await.expect("end");
    let before = bank.requests().len();

    // What a host builds after a state format change: no BPD, no UPD.
    let rebuilt = fresh(&bank.model()).with_system_id(&first.system_id);
    assert!(!rebuilt.can_skip_sync());
    let (dialog, _) = open_with(&bank.client(&product()), &bank, &rebuilt)
        .await
        .expect("open");
    let state = dialog.end().await.expect("end");

    let requests = bank.requests();
    let run = &requests[before..];
    assert!(run.iter().all(|r| !r.has("HKSYN")), "no sync dialog");
    let idn: HkIdn2 = run[0].get().expect("HKIDN");
    assert_eq!(idn.system_id.as_str(), first.system_id.as_str());
    assert_eq!(bank.issued_system_ids(), [first.system_id.as_str()]);
    assert_eq!(state.system_id, first.system_id);
    assert!(state.can_skip_sync(), "BPD and UPD are fetched");
    bank.assert_clean();
}

fn masked_json(state: &SessionState) -> serde_json::Value {
    let mut v: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("updated_at");
    v
}

#[tokio::test]
async fn snapshot_after_open_equals_the_state_from_end() {
    let bank = TestBank::new(BankModel::default()).serve().await;
    let (dialog, _) = open_fresh(&bank).await;
    let before = bank.requests().len();

    let snapshot = dialog.session_state();
    let again = dialog.session_state();
    assert_eq!(bank.requests().len(), before, "a snapshot sends nothing");
    assert_eq!(snapshot.system_id.as_str(), "TESTSYS0001");
    assert!(snapshot.can_skip_sync());
    assert_eq!(masked_json(&snapshot), masked_json(&again));

    let ended = dialog.end().await.expect("end");
    assert_eq!(masked_json(&snapshot), masked_json(&ended));
    bank.assert_clean();
}

#[tokio::test]
async fn a_job_changes_the_state_from_end_but_not_the_earlier_snapshot() {
    let bank = TestBank::new(consors_like()).serve().await;
    let (mut dialog, _) = open_fresh(&bank).await;
    let snapshot = dialog.session_state();
    assert!(snapshot.tan_required_learned.is_empty());

    let r = jobs::balance(&mut dialog, &account(&bank)).await.map(done);
    assert!(r.expect("retried with HKTAN").is_some());
    let later = dialog.session_state();
    let ended = dialog.end().await.expect("end");

    let hksal = vec![fints_client::SegmentType::new("HKSAL")];
    assert!(snapshot.tan_required_learned.is_empty(), "the old snapshot");
    assert_eq!(later.tan_required_learned, hksal);
    assert_eq!(ended.tan_required_learned, hksal);
    assert_eq!(masked_json(&later), masked_json(&ended));
    assert_eq!(snapshot.system_id, ended.system_id);
}

/// The TAN challenge of a balance request that needs a TAN, on a bank
/// whose one typed method is `method`.
async fn typed_challenge(method: testbank::TanMethod) -> jobs::TanChallenge {
    let mut m = BankModel {
        tan_methods: vec![method],
        hitans_version: 7,
        ..BankModel::default()
    };
    let sal = m.job_mut("HKSAL");
    sal.hipins_needs_tan = false;
    sal.server_tan = TanRule::Required;
    let bank = TestBank::new(m).serve().await;
    let (mut dialog, _) = open_fresh(&bank).await;
    let step = jobs::balance(&mut dialog, &account(&bank))
        .await
        .expect("balance");
    match step {
        Step::NeedTan(c, _) => c,
        other => panic!("expected NeedTan, got {other:?}"),
    }
}

#[tokio::test]
async fn challenge_markup_follows_the_hitans_flag_of_the_method_in_use() {
    let plain = typed_challenge(testbank::TanMethod::typed(921, "Test-TAN", "123456")).await;
    assert!(!plain.challenge_markup);

    let method = testbank::TanMethod::typed(921, "Test-TAN", "123456")
        .with_structured_challenge("Bitte <b>TAN</b> eingeben");
    let marked = typed_challenge(method).await;
    assert!(marked.challenge_markup);
    assert!(marked.challenge.as_str().contains("<b>"), "{marked:?}");
}
