//! Business orders in an open dialog: pagination, CAMT and depot requests,
//! per-request TAN and resuming with a stored system ID. All bank data is
//! synthetic.
//!
//! Touchdown paging, the decoupled order TAN and the stored-system-ID probe
//! run against the synthetic test bank; tests that need an exact sequence of
//! answers (or answers the bank model does not produce, such as CAMT or a
//! broken HITAN) use a [`Script`] of canned messages.

use fints_client::jobs::{self, BankClient, Credentials, Format, Range, Step, TanKind};
use fints_client::{
    Account, BankConfig, Blz, Pin, ProductId, SessionState, TransactionStatus, UserId,
};
use fints_client_testbank::{
    self as testbank, BankModel, Booking, HttpBank, HttpScript, LoginSca, Script, TanRule,
    TestBank, date,
};
use rust_decimal::Decimal;

/// Wrap response segments in a minimal FinTS message.
fn msg(body: &str) -> Vec<u8> {
    format!("HNHBK:1:3+000000000000+300+DLG-1+1'{body}HNHBS:9:1+1'").into_bytes()
}

/// A segment with one binary data element: `<head>+@len@<payload>'`.
fn binary_segment(head: &str, payload: &str) -> String {
    format!("{head}+@{}@{payload}'", payload.len())
}

/// Serve canned responses in order (the last one repeats).
async fn scripted_bank(responses: Vec<String>) -> HttpScript {
    Script::new(responses.iter().map(|r| msg(r)).collect())
        .serve()
        .await
}

/// The BPD a bank offering the order versions the client sends (HKSAL 7,
/// HKKAZ 7, HKCAZ 1, HKWPD 6, HKWDU 5) and the decoupled method 901 gives
/// (no HIPINS: every order gets HKTAN). Orders need a BPD to choose their
/// version.
const BPD: &str = "HIBPA:4:3:4+1+280:12030000+Testbank+1+1+300'\
    HISALS:5:7:4+1+1+1'HIKAZS:6:7:4+1+1+1'HICAZS:7:1:4+1+1+1'\
    HIWPDS:8:6:4+1+1+1'HIWDUS:9:5:4+1+1+0+N'\
    HITANS:10:7:4+1+1+0+N:N:1:901:2:MS2.0.0:Decoupled::App:::TAN:999:J:1:N:0:0:N:J:00:0:J:1:60:5:5:N:J'";
/// Dialog opened with that BPD.
const OPENED: &str = "HIRMG:2:2+0010::Nachricht entgegengenommen'\
    HIRMS:3:2:4+0020::Dialog initialisiert'\
    HIBPA:4:3:4+1+280:12030000+Testbank+1+1+300'\
    HISALS:5:7:4+1+1+1'HIKAZS:6:7:4+1+1+1'HICAZS:7:1:4+1+1+1'\
    HIWPDS:8:6:4+1+1+1'HIWDUS:9:5:4+1+1+0+N'\
    HITANS:10:7:4+1+1+0+N:N:1:901:2:MS2.0.0:Decoupled::App:::TAN:999:J:1:N:0:0:N:J:00:0:J:1:60:5:5:N:J'";
const IBAN: &str = "DE00120300009900012345";
const BIC: &str = "BYLADEM1001";

/// A state that skips the sync dialog: stored system ID, the `BPD` above,
/// method 901 selected.
fn scripted_state() -> SessionState {
    let json = serde_json::json!({
        "format": "fints-client-session/2",
        "blz": "12030000",
        "user_id": "test1",
        "system_id": "SYS;001",
        "bpd_version": 1,
        "upd_version": 0,
        "bpd": BPD,
        "upd": "",
        "tan_method": "901",
        "tan_medium": null,
        "allowed_security_functions": ["901"],
        "previous_user_id": null,
        "tan_required_learned": [],
        "updated_at": "2026-10-15T00:00:00Z",
    });
    SessionState::from_json(&json.to_string()).unwrap()
}

fn creds() -> Credentials {
    Credentials::new(UserId::new("test1"), Pin::new("12345"))
}

fn scripted_client(url: &str) -> BankClient {
    BankClient::new(
        BankConfig::new("Testbank", "12030000", "BYLADEM1001", url),
        ProductId::new("P"),
    )
    .with_now(fints_client_testbank::TESTBANK_NOW)
}

/// A dialog against a script whose first answer is the init response.
async fn open_scripted(url: &str) -> jobs::Dialog {
    let (dialog, step) = jobs::open(&scripted_client(url), &creds(), &scripted_state())
        .await
        .expect("open");
    assert!(matches!(step, Step::Done(_)), "expected an open dialog");
    dialog
}

