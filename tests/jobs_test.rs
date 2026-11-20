//! `jobs`: steps, caller-driven TAN and polling, against the synthetic
//! test bank in-process. All data is synthetic.

use std::sync::Arc;

use fints_client::codec::defs::{HkTan6, HkTan7};
use fints_client::jobs::{
    self, BankClient, Cancelled, Credentials, Opened, Pending, Phase, Step, TanKind,
};
use fints_client::{
    Account, BankConfig, Category, FinTSError, Pin, ProductId, SessionState, UserId,
};
use fints_client_testbank::{BankHandle, BankModel, LoginSca, TanRule, TestBank};

fn client(handle: &BankHandle) -> BankClient {
    let m = handle.model();
    let config = BankConfig::new(m.name, m.blz, m.bic, "in-process://testbank");
    BankClient::new(config, ProductId::new("TEST-123"))
        .with_now(fints_client_testbank::TESTBANK_NOW)
        .with_transport(Arc::new(handle.clone()))
}

fn creds(m: &BankModel) -> Credentials {
    Credentials::new(UserId::new(&m.customer.user_id), Pin::new(&m.customer.pin))
}

fn empty_state(m: &BankModel) -> SessionState {
    SessionState::new(
        &fints_client::Blz::new(&m.blz),
        &UserId::new(&m.customer.user_id),
    )
}

fn account(m: &BankModel) -> Account {
    Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap()
}

fn bank(m: BankModel) -> BankHandle {
    TestBank::new(m).handle()
}

fn done<T: std::fmt::Debug>(step: Step<T>) -> jobs::Completed<T> {
    match step {
        Step::Done(d) => d,
        other => panic!("expected Done, got {other:?}"),
    }
}