fn mt940(lines: &[&str]) -> String {
    let mut text = vec![
        ":20:STARTUMS",
        ":25:12030000/9900012345",
        ":28C:0",
        ":60F:C260101EUR100,00",
    ];
    text.extend_from_slice(lines);
    text.push(":62F:C260102EUR90,00");
    text.join("\r\n") + "\r\n-"
}

// ── Against the test bank ───────────────────────────────────────────────

const BANK_USER: &str = "test1";
const BANK_PIN: &str = "12345";
const BANK_NUMBER: &str = "0009900012";

/// The test bank behind `open_dialog`: user test1, login exempt from SCA,
/// decoupled method 902 (HITANS 7) approved on the second poll, and one
/// current account with the given bookings.
fn open_dialog_model(bookings: Vec<Booking>) -> BankModel {
    let mut m = BankModel::default();
    m.customer.user_id = BANK_USER.into();
    m.customer.pin = BANK_PIN.into();
    m.customer.customer_id = BANK_USER.into();
    m.tan_methods = vec![testbank::TanMethod::decoupled(902, "App-Freigabe Test", 2)];
    m.hitans_version = 7;
    m.login_sca = LoginSca::Exempt;
    m.accounts = vec![
        testbank::Account::giro(&m.blz, BANK_NUMBER, "Testperson Dialog")
            .with_bookings(Decimal::new(10_000, 2), bookings),
    ];
    m
}

fn bank_account(bank: &HttpBank) -> Account {
    let m = bank.model();
    Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap()
}

/// The test bank's client and an open dialog (login exempt).
async fn open_test_bank_dialog(bank: &HttpBank) -> jobs::Dialog {
    let client = bank.client(&ProductId::new("P"));
    let state = SessionState::new(&Blz::new(&bank.model().blz), &UserId::new(BANK_USER));
    let (dialog, step) = jobs::open(&client, &creds(), &state).await.expect("open");
    assert!(matches!(step, Step::Done(_)), "login is exempt");
    dialog
}

fn done<T: std::fmt::Debug>(step: Step<T>) -> jobs::Completed<T> {
    match step {
        Step::Done(d) => d,
        other => panic!("expected Done, got {other:?}"),
    }
}

fn window() -> Range {
    Range::new(
        jiff::civil::date(2025, 1, 1),
        jiff::civil::date(2026, 1, 31),
    )
}

#[tokio::test]
async fn hktan_is_bundled_on_every_touchdown_page() {
    let mut m = open_dialog_model(vec![
        Booking::new(date(2026, 1, 2), "-4.00", "Seite eins"),
        Booking::new(date(2026, 1, 2), "-6.00", "Seite zwei"),
    ]);
    // HIPINS lists HKKAZ with J, so every page carries HKTAN; the bank
    // grants the exemption (3076) per page.
    m.job_mut("HKKAZ").page_size = Some(1);
    m.job_mut("HKKAZ").hipins_needs_tan = true;
    let bank = TestBank::new(m).serve().await;
    let mut dialog = open_test_bank_dialog(&bank).await;
    let account = bank_account(&bank);
    let opened = bank.requests().len();
    let statement = done(
        jobs::transactions(&mut dialog, &account, window(), Format::Mt940)
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(statement.booked.len(), 2);

    // The init message, then the two pages.
    let reqs = bank.requests()[opened - 1..].to_vec();
    assert_eq!(reqs.len(), 3);
    for page in &reqs[1..] {
        let text = page.text();
        assert!(page.has("HKKAZ"), "{text}");
        assert!(text.contains("+4+HKKAZ"), "HKTAN:4 on every page: {text}");
    }
    assert!(
        reqs[2].text().contains("TD-1"),
        "touchdown sent: {}",
        reqs[2].text()
    );
    bank.assert_clean();
}

#[tokio::test]
async fn camt_round_trip() {
    let booked = "<Document><BkToCstmrAcctRpt><Rpt><Acct><Ccy>EUR</Ccy></Acct><Ntry>\
        <Amt Ccy=\"EUR\">12.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>\
        <BookgDt><Dt>2026-01-05</Dt></BookgDt><ValDt><Dt>2026-01-04</Dt></ValDt>\
        <NtryDtls><TxDtls><Refs><MndtId>MANDATE-TEST-1</MndtId></Refs><Purp><Cd>SUBS</Cd></Purp></TxDtls></NtryDtls>\
        </Ntry></Rpt></BkToCstmrAcctRpt></Document>";
    let pending = "<Document><BkToCstmrAcctRpt><Rpt><Ntry>\
        <Amt Ccy=\"EUR\">3.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>\
        <ValDt><Dt>2026-01-06</Dt></ValDt></Ntry></Rpt></BkToCstmrAcctRpt></Document>";
    let hicaz = format!(
        "HIRMG:2:2+0010::ok'HIRMS:3:2:4+0020::Auftrag ausgefuehrt'\
         HICAZ:4:1:4+{IBAN}:{BIC}+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+@{}@{booked}+@{}@{pending}'",
        booked.len(),
        pending.len()
    );
    let script = scripted_bank(vec![OPENED.into(), hicaz]).await;
    let mut dialog = open_scripted(script.url()).await;
    let account = Account::new(IBAN, BIC).unwrap();
    let range = Range::new(
        jiff::civil::date(2026, 1, 1),
        jiff::civil::date(2026, 1, 31),
    );
    let statement = done(
        jobs::transactions(&mut dialog, &account, range, Format::Camt)
            .await
            .unwrap(),
    )
    .value;
    let txs = &statement.booked;
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].date.to_string(), "2026-01-05");
    assert_eq!(txs[0].valuta_date.unwrap().to_string(), "2026-01-04");
    assert_eq!(txs[0].mandate_id.as_deref(), Some("MANDATE-TEST-1"));
    assert_eq!(txs[0].purpose_code.as_deref(), Some("SUBS"));
    let pending = statement.pending.reported().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, TransactionStatus::Pending);
    let reqs = script.requests();
    assert!(
        reqs[1].contains(&format!("HKCAZ:3:1+{IBAN}:{BIC}+urn?:iso")),
        "{}",
        reqs[1]
    );
}