/// Login exempt from SCA (3076): `Done` at once; balance is `Done`; `end`
/// returns the state with the issued system ID.
#[tokio::test]
async fn sca_exemption_opens_without_tan() {
    let handle = bank(BankModel::default());
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let opened = done(step).value;
    assert_eq!(opened.accounts.len(), 1);
    assert!(matches!(dialog.phase(), Phase::Open));

    let balance = done(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    assert_eq!(balance.value.unwrap().amount.to_string(), "1654.33");

    let state = dialog.end().await.unwrap();
    assert_eq!(state.system_id.as_str(), "TESTSYS0001");
    assert!(state.can_skip_sync());
    handle.assert_clean();
}

fn decoupled_model(approve_after: u32) -> BankModel {
    let mut m = BankModel {
        login_sca: LoginSca::Tan,
        hitans_version: 7,
        ..BankModel::default()
    };
    m.tan_methods = vec![fints_client_testbank::TanMethod::decoupled(
        902,
        "App-Freigabe Test",
        approve_after,
    )];
    m
}

/// Decoupled login: the caller polls, one HKTAN process S per call;
/// 3956 gives `NeedTan` again with `polls_done + 1`.
#[tokio::test]
async fn decoupled_login_caller_polls() {
    let handle = bank(decoupled_model(3));
    let m = handle.model();
    let (mut dialog, mut step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let mut seen = Vec::new();
    let opened: Opened = loop {
        match step {
            Step::Done(d) => break d.value,
            Step::NeedTan(challenge, resume) => {
                let TanKind::Decoupled {
                    polls_done,
                    max_polls,
                    ..
                } = challenge.kind
                else {
                    panic!("decoupled expected: {challenge:?}");
                };
                assert_eq!(max_polls, None, "0 = unlimited");
                seen.push(polls_done);
                assert!(dialog.pending().is_some());
                let before = handle.requests().len();
                step = dialog.poll(resume).await.unwrap();
                let requests = handle.requests();
                assert_eq!(requests.len(), before + 1, "one request per poll");
                let tan: HkTan7 = requests[before].get().expect("HKTAN 7");
                assert_eq!(tan.process.as_str(), "S");
                assert!(!requests[before].has("HKIDN"), "HKTAN alone");
            }
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(seen, [0, 1, 2]);
    assert_eq!(opened.accounts.len(), 1);
    done(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Past the bank's maximum of status requests, `poll` fails without
/// sending; a failed login ends the dialog.
#[tokio::test]
async fn decoupled_max_polls_fails_without_a_request() {
    let mut m = decoupled_model(5);
    m.decoupled_max_polls = 1;
    let handle = bank(m);
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::NeedTan(_, resume) = step else {
        panic!("TAN expected")
    };
    let Step::NeedTan(challenge, resume) = dialog.poll(resume).await.unwrap() else {
        panic!("still pending expected")
    };
    assert!(matches!(
        challenge.kind,
        TanKind::Decoupled {
            polls_done: 1,
            max_polls: Some(1),
            ..
        }
    ));
    let before = handle.requests().len();
    let err = dialog.poll(resume).await.unwrap_err();
    assert!(matches!(err, FinTSError::DecoupledMaxPolls(1)), "{err}");
    assert_eq!(handle.requests().len(), before, "nothing sent");
    assert!(matches!(dialog.phase(), Phase::Ended));
    let err = jobs::balance(&mut dialog, &account(&m)).await.unwrap_err();
    assert!(matches!(err, FinTSError::DialogEnded));
    assert_eq!(err.kind(), Category::DialogEnded);
}

/// `not_before` is the arrival of the bank's answer plus the HITANS wait:
/// the first wait for the challenge, the next wait after each 3956; none
/// when the bank gives no wait, and none for a typed challenge.
#[tokio::test]
async fn decoupled_not_before_follows_the_banks_waits() {
    let mut m = decoupled_model(5);
    m.decoupled_wait_secs = 30;
    let handle = bank(m);
    let m = handle.model();
    let t0 = jiff::Timestamp::now();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let t1 = jiff::Timestamp::now();
    let Step::NeedTan(challenge, resume) = step else {
        panic!("TAN expected")
    };
    let wait = jiff::SignedDuration::from_secs(30);
    let nb = challenge.not_before.expect("bank gives a wait");
    assert!(
        nb >= t0 + wait && nb <= t1 + wait,
        "{nb} not in [{t0}, {t1}] + 30 s"
    );

    let t0 = jiff::Timestamp::now();
    let Step::NeedTan(challenge, _) = dialog.poll(resume).await.unwrap() else {
        panic!("still pending expected")
    };
    let t1 = jiff::Timestamp::now();
    let nb = challenge.not_before.expect("bank gives a wait");
    assert!(
        nb >= t0 + wait && nb <= t1 + wait,
        "{nb} not in [{t0}, {t1}] + 30 s"
    );

    // No wait from the bank: none; typed challenge: none.
    let handle = bank(decoupled_model(5));
    let m = handle.model();
    let (_, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::NeedTan(challenge, _) = step else {
        panic!("TAN expected")
    };
    assert_eq!(challenge.not_before, None);
}

/// Polls until `DecoupledMaxPolls(2)`; returns the requests sent.
async fn polls_until_limit<T: jobs::JobValue + std::fmt::Debug>(
    handle: &BankHandle,
    dialog: &mut jobs::Dialog,
    mut step: Step<T>,
) -> usize {
    let mut sent = 0;
    loop {
        let Step::NeedTan(_, resume) = step else {
            panic!("pending expected")
        };
        let before = handle.requests().len();
        match dialog.poll(resume).await {
            Ok(next) => {
                assert_eq!(handle.requests().len(), before + 1);
                sent += 1;
                step = next;
            }
            Err(e) => {
                assert!(matches!(e, FinTSError::DecoupledMaxPolls(2)), "{e}");
                assert_eq!(handle.requests().len(), before, "nothing sent");
                return sent;
            }
        }
    }
}

/// A login's status requests count against the bank's maximum exactly
/// like an order's: with a maximum of 2, two requests are sent and the
/// third `poll` fails without sending, for both.
#[tokio::test]
async fn decoupled_max_polls_counts_alike_for_login_and_order() {
    let mut m = decoupled_model(9);
    m.decoupled_max_polls = 2;
    let handle = bank(m.clone());
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    assert_eq!(polls_until_limit(&handle, &mut dialog, step).await, 2);

    let mut m = decoupled_model(9);
    m.decoupled_max_polls = 2;
    m.login_sca = LoginSca::Exempt;
    let job = m.job_mut("HKSAL");
    job.hipins_needs_tan = true;
    job.server_tan = TanRule::Required;
    let handle = bank(m);
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let step = jobs::balance(&mut dialog, &account(&m)).await.unwrap();
    assert_eq!(polls_until_limit(&handle, &mut dialog, step).await, 2);
}

/// Typed TAN for the login (HKTAN process 2 with the TAN in HNSHA).
#[tokio::test]
async fn typed_tan_login() {
    let handle = bank(BankModel {
        login_sca: LoginSca::Tan,
        ..BankModel::default()
    });
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::NeedTan(challenge, resume) = step else {
        panic!("TAN expected")
    };
    assert!(matches!(challenge.kind, TanKind::Typed { .. }));
    assert_eq!(challenge.challenge.as_str(), "Bitte geben Sie die TAN ein");
    let opened = done(dialog.submit_tan(resume, "123456").await.unwrap()).value;
    assert_eq!(opened.accounts.len(), 1);
    done(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    dialog.end().await.unwrap();
    handle.assert_clean();
}

fn order_tan_model() -> BankModel {
    let mut m = BankModel::default();
    let job = m.job_mut("HKSAL");
    job.hipins_needs_tan = true;
    job.server_tan = TanRule::Required;
    m
}

/// Typed TAN on an order (new): HKSAL + HKTAN process 4 gets a challenge;
/// HKTAN process 2, alone with the TAN, gets the balance.
#[tokio::test]
async fn typed_tan_on_an_order() {
    let handle = bank(order_tan_model());
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let Step::NeedTan(challenge, resume) = jobs::balance(&mut dialog, &account(&m)).await.unwrap()
    else {
        panic!("TAN expected")
    };
    assert!(matches!(challenge.kind, TanKind::Typed { media: None }));
    let pending = dialog.pending().expect("paused");
    let json = serde_json::to_string(pending).unwrap();
    assert!(json.contains("Balance"), "{json}");

    let before = handle.requests().len();
    let balance = done(dialog.submit_tan(resume, "123456").await.unwrap());
    assert_eq!(balance.value.unwrap().amount.to_string(), "1654.33");
    let requests = handle.requests();
    assert_eq!(
        requests.len(),
        before + 1,
        "data came with the confirmation"
    );
    let tan: HkTan6 = requests[before].get().expect("HKTAN 6");
    assert_eq!(tan.process.as_str(), "2");
    assert!(!requests[before].has("HKSAL"), "HKTAN alone");
    assert_eq!(requests[before].tan.as_deref(), Some("123456"));
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// A wrong TAN the bank ends the order on fails the job; the dialog stays
/// open.
#[tokio::test]
async fn wrong_tan_on_an_order_keeps_the_dialog() {
    let mut m = order_tan_model();
    m.wrong_tan.kept = 0;
    let handle = bank(m);
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let Step::NeedTan(_, resume) = jobs::balance(&mut dialog, &account(&m)).await.unwrap() else {
        panic!("TAN expected")
    };
    let err = dialog.submit_tan(resume, "000000").await.unwrap_err();
    assert_eq!(err.kind(), Category::Tan, "{err}");
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
}

/// 9075 without HKTAN: the order is sent once more with HKTAN, and the
/// learned code reaches the session state on `end`; the next dialog sends
/// HKTAN at once.
#[tokio::test]
async fn sca_required_is_learned_on_balance() {
    let mut m = BankModel::default();
    m.job_mut("HKSAL").server_tan = TanRule::RequiredExempt;
    let handle = bank(m);
    let m = handle.model();
    let client = client(&handle);
    let (mut dialog, step) = jobs::open(&client, &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let before = handle.requests().len();
    done(jobs::balance(&mut dialog, &account(&m)).await.unwrap())
        .value
        .unwrap();
    assert_eq!(handle.requests().len(), before + 2, "9075, then with HKTAN");
    let state = dialog.end().await.unwrap();
    assert_eq!(
        state
            .tan_required_learned
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>(),
        ["HKSAL"]
    );

    let (mut dialog, step) = jobs::open(&client, &creds(&m), &state).await.unwrap();
    done(step);
    let before = handle.requests().len();
    done(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    let requests = handle.requests();
    assert_eq!(requests.len(), before + 1, "HKTAN at once");
    assert!(requests[before].has("HKTAN"));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// While paused, another job is refused; a ticket of another job or
/// dialog is refused; nothing is sent. `cancel` reopens the dialog.
#[tokio::test]
async fn paused_dialog_and_wrong_resume() {
    let handle = bank(BankModel {
        login_sca: LoginSca::Tan,
        ..order_tan_model()
    });
    let m = handle.model();
    let client = client(&handle);

    // Dialog A stays paused on its login.
    let (mut a, step) = jobs::open(&client, &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::NeedTan(_, login_a) = step else {
        panic!("TAN expected")
    };
    let before = handle.requests().len();
    let err = jobs::balance(&mut a, &account(&m)).await.unwrap_err();
    assert!(matches!(err, FinTSError::DialogPaused), "{err}");
    assert_eq!(err.kind(), Category::ClientFault);
    assert_eq!(handle.requests().len(), before, "nothing sent");

    // Dialog B: logged in, paused on a balance.
    let (mut b, step) = jobs::open(&client, &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::NeedTan(_, login_b) = step else {
        panic!("TAN expected")
    };
    done(b.submit_tan(login_b, "123456").await.unwrap());
    let Step::NeedTan(_, balance_b) = jobs::balance(&mut b, &account(&m)).await.unwrap() else {
        panic!("TAN expected")
    };
    let before_wrong = handle.requests().len();
    let err = b.submit_tan(login_a, "123456").await.unwrap_err();
    assert!(matches!(err, FinTSError::WrongResume), "{err}");
    assert_eq!(err.kind(), Category::ClientFault);
    assert_eq!(handle.requests().len(), before_wrong, "nothing sent");
    assert!(b.pending().is_some(), "B still waits for its balance");

    assert_eq!(b.cancel(balance_b).unwrap(), Cancelled::NothingSent);
    assert!(matches!(b.phase(), Phase::Open));
    let Step::NeedTan(_, again) = jobs::balance(&mut b, &account(&m)).await.unwrap() else {
        panic!("TAN expected")
    };
    done(b.submit_tan(again, "123456").await.unwrap());
    b.end().await.unwrap();
    a.end().await.unwrap();
    handle.assert_clean();
}

/// Notices of every message of a job, those before `NeedTan` included,
/// arrive on `Done`; consumed codes (0030, 3920) do not.
#[tokio::test]
async fn notices_arrive_on_done() {
    let mut m = BankModel {
        login_sca: LoginSca::Tan,
        ..BankModel::default()
    };
    m.customer.changed_user_id = Some("NEWUSER".into());
    let handle = bank(m);
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::NeedTan(_, resume) = step else {
        panic!("TAN expected")
    };
    let opened = done(dialog.submit_tan(resume, "123456").await.unwrap());
    let codes: Vec<String> = opened
        .notices
        .iter()
        .map(|n| n.code().unwrap().code.to_string())
        .collect();
    assert!(codes.contains(&"3072".to_string()), "{codes:?}");
    assert!(!codes.contains(&"0030".to_string()), "{codes:?}");
    assert!(!codes.contains(&"3920".to_string()), "{codes:?}");
    let state = dialog.end().await.unwrap();
    assert_eq!(state.user_id.as_str(), "NEWUSER");
}

// ── Paging jobs ─────────────────────────────────────────────────────────

use fints_client::jobs::{Format, Range, StatementFormat};
use fints_client::{FinTSError as Err_, TransactionStatus};
use fints_client_testbank::{Answer, Booking, Feedback, Reply, date};

fn range() -> Range {
    Range::new(
        jiff::civil::date(2026, 1, 1),
        jiff::civil::date(2026, 12, 31),
    )
}

async fn open_dialog(handle: &BankHandle) -> jobs::Dialog {
    let m = handle.model();
    let (dialog, step) = jobs::open(&client(handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    dialog
}

/// Three bookings, one per page.
fn paged_model() -> BankModel {
    let mut m = BankModel::default();
    m.accounts[0] = fints_client_testbank::Account::giro(&m.blz, "0001234567", "Testkundin Eins")
        .with_bookings(
            "500.00".parse().unwrap(),
            vec![
                Booking::new(date(2026, 3, 2), "-10.00", "SVWZ+Seite eins"),
                Booking::new(date(2026, 3, 3), "-20.00", "SVWZ+Seite zwei"),
                Booking::new(date(2026, 3, 4), "-30.00", "SVWZ+Seite drei"),
            ],
        );
    m.job_mut("HKKAZ").page_size = Some(1);
    m
}

/// A TAN from page 2 on: page 1 comes at once, pages 2 and 3 each pause.
/// The pending job keeps the pages so far (and survives JSON); after each
/// TAN the confirming response is the page and paging continues.
#[tokio::test]
async fn tan_on_page_two_pauses_and_paging_continues() {
    let mut m = paged_model();
    m.job_mut("HKKAZ").server_tan = TanRule::OnPage(2);
    let handle = bank(m);
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;

    let before = handle.requests().len();
    let Step::NeedTan(challenge, resume) =
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
            .await
            .unwrap()
    else {
        panic!("TAN expected on page 2")
    };
    assert!(matches!(challenge.kind, TanKind::Typed { .. }));
    // Page 1; page 2 without HKTAN (9075); page 2 with HKTAN.
    let requests = handle.requests();
    assert_eq!(requests.len(), before + 3);
    assert!(!requests[before].has("HKTAN"));
    assert!(requests[before + 2].has("HKTAN"));
    assert!(requests[before + 2].text().contains("TD-1"));
    let json = serde_json::to_string(dialog.pending().unwrap()).unwrap();
    assert!(json.contains("TD-1"), "touchdown point kept: {json}");

    // The TAN confirms page 2; page 3 pauses again (learned: HKTAN at once).
    let before = handle.requests().len();
    let Step::NeedTan(_, resume) = dialog.submit_tan(resume, "123456").await.unwrap() else {
        panic!("TAN expected on page 3")
    };
    let requests = handle.requests();
    assert_eq!(requests.len(), before + 2, "HKTAN 2, then page 3");
    assert!(requests[before + 1].text().contains("TD-2"));

    let statement = done(dialog.submit_tan(resume, "123456").await.unwrap()).value;
    assert_eq!(statement.format, StatementFormat::Mt940);
    let amounts: Vec<String> = statement
        .booked
        .iter()
        .map(|t| t.amount.to_string())
        .collect();
    assert_eq!(amounts, ["-10.00", "-20.00", "-30.00"]);
    assert!(matches!(statement.pending, Pending::NotSent));
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Notices of every page arrive on `Done`.
#[tokio::test]
async fn notices_accumulate_across_pages() {
    let handle = TestBank::new(paged_model())
        .tamper_clear(|_, segments| {
            if !segments.iter().any(|s| s.starts_with(b"HIKAZ:")) {
                return segments;
            }
            segments
                .into_iter()
                .map(|mut s| {
                    if s.starts_with(b"HIRMS:") {
                        s.pop();
                        s.extend_from_slice(b"+3050::Hinweis\'");
                    }
                    s
                })
                .collect()
        })
        .handle();
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let done = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
            .await
            .unwrap(),
    );
    assert_eq!(done.value.booked.len(), 3);
    let hints = done
        .notices
        .iter()
        .filter(|n| n.code().unwrap().code.to_string() == "3050")
        .count();
    assert_eq!(hints, 3, "one per page: {:?}", done.notices);
    assert!(
        !done
            .notices
            .iter()
            .any(|n| n.code().unwrap().code.to_string() == "3040"),
        "touchdown is consumed"
    );
    dialog.end().await.unwrap();
}

/// A bank error on page 2 fails the job (no pages returned); the dialog
/// stays open and a retry returns every entry once.
#[tokio::test]
async fn error_on_a_later_page_fails_the_job_and_keeps_the_dialog() {
    let mut calls = 0;
    let handle = TestBank::new(paged_model())
        .on("HKKAZ", move |_, _| {
            calls += 1;
            (calls == 2).then(|| Answer::error("9010", "Verarbeitung nicht möglich"))
        })
        .handle();
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let err = jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Category::Rejected, "{err}");
    assert!(matches!(dialog.phase(), Phase::Open));
    done(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(statement.booked.len(), 3);
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// 9800 on page 2 ends the dialog: typed error, then `DialogEnded`.
#[tokio::test]
async fn dialog_abort_mid_paging_ends_the_dialog() {
    let handle = TestBank::new(paged_model())
        .abort_dialog_on("HKKAZ", 2)
        .handle();
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let err = jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Category::DialogEnded, "{err}");
    assert!(matches!(dialog.phase(), Phase::Ended));
    let err = jobs::balance(&mut dialog, &account(&m)).await.unwrap_err();
    assert!(matches!(err, FinTSError::DialogEnded), "{err}");
    dialog.end().await.unwrap();
}

const CAMT_BOOKED: &str = "<Document><BkToCstmrAcctRpt><Rpt><Acct><Ccy>EUR</Ccy></Acct><Ntry>\
    <Amt Ccy=\"EUR\">12.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>\
    <BookgDt><Dt>2026-01-05</Dt></BookgDt><ValDt><Dt>2026-01-04</Dt></ValDt>\
    </Ntry></Rpt></BkToCstmrAcctRpt></Document>";
const CAMT_PENDING: &str = "<Document><BkToCstmrAcctRpt><Rpt><Ntry>\
    <Amt Ccy=\"EUR\">3.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>\
    <ValDt><Dt>2026-01-06</Dt></ValDt></Ntry></Rpt></BkToCstmrAcctRpt></Document>";

/// A bank that answers HKCAZ with `pending` as its pending document (none
/// when `None`);
/// `listed` says whether the UPD lists HKCAZ for the account and
/// `versions` which HICAZS versions the BPD offers.
fn camt_bank(listed: bool, versions: &[u16], pending: Option<&'static str>) -> BankHandle {
    let mut m = BankModel::default();
    m.jobs
        .insert("HKCAZ", fints_client_testbank::JobSpec::new(versions));
    if listed {
        m.account_mut().allowed_jobs.push("HKCAZ");
    }
    let iban = m.accounts[0].iban.clone().unwrap();
    let bic = m.bic.clone();
    TestBank::new(m)
        .on("HKCAZ", move |_, _| {
            let mut body = format!(
                "{iban}:{bic}+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+@{}@{CAMT_BOOKED}",
                CAMT_BOOKED.len(),
            );
            if let Some(pending) = pending {
                body.push_str(&format!("+@{}@{pending}", pending.len()));
            }
            Some(Answer::ok(vec![Reply::Raw {
                code: "HICAZ",
                version: 1,
                body,
            }]))
        })
        .handle()
}

fn sent(handle: &BankHandle, code: &str) -> usize {
    handle.requests().iter().filter(|r| r.has(code)).count()
}

/// camt is preferred when the bank offers a version we build and the UPD
/// lists it: HKCAZ only, pending entries marked.
#[tokio::test]
async fn camt_is_preferred() {
    let handle = camt_bank(true, &[1], Some(CAMT_PENDING));
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(statement.format, StatementFormat::Camt);
    assert_eq!(statement.booked.len(), 1);
    let pending = statement.pending.reported().expect("reported");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, TransactionStatus::Pending);
    assert_eq!(sent(&handle, "HKCAZ"), 1);
    assert_eq!(sent(&handle, "HKKAZ"), 0);
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// MT940 when camt cannot be sent, decided before sending: the UPD does
/// not list HKCAZ, or the BPD has no version we build, or no HICAZS.
#[tokio::test]
async fn mt940_when_camt_cannot_be_sent() {
    for (listed, versions) in [(false, vec![1]), (true, vec![2])] {
        let handle = camt_bank(listed, &versions, Some(CAMT_PENDING));
        let m = handle.model();
        let mut dialog = open_dialog(&handle).await;
        let statement = done(
            jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
                .await
                .unwrap(),
        )
        .value;
        assert_eq!(
            statement.format,
            StatementFormat::Mt940,
            "{listed} {versions:?}"
        );
        assert_eq!(statement.booked.len(), 2);
        assert_eq!(sent(&handle, "HKCAZ"), 0, "{listed} {versions:?}");
        dialog.end().await.unwrap();
        handle.assert_clean();
    }

    // Not offered at all.
    let handle = bank(BankModel::default());
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(statement.format, StatementFormat::Mt940);

    // An explicit camt request does not fall back.
    let err = jobs::transactions(&mut dialog, &account(&m), range(), Format::Camt)
        .await
        .unwrap_err();
    assert!(matches!(err, Err_::VersionMismatch(_)), "{err}");
    dialog.end().await.unwrap();
}

/// A bank rejecting HKCAZ is an error, not a reason to try HKKAZ.
#[tokio::test]
async fn camt_rejection_is_an_error_not_a_fallback() {
    let mut m = BankModel::default();
    m.jobs
        .insert("HKCAZ", fints_client_testbank::JobSpec::new(&[1]));
    m.account_mut().allowed_jobs.push("HKCAZ");
    let handle = TestBank::new(m)
        .on("HKCAZ", |_, _| {
            Some(Answer::error("9010", "Verarbeitung nicht möglich"))
        })
        .handle();
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let err = jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Category::Rejected, "{err}");
    assert_eq!(sent(&handle, "HKKAZ"), 0);
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
}

/// Pending data that does not parse is reported, the booked entries stay.
#[tokio::test]
async fn unparseable_pending_data_is_reported_not_lost() {
    let handle = camt_bank(true, &[1], Some("<Document><unclosed>"));
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(statement.booked.len(), 1);
    assert!(
        matches!(&statement.pending, Pending::Unreadable { message, .. } if !message.is_empty()),
        "{:?}",
        statement.pending
    );
    dialog.end().await.unwrap();
}

/// A camt answer without the pending element is `NotSent`; an empty
/// pending document is `Reported` (nothing pending).
#[tokio::test]
async fn pending_part_absent_or_empty() {
    let handle = camt_bank(true, &[1], None);
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(statement.booked.len(), 1);
    assert!(matches!(statement.pending, Pending::NotSent));
    assert!(statement.pending.reported().is_none());
    dialog.end().await.unwrap();

    let handle = camt_bank(true, &[1], Some(""));
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Auto)
            .await
            .unwrap(),
    )
    .value;
    assert!(matches!(&statement.pending, Pending::Reported(p) if p.is_empty()));
    dialog.end().await.unwrap();
}

/// MT940: the test bank sends no MT942 (`NotSent`); an MT942 in the answer
/// is `Reported` (even without entries), a broken one `Unreadable`.
#[tokio::test]
async fn mt940_pending_part_states() {
    let handle = bank(BankModel::default());
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
            .await
            .unwrap(),
    )
    .value;
    assert!(matches!(statement.pending, Pending::NotSent));
    dialog.end().await.unwrap();

    for (mt942, expect_reported) in [
        (
            ":20:STARTDISPE\r\n:25:12345678/0000001234\r\n:28C:0\r\n:13D:2601021230+0100\r\n-",
            true,
        ),
        (":20:STARTDISPE\r\n:61:garbage\r\n-", false),
    ] {
        let handle = TestBank::new(BankModel::default())
            .tamper_clear(move |_, segments| {
                segments
                    .into_iter()
                    .map(|s| match unbooked(&s, mt942) {
                        Some(with_pending) => with_pending,
                        None => s,
                    })
                    .collect()
            })
            .handle();
        let m = handle.model();
        let mut dialog = open_dialog(&handle).await;
        let statement = done(
            jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
                .await
                .unwrap(),
        )
        .value;
        match (&statement.pending, expect_reported) {
            (Pending::Reported(p), true) => assert!(p.is_empty()),
            (Pending::Unreadable { .. }, false) => {}
            (other, _) => panic!("unexpected {other:?}"),
        }
        dialog.end().await.unwrap();
    }
}

/// `segment` (an HIKAZ) with `mt942` as its second data element.
fn unbooked(segment: &[u8], mt942: &str) -> Option<Vec<u8>> {
    if !segment.starts_with(b"HIKAZ:") {
        return None;
    }
    let mut out = segment.to_vec();
    assert_eq!(out.pop(), Some(b'\''));
    out.extend_from_slice(format!("+@{}@{mt942}'", mt942.len()).as_bytes());
    Some(out)
}

/// A bank with two booked and three pending entries, two of each per
/// HKKAZ page, and an MT942 part (`model`).
fn mt942_model(model: fints_client_testbank::Mt942Model) -> BankModel {
    let mut m = BankModel::default();
    let mut a = fints_client_testbank::Account::giro(&m.blz, "0001234567", "Testkundin Eins")
        .with_bookings(
            "500.00".parse().unwrap(),
            vec![
                Booking::new(date(2026, 6, 1), "-10.00", "SVWZ+Gebucht eins"),
                Booking::new(date(2026, 6, 2), "25.00", "SVWZ+Gebucht zwei"),
            ],
        );
    a.pending = vec![
        Booking::new(date(2026, 6, 15), "-4.20", "SVWZ+Karte eins").posting_text("KARTE"),
        Booking::new(date(2026, 6, 15), "100.00", "SVWZ+Vorgemerkt"),
        Booking::new(date(2026, 6, 16), "-7.80", "SVWZ+Karte zwei"),
    ];
    m.accounts[0] = a;
    m.job_mut("HKKAZ").page_size = Some(2);
    m.mt942 = Some(model);
    m
}

/// MT942 from the test bank over two touchdown pages: page 1 has two
/// booked and two pending entries, page 2 only the third pending one. The
/// pending list is `Reported` in the bank's order with the `:13D:`
/// creation time (or the `:13:` one, without offset).
#[tokio::test]
async fn mt942_pending_entries_are_reported_over_touchdown_pages() {
    use fints_client_testbank::Mt942Model;
    let created = jiff::civil::date(2026, 6, 15).at(9, 5, 0, 0);
    for (offset, timestamp) in [(Some("+0200"), Some("2026-06-15T07:05:00Z")), (None, None)] {
        let handle = bank(mt942_model(Mt942Model {
            created,
            offset,
            ..Mt942Model::default()
        }));
        let m = handle.model();
        let mut dialog = open_dialog(&handle).await;
        let before = handle.requests().len();
        let statement = done(
            jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
                .await
                .unwrap(),
        )
        .value;
        let requests = handle.requests();
        assert_eq!(requests.len(), before + 2, "two pages");
        assert!(requests[before + 1].text().contains("TD-2"));

        let amounts = |txs: &[fints_client::Transaction]| -> Vec<String> {
            txs.iter().map(|t| t.amount.to_string()).collect()
        };
        assert_eq!(amounts(&statement.booked), ["-10.00", "25.00"]);
        let Pending::Reported(pending) = &statement.pending else {
            panic!("expected Reported, got {:?}", statement.pending)
        };
        assert_eq!(amounts(pending), ["-4.20", "100.00", "-7.80"]);
        assert!(
            pending
                .iter()
                .all(|t| t.status == TransactionStatus::Pending)
        );
        assert_eq!(pending[0].purpose.as_deref(), Some("Karte eins"));
        assert_eq!(pending[0].posting_text.as_deref(), Some("KARTE"));
        assert_eq!(pending[2].date, date(2026, 6, 16));
        assert!(pending.iter().all(|t| t.booking_id.is_none()));

        let at = statement.pending_created_at.expect("creation time");
        assert_eq!(at.datetime, created);
        assert_eq!(
            at.timestamp(),
            timestamp.map(|t| t.parse::<jiff::Timestamp>().unwrap())
        );
        assert_eq!(at.offset.is_some(), offset.is_some());
        dialog.end().await.unwrap();
        handle.assert_clean();
    }
}

/// An MT942 whose `:90D:` announces more bookings than it lists is not a
/// complete list: `Unreadable`, with no creation time; the booked entries
/// of both pages stay.
#[tokio::test]
async fn mt942_with_wrong_totals_is_unreadable_booked_part_stays() {
    let handle = bank(mt942_model(fints_client_testbank::Mt942Model {
        miscount: true,
        ..Default::default()
    }));
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(statement.booked.len(), 2);
    match &statement.pending {
        Pending::Unreadable { kind, message } => {
            assert_eq!(*kind, Category::ClientFault, "{message}");
            assert!(message.contains("(:90D:)"), "{message}");
            assert!(message.contains("not complete"), "{message}");
        }
        other => panic!("expected Unreadable, got {other:?}"),
    }
    assert!(statement.pending.reported().is_none());
    assert_eq!(statement.pending_created_at, None);
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// An MT942 whose `:90D:` sum is off but whose counts match lists every
/// booking: `Reported` (the sum mismatch is only logged).
#[tokio::test]
async fn mt942_with_wrong_sum_but_right_counts_is_reported() {
    let handle = bank(mt942_model(fints_client_testbank::Mt942Model {
        missum: true,
        ..Default::default()
    }));
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let statement = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
            .await
            .unwrap(),
    )
    .value;
    let pending = statement.pending.reported().expect("Reported");
    assert_eq!(pending.len(), 3);
    assert!(statement.pending_created_at.is_some());
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// A range that starts before the bank's storage period (the test bank
/// says 365 days for HIKAZS) is sent from the oldest stored day, with a
/// notice; a range inside the period is sent as asked and has no notice.
#[tokio::test]
async fn range_before_the_storage_period_is_clamped() {
    let handle = bank(BankModel::default());
    let m = handle.model();
    let mut dialog = open_dialog(&handle).await;
    let before = handle.requests().len();
    let old = Range::new(
        jiff::civil::date(2000, 1, 1),
        jiff::civil::date(2000, 12, 31),
    );
    let done = done(
        jobs::transactions(&mut dialog, &account(&m), old, Format::Mt940)
            .await
            .unwrap(),
    );
    let oldest = fints_client_testbank::TESTBANK_TODAY
        .checked_sub(jiff::Span::new().days(365))
        .unwrap();
    assert_eq!(oldest, jiff::civil::date(2025, 6, 15));
    let warning = done
        .notices
        .iter()
        .find_map(|n| n.client_warning())
        .expect("clamp notice");
    assert_eq!(
        warning,
        fints_client::ClientWarning::RangeClamped {
            requested: old.from,
            used: oldest
        }
    );
    let request = handle.requests()[before].text();
    assert!(
        request.contains(&oldest.strftime("%Y%m%d").to_string()),
        "{request}"
    );
    assert!(!request.contains("20000101"), "{request}");

    let done = self::done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
            .await
            .unwrap(),
    );
    assert!(done.notices.iter().all(|n| n.client_warning().is_none()));
    dialog.end().await.unwrap();
}

/// The pinned date, not the wall clock, drives the clamp: the same range
/// is clamped from another day when the client is pinned elsewhere.
#[tokio::test]
async fn pinned_date_drives_the_clamp() {
    let handle = bank(BankModel::default());
    let m = handle.model();
    let pinned = client(&handle).with_now(jiff::civil::datetime(2030, 3, 1, 0, 0, 0, 0));
    let (mut dialog, step) = jobs::open(&pinned, &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let done = done(
        jobs::transactions(&mut dialog, &account(&m), range(), Format::Mt940)
            .await
            .unwrap(),
    );
    // 365 days before 2030-03-01.
    assert_eq!(
        done.notices.iter().find_map(|n| n.client_warning()),
        Some(fints_client::ClientWarning::RangeClamped {
            requested: range().from,
            used: jiff::civil::date(2029, 3, 1),
        })
    );
    dialog.end().await.unwrap();
}

const MT535: &str = ":16R:GENL\r\n\
:97A::SAFE//12345678/0007000999\r\n\
:98A::STAT//20260410\r\n\
:16S:GENL\r\n\
:16R:FIN\r\n\
:35B:ISIN XX0000TEST01\r\n\
TESTFONDS WELT\r\n\
:90B::MRKT//ACTU/EUR21,5\r\n\
:93B::AGGR//UNIT/40,\r\n\
:19A::HOLD//EUR860,\r\n\
:16S:FIN\r\n\
-";

fn depot_model() -> (BankModel, Account) {
    let mut m = BankModel::default();
    let mut depot =
        fints_client_testbank::Account::depot(&m.blz, "0007000999", "Testperson Depot", MT535);
    depot.allowed_jobs.push("HKWDU");
    // A depot is addressed nationally.
    let account =
        Account::national("0007000999", None, &fints_client::Blz::new(&m.blz), &m.bic).unwrap();
    m.accounts.push(depot);
    (m, account)
}

#[tokio::test]
async fn holdings_of_a_depot() {
    let (m, depot) = depot_model();
    let handle = bank(m);
    let mut dialog = open_dialog(&handle).await;
    let holdings = done(jobs::holdings(&mut dialog, &depot, None).await.unwrap()).value;
    assert_eq!(holdings.len(), 1);
    assert_eq!(holdings[0].name, "TESTFONDS WELT");
    assert_eq!(holdings[0].quantity.to_string(), "40");
    dialog.end().await.unwrap();
    handle.assert_clean();
}

fn mt536(reference: &str) -> String {
    format!(
        ":16R:GENL\r\n:97A::SAFE//12345678/0007000999\r\n:16S:GENL\r\n\
:16R:SUBSAFE\r\n:16R:FIN\r\n:35B:ISIN XX0000TEST01\r\nTESTFONDS WELT\r\n\
:16R:TRAN\r\n:16R:TRANSDET\r\n:36B::PSTA//UNIT/10,\r\n:19A::PSTA//NEUR425,\r\n\
:22H::REDE//RECE\r\n:98A::ESET//20260315\r\n:70E::TRDE//{reference}\r\n\
:16S:TRANSDET\r\n:16S:TRAN\r\n:16S:FIN\r\n:16S:SUBSAFE\r\n-"
    )
}

/// Depot transactions over two touchdown pages.
#[tokio::test]
async fn depot_transactions_page_through_touchdown() {
    let (mut m, depot) = depot_model();
    m.jobs
        .insert("HKWDU", fints_client_testbank::JobSpec::new(&[5]));
    let mut calls = 0;
    let handle = TestBank::new(m)
        .on("HKWDU", move |_, _| {
            calls += 1;
            let page = mt536(&format!("KAUF {calls}"));
            let data = Reply::Raw {
                code: "HIWDU",
                version: 5,
                body: format!("@{}@{page}", page.len()),
            };
            Some(if calls == 1 {
                Answer::default()
                    .with(Feedback::new("3040", "Weitere Informationen").with_params(&["TD-1"]))
                    .with_data(data)
            } else {
                Answer::ok(vec![data])
            })
        })
        .handle();
    let mut dialog = open_dialog(&handle).await;
    let found = done(
        jobs::depot_transactions(&mut dialog, &depot, Some(range()))
            .await
            .unwrap(),
    )
    .value;
    assert_eq!(found.len(), 2, "{found:?}");
    let requests = handle.requests();
    let wdu: Vec<_> = requests.iter().filter(|r| r.has("HKWDU")).collect();
    assert_eq!(wdu.len(), 2);
    assert!(wdu[1].text().contains("TD-1"));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Orders the bank does not offer fail before anything is sent.
#[tokio::test]
async fn unoffered_jobs_send_nothing() {
    let (m, depot) = depot_model();
    let handle = bank(m);
    let mut dialog = open_dialog(&handle).await;
    let before = handle.requests().len();
    let err = jobs::depot_transactions(&mut dialog, &depot, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Err_::VersionMismatch(_)), "{err}");
    assert_eq!(handle.requests().len(), before, "nothing sent");
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
}

/// Every job refuses a missing BPD version before anything is sent, and
/// the dialog stays usable.
#[tokio::test]
async fn every_job_refuses_an_unoffered_order_before_sending() {
    let mut m = BankModel::default();
    m.jobs.remove("HKSAL");
    m.jobs.remove("HKKAZ");
    m.jobs.remove("HKWPD");
    let handle = bank(m);
    let m = handle.model();
    let acc = account(&m);
    let mut dialog = open_dialog(&handle).await;
    let before = handle.requests().len();
    let err = jobs::balance(&mut dialog, &acc).await.unwrap_err();
    assert!(matches!(err, Err_::VersionMismatch(_)), "{err}");
    let err = jobs::transactions(&mut dialog, &acc, range(), Format::Mt940)
        .await
        .unwrap_err();
    assert!(matches!(err, Err_::VersionMismatch(_)), "{err}");
    let err = jobs::transactions(&mut dialog, &acc, range(), Format::Camt)
        .await
        .unwrap_err();
    assert!(matches!(err, Err_::VersionMismatch(_)), "{err}");
    let err = jobs::holdings(&mut dialog, &acc, None).await.unwrap_err();
    assert!(matches!(err, Err_::VersionMismatch(_)), "{err}");
    assert_eq!(handle.requests().len(), before, "nothing sent");
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
}

/// `Dialog::capabilities` says what the jobs then do: the camt/MT940
/// choice, the offered versions, and a UPD `No` shown but not blocking.
#[tokio::test]
async fn capabilities_match_what_the_jobs_do() {
    use fints_client::Allowance;
    use fints_client::jobs::{JobKind, Support};

    // HKCAZ offered but not in the UPD: MT940.
    let handle = camt_bank(false, &[1], Some(CAMT_PENDING));
    let m = handle.model();
    let acc = account(&m);
    let mut dialog = open_dialog(&handle).await;
    let caps = dialog.capabilities(&[]);
    let tx = caps.accounts[0].job(JobKind::Transactions).unwrap();
    assert!(
        matches!(&tx.support, Support::Available { code: "HKKAZ", .. }),
        "{tx:?}"
    );
    assert_eq!(tx.upd, Allowance::Yes);
    done(
        jobs::transactions(&mut dialog, &acc, range(), Format::Auto)
            .await
            .unwrap(),
    );
    assert_eq!((sent(&handle, "HKKAZ"), sent(&handle, "HKCAZ")), (1, 0));
    dialog.end().await.unwrap();

    // Listed: camt, and the same answer from the stored state.
    let handle = camt_bank(true, &[1], Some(CAMT_PENDING));
    let m = handle.model();
    let acc = account(&m);
    let mut dialog = open_dialog(&handle).await;
    let caps = dialog.capabilities(&[]);
    let tx = caps.accounts[0].job(JobKind::Transactions).unwrap();
    assert!(
        matches!(
            &tx.support,
            Support::Available {
                code: "HKCAZ",
                version: 1,
                ..
            }
        ),
        "{tx:?}"
    );
    done(
        jobs::transactions(&mut dialog, &acc, range(), Format::Auto)
            .await
            .unwrap(),
    );
    assert_eq!((sent(&handle, "HKKAZ"), sent(&handle, "HKCAZ")), (0, 1));
    let state = dialog.end().await.unwrap();
    assert_eq!(state.capabilities(&[]), caps);

    // A UPD `No` is shown and the order is sent anyway: the giro account
    // does not list HKWPD, the bank answers (with an error).
    let (m, _) = depot_model();
    let giro = account(&m);
    let handle = bank(m);
    let mut dialog = open_dialog(&handle).await;
    let caps = dialog.capabilities(std::slice::from_ref(&giro));
    let h = caps.accounts[0].job(JobKind::Holdings).unwrap();
    assert_eq!(h.upd, Allowance::No);
    assert!(
        matches!(
            h.support,
            Support::Available {
                code: "HKWPD",
                version: 6,
                ..
            }
        ),
        "{h:?}"
    );
    let _ = jobs::holdings(&mut dialog, &giro, None).await;
    assert_eq!(sent(&handle, "HKWPD"), 1, "sent despite the UPD");
    let _ = dialog.end().await;
}

/// `InitMode::Direct` (ING): no synchronisation dialog, no HKTAN; the
/// business dialog opens with a client-generated system ID and security
/// function 900.
#[tokio::test]
async fn direct_init_sends_no_sync() {
    // Like ING, the bank takes a system ID it did not issue.
    let handle = bank(BankModel {
        accept_unknown_system_ids: true,
        ..BankModel::default()
    });
    let m = handle.model();
    let mut config = BankConfig::new(m.name.clone(), &m.blz, &m.bic, "in-process://testbank");
    config.profile.init = fints_client::InitMode::Direct {
        security_function: "900".into(),
    };
    let client = BankClient::new(config, ProductId::new("TEST-123"))
        .with_now(fints_client_testbank::TESTBANK_NOW)
        .with_transport(Arc::new(handle.clone()));
    let (dialog, step) = jobs::open(&client, &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    assert_eq!(done(step).value.accounts.len(), 1);
    dialog.end().await.unwrap();

    let sent = handle.requests();
    assert!(sent.iter().all(|r| !r.has("HKSYN")), "no sync");
    assert!(sent.iter().all(|r| !r.has("HKTAN")), "no TAN");
    assert!(
        sent[0].has("HKIDN") && sent[0].has("HKVVB"),
        "starts with the business init"
    );
    assert!(sent[0].text().contains("+900+"), "security function 900");
    handle.assert_clean();
}

/// A model whose customer has the completed Consorsbank-style user ID, and
/// a client with that bank's profile (the rule completes 9 digits to 12).
fn consors_like(login_sca: LoginSca) -> (BankHandle, BankClient) {
    consors_bank(BankModel {
        login_sca,
        ..order_tan_model()
    })
}

/// [`consors_like`] with any model.
fn consors_bank(mut model: BankModel) -> (BankHandle, BankClient) {
    model.customer.user_id = "123456789001".into();
    let handle = bank(model);
    let m = handle.model();
    let profile = fints_client::banks::profile_by_blz("76030080");
    assert!(profile.user_id.is_some());
    let config =
        BankConfig::new(m.name, m.blz, m.bic, "in-process://testbank").with_profile(profile);
    let client = BankClient::new(config, ProductId::new("TEST-123"))
        .with_now(fints_client_testbank::TESTBANK_NOW)
        .with_transport(Arc::new(handle.clone()));
    (handle, client)
}

fn short_creds(m: &BankModel) -> Credentials {
    Credentials::new(UserId::new("123456789"), Pin::new(&m.customer.pin))
}

#[tokio::test]
async fn short_user_id_is_completed_on_the_wire_and_in_the_state() {
    let (handle, client) = consors_like(LoginSca::Exempt);
    let m = handle.model();
    let blz = fints_client::Blz::new(&m.blz);
    let state = SessionState::new(&blz, &UserId::new("123456789"));
    let (dialog, step) = jobs::open(&client, &short_creds(&m), &state).await.unwrap();
    done(step);
    let state = dialog.end().await.unwrap();
    assert_eq!(state.user_id.as_str(), "123456789001");
    for r in handle.requests().iter().filter(|r| r.has("HKIDN")) {
        assert!(r.text().contains("+123456789001+"), "{}", r.text());
        assert!(!r.text().contains("+123456789+"));
    }

    // A state recorded with the full ID matches a login with the short form:
    // it is reused, so no synchronisation dialog runs again.
    let before = handle.requests().len();
    let (dialog, step) = jobs::open(&client, &short_creds(&m), &state).await.unwrap();
    done(step);
    dialog.end().await.unwrap();
    let later = handle.requests();
    assert!(
        later[before..].iter().all(|r| !r.has("HKSYN")),
        "state kept"
    );
    handle.assert_clean();
}

#[tokio::test]
async fn thaw_with_the_short_user_id_matches_the_completed_one() {
    let (handle, client) = consors_like(LoginSca::Tan);
    let m = handle.model();
    let blz = fints_client::Blz::new(&m.blz);
    let state = SessionState::new(&blz, &UserId::new("123456789"));
    let (dialog, step) = jobs::open(&client, &short_creds(&m), &state).await.unwrap();
    let Step::NeedTan(_, login) = step else {
        panic!("TAN expected")
    };
    let paused = dialog.freeze().unwrap();
    let mut thawed = jobs::Dialog::thaw(paused, &client, &short_creds(&m)).unwrap();
    done(thawed.submit_tan(login, "123456").await.unwrap());
    thawed.end().await.unwrap();
    handle.assert_clean();
}

/// Two methods allowed: decoupled 902 and typed 921.
fn two_methods(login_sca: LoginSca) -> BankModel {
    let mut m = BankModel {
        login_sca,
        hitans_version: 7,
        ..BankModel::default()
    };
    m.tan_methods = vec![
        fints_client_testbank::TanMethod::decoupled(902, "App-Freigabe Test", 1),
        fints_client_testbank::TanMethod::typed(921, "Test-TAN", "123456"),
    ];
    m
}

/// The security function of the dialog initialisation (HKIDN without
/// HKSYN) among `requests`.
fn init_function(requests: &[fints_client_testbank::Request]) -> String {
    let init = requests
        .iter()
        .find(|r| r.has("HKIDN") && !r.has("HKSYN"))
        .expect("a dialog initialisation");
    init.security_function.clone().unwrap_or_default()
}

/// `synchronise` runs the sync dialog only (no HKTAN, no login), the
/// state then lists both methods; the caller picks the second and `open`
/// logs in with it, without another sync dialog.
#[tokio::test]
async fn synchronise_then_open_uses_the_chosen_method() {
    let handle = bank(two_methods(LoginSca::Tan));
    let m = handle.model();
    let client = client(&handle);
    let state = jobs::synchronise(&client, &creds(&m), &empty_state(&m))
        .await
        .unwrap()
        .value;
    let sent = handle.requests();
    assert!(sent.iter().any(|r| r.has("HKSYN")), "sync dialog");
    assert!(sent.iter().all(|r| !r.has("HKTAN")), "no TAN");
    assert_eq!(sent.len(), 2, "sync and its end");
    let functions: Vec<&str> = state
        .allowed_security_functions
        .iter()
        .map(|f| f.as_str())
        .collect();
    assert_eq!(functions, ["902", "921"]);
    assert_eq!(state.tan_methods().len(), 2);
    assert_eq!(
        state.tan_method.as_ref().unwrap().as_str(),
        "902",
        "default"
    );
    assert!(state.can_skip_sync());

    let chosen = state.tan_methods()[1].security_function.clone();
    let state = state.with_tan_method(&chosen);
    let (mut dialog, step) = jobs::open(&client, &creds(&m), &state).await.unwrap();
    let Step::NeedTan(challenge, resume) = step else {
        panic!("TAN expected")
    };
    assert!(matches!(challenge.kind, TanKind::Typed { .. }));
    let later = handle.requests()[2..].to_vec();
    assert!(later.iter().all(|r| !r.has("HKSYN")), "no second sync");
    assert_eq!(init_function(&later), "921");
    done(dialog.submit_tan(resume, "123456").await.unwrap());
    let state = dialog.end().await.unwrap();
    assert_eq!(state.tan_method.as_ref().unwrap().as_str(), "921");

    // Again with a system ID: the parameter probe instead of HKSYN, and
    // the choice stays.
    let before = handle.requests().len();
    let state = jobs::synchronise(&client, &creds(&m), &state)
        .await
        .unwrap()
        .value;
    let later = handle.requests()[before..].to_vec();
    assert!(later.iter().all(|r| !r.has("HKSYN") && !r.has("HKTAN")));
    assert_eq!(state.tan_method.as_ref().unwrap().as_str(), "921");
    assert_eq!(state.system_id.as_str(), "TESTSYS0001");
    handle.assert_clean();
}

/// A fresh state made with the 9-digit Consorsbank user ID and a TAN
/// preference: `open` and `synchronise` keep the preference although the
/// profile completes the ID.
#[tokio::test]
async fn short_user_id_keeps_the_tan_preference() {
    let (handle, client) = consors_bank(two_methods(LoginSca::Exempt));
    let m = handle.model();
    let blz = fints_client::Blz::new(&m.blz);
    let typed = fints_client::SecurityFunction::new("921");
    let state = SessionState::new(&blz, &UserId::new("123456789")).with_tan_method(&typed);

    let (dialog, step) = jobs::open(&client, &short_creds(&m), &state).await.unwrap();
    done(step);
    assert_eq!(init_function(&handle.requests()), "921");
    let ended = dialog.end().await.unwrap();
    assert_eq!(ended.tan_method.as_ref(), Some(&typed));

    let synced = jobs::synchronise(&client, &short_creds(&m), &state)
        .await
        .unwrap()
        .value;
    assert_eq!(synced.user_id.as_str(), "123456789001");
    assert_eq!(synced.tan_method.as_ref(), Some(&typed));
    handle.assert_clean();
}

/// `InitMode::Direct` has no synchronisation dialog: `synchronise` sends
/// nothing and names the profile's security function, which `open` uses.
#[tokio::test]
async fn synchronise_with_direct_init_sends_nothing() {
    let handle = bank(BankModel {
        accept_unknown_system_ids: true,
        ..BankModel::default()
    });
    let m = handle.model();
    let mut config = BankConfig::new(m.name.clone(), &m.blz, &m.bic, "in-process://testbank");
    config.profile.init = fints_client::InitMode::Direct {
        security_function: "900".into(),
    };
    let client = BankClient::new(config, ProductId::new("TEST-123"))
        .with_now(fints_client_testbank::TESTBANK_NOW)
        .with_transport(Arc::new(handle.clone()));
    let synced = jobs::synchronise(&client, &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    assert!(handle.requests().is_empty(), "nothing sent");
    assert!(synced.notices.is_empty());
    let state = synced.value;
    assert_eq!(state.tan_method.as_ref().unwrap().as_str(), "900");
    assert!(!state.can_skip_sync());

    let (dialog, step) = jobs::open(&client, &creds(&m), &state).await.unwrap();
    done(step);
    dialog.end().await.unwrap();
    assert_eq!(init_function(&handle.requests()), "900");
    handle.assert_clean();
}