// ── Per-request decoupled TAN in an open dialog ─────────────────────────

const ORDER_PUSH_SENT: &str = "HIRMG:2:2+3060::Teilweise Warnungen'\
    HIRMS:3:2:4+3955::Bitte bestaetigen Sie den Auftrag in Ihrer App'\
    HITAN:4:7:4+4++TASK-9+Bitte in der App freigeben'";
const CONFIRMED_NO_DATA: &str = "HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::Auftrag ausgefuehrt'";

fn hikaz_page(purpose: &str) -> String {
    format!(
        "HIRMG:2:2+0010::ok'HIRMS:3:2:4+0020::Auftrag ausgefuehrt'{}",
        binary_segment(
            "HIKAZ:4:7:4",
            &mt940(&[
                ":61:2601020102D4,00NMSCNONREF",
                &format!(":86:166?00TEST?20{purpose}"),
            ])
        )
    )
}

#[tokio::test]
async fn order_pauses_for_decoupled_tan_and_takes_data_from_confirmation() {
    let mut m = open_dialog_model(vec![Booking::new(
        date(2026, 1, 2),
        "-4.00",
        "Nach Freigabe",
    )]);
    m.job_mut("HKKAZ").hipins_needs_tan = true;
    m.job_mut("HKKAZ").server_tan = TanRule::Required;
    let bank = TestBank::new(m).serve().await;
    let mut dialog = open_test_bank_dialog(&bank).await;
    let account = bank_account(&bank);
    let opened = bank.requests().len();

    let step = jobs::transactions(&mut dialog, &account, window(), Format::Mt940)
        .await
        .unwrap();
    let Step::NeedTan(challenge, mut resume) = step else {
        panic!("expected a decoupled challenge");
    };
    assert!(matches!(challenge.kind, TanKind::Decoupled { .. }));
    let statement = loop {
        match dialog.poll(resume).await.unwrap() {
            Step::Done(d) => break d.value,
            Step::NeedTan(_, next) => resume = next,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(statement.booked.len(), 1);
    assert_eq!(
        statement.booked[0].purpose.as_deref(),
        Some("Nach Freigabe")
    );

    let reqs = bank.requests()[opened - 1..].to_vec();
    assert_eq!(reqs.len(), 4, "init, HKKAZ, two polls");
    for poll in &reqs[2..] {
        let text = poll.text();
        assert!(text.contains("+S++++TASK0001+"), "{text}");
        assert!(!poll.has("HKKAZ"), "{text}");
    }
    bank.assert_clean();
}

#[tokio::test]
async fn order_is_sent_again_when_confirmation_has_no_data() {
    let script = scripted_bank(vec![
        OPENED.into(),
        ORDER_PUSH_SENT.into(),
        CONFIRMED_NO_DATA.into(),
        hikaz_page("Zweiter Versuch"),
    ])
    .await;
    let mut dialog = open_scripted(script.url()).await;
    let account = Account::new(IBAN, BIC).unwrap();
    let Step::NeedTan(_, resume) =
        jobs::transactions(&mut dialog, &account, window(), Format::Mt940)
            .await
            .unwrap()
    else {
        panic!("expected a challenge");
    };
    let statement = done(dialog.poll(resume).await.unwrap()).value;
    assert_eq!(
        statement.booked[0].purpose.as_deref(),
        Some("Zweiter Versuch")
    );
    let reqs = script.requests();
    assert_eq!(reqs.len(), 4);
    assert!(reqs[3].contains("HKKAZ:"), "order resent: {}", reqs[3]);
}

#[tokio::test]
async fn the_library_does_not_poll_by_itself() {
    let script = scripted_bank(vec![OPENED.into(), ORDER_PUSH_SENT.into()]).await;
    let mut dialog = open_scripted(script.url()).await;
    let account = Account::new(IBAN, BIC).unwrap();
    let step = jobs::transactions(&mut dialog, &account, window(), Format::Mt940)
        .await
        .unwrap();
    assert!(matches!(step, Step::NeedTan(..)), "{step:?}");
    assert_eq!(script.requests().len(), 2, "no polling");
}

// ── Resuming with a stored system ID but no BPD ─────────────────────────

const STORED_ID: &str = "SYS-STORED-1";

/// A bank that keeps no record of system IDs (accepts the stored one) and
/// asks for a typed TAN (method 942) on login.
async fn resume_bank() -> HttpBank {
    let mut m = BankModel::default();
    m.customer.user_id = BANK_USER.into();
    m.customer.pin = BANK_PIN.into();
    m.customer.customer_id = BANK_USER.into();
    m.tan_methods = vec![testbank::TanMethod::typed(942, "mobileTAN", "654321")];
    m.login_sca = LoginSca::Tan;
    m.accept_unknown_system_ids = true;
    TestBank::new(m).serve().await
}

fn stored_id_only() -> SessionState {
    SessionState::new(&Blz::new("12345678"), &UserId::new(BANK_USER))
        .with_system_id(&fints_client::SystemId::new(STORED_ID))
}

#[tokio::test]
async fn stored_system_id_without_bpd_skips_the_sync_dialog() {
    let bank = resume_bank().await;
    let client = bank.client(&ProductId::new("P"));
    let (dialog, step) = jobs::open(&client, &creds(), &stored_id_only())
        .await
        .expect("open");
    let Step::NeedTan(challenge, _) = step else {
        panic!("expected a TAN challenge");
    };
    assert!(matches!(challenge.kind, TanKind::Typed { .. }));

    let reqs: Vec<String> = bank.requests().iter().map(|r| r.text()).collect();
    assert_eq!(reqs.len(), 3);
    assert!(reqs.iter().all(|r| !r.contains("HKSYN")), "no sync dialog");
    assert!(reqs[0].contains(STORED_ID) && !reqs[0].contains("HKTAN"));
    assert!(
        reqs[0].contains("+PIN:1+999+"),
        "probe is PIN-only: {}",
        reqs[0]
    );
    assert!(reqs[1].contains("HKEND"));
    assert!(reqs[2].contains(STORED_ID) && reqs[2].contains("HKTAN"));
    assert!(
        reqs[2].contains("+PIN:2+942+"),
        "init uses the 3920 method: {}",
        reqs[2]
    );
    assert!(bank.issued_system_ids().is_empty());
    let state = dialog.end().await.unwrap();
    assert_eq!(state.system_id.as_str(), STORED_ID);
    bank.assert_clean();
}

#[tokio::test]
async fn probe_with_wrong_pin_stops_without_a_second_attempt() {
    let bank = resume_bank().await;
    let client = bank.client(&ProductId::new("P"));
    let wrong = Credentials::new(UserId::new(BANK_USER), Pin::new("00000"));
    let result = jobs::open(&client, &wrong, &stored_id_only()).await;
    assert!(matches!(
        &result,
        Err(e) if e.kind()
            == fints_client::Category::Credentials(fints_client::return_code::Credentials::PinWrong)
    ));
    assert_eq!(bank.requests().len(), 1, "no sync after a wrong PIN");
    bank.assert_clean();
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn dialog_and_client_stay_send_and_sync() {
    assert_send_sync::<jobs::Dialog>();
    assert_send_sync::<BankClient>();
}

/// A HITAN its definition rejects fails with a
/// typed error that names the segment, in dialog init and in a job. A
/// well-formed one (and no HITAN at all) still works.
const BAD_HITAN: [(&str, u16); 2] = [
    ("HITAN:4:6:4+44++TASK-9+Bitte in der App freigeben'", 6),
    ("HITAN:4:7:4+44++TASK-9+Bitte in der App freigeben'", 7),
];

fn assert_unreadable_hitan(err: fints_client::FinTSError, version: u16) {
    match err {
        fints_client::FinTSError::SegmentUnreadable(e) => {
            let id = e.segment.expect("segment in the error");
            assert_eq!((id.code.as_str(), id.version), ("HITAN", version));
            assert!(!e.field.is_empty());
        }
        other => panic!("expected SegmentUnreadable, got {other}"),
    }
}

#[tokio::test]
async fn unreadable_hitan_fails_dialog_init() {
    for (hitan, version) in BAD_HITAN {
        let init =
            format!("HIRMG:2:2+0010::ok'HIRMS:3:2:4+3955::Bitte in der App bestaetigen'{hitan}");
        let script = scripted_bank(vec![init]).await;
        match jobs::open(&scripted_client(script.url()), &creds(), &scripted_state()).await {
            Err(e) => assert_unreadable_hitan(e, version),
            Ok(_) => panic!("init must fail on an unreadable HITAN {version}"),
        }
    }
}

#[tokio::test]
async fn unreadable_hitan_fails_a_job() {
    for (hitan, version) in BAD_HITAN {
        let order = format!(
            "HIRMG:2:2+3060::Teilweise Warnungen'\
             HIRMS:3:2:4+3955::Bitte bestaetigen Sie den Auftrag in Ihrer App'{hitan}"
        );
        let script = scripted_bank(vec![OPENED.into(), order]).await;
        let mut dialog = open_scripted(script.url()).await;
        let account = Account::new(IBAN, BIC).unwrap();
        let Err(err) = jobs::transactions(&mut dialog, &account, window(), Format::Mt940).await
        else {
            panic!("a job must fail on an unreadable HITAN {version}");
        };
        assert_unreadable_hitan(err, version);
    }
}

#[tokio::test]
async fn readable_hitan_still_starts_the_tan_step() {
    for version in [6, 7] {
        let order = format!(
            "HIRMG:2:2+3060::Teilweise Warnungen'\
             HIRMS:3:2:4+3955::Bitte bestaetigen Sie den Auftrag in Ihrer App'\
             HITAN:4:{version}:4+4++TASK-9+Bitte in der App freigeben'"
        );
        let script = scripted_bank(vec![OPENED.into(), order]).await;
        let mut dialog = open_scripted(script.url()).await;
        let account = Account::new(IBAN, BIC).unwrap();
        match jobs::transactions(&mut dialog, &account, window(), Format::Mt940)
            .await
            .unwrap()
        {
            Step::NeedTan(challenge, _) => {
                assert_eq!(challenge.challenge.as_str(), "Bitte in der App freigeben");
            }
            other => panic!("expected NeedTan, got {other:?}"),
        }
    }
}

fn assert_send<F: std::future::Future + Send>(_: F) {}

type Receipt = jobs::Resume<jobs::TransferReceipt>;

/// Awaits every job future a host drives. It is never run: it compiles only
/// when each of those futures is `Send`.
#[allow(dead_code, clippy::too_many_arguments)]
async fn every_job_future(
    client: &BankClient,
    creds: &Credentials,
    state: &SessionState,
    dialog: &mut jobs::Dialog,
    account: &Account,
    range: Range,
    transfer: &jobs::Transfer,
    resumes: (Receipt, Receipt, Receipt),
) {
    let _ = jobs::open(client, creds, state).await;
    let _ = jobs::balance(dialog, account).await;
    let _ = jobs::transactions(dialog, account, range, Format::Mt940).await;
    let _ = jobs::transfer(dialog, transfer).await;
    let _ = dialog.submit_tan(resumes.0, "123456").await;
    let _ = dialog.poll(resumes.1).await;
    let _ = dialog.confirm_vop(resumes.2).await;
}

#[allow(dead_code)]
async fn end_future(dialog: jobs::Dialog) {
    let _ = dialog.end().await;
}

#[allow(dead_code, clippy::too_many_arguments)]
fn job_futures_are_send(
    client: &BankClient,
    creds: &Credentials,
    state: &SessionState,
    dialog: &mut jobs::Dialog,
    account: &Account,
    range: Range,
    transfer: &jobs::Transfer,
    resumes: (Receipt, Receipt, Receipt),
    ended: jobs::Dialog,
) {
    assert_send(every_job_future(
        client, creds, state, dialog, account, range, transfer, resumes,
    ));
    assert_send(end_future(ended));
}
