//! How a transfer ends when the client stops before the bank's answer:
//! cancel, poll limit, an unreadable answer after the order, a dropped
//! future (docs/payments-design.md). A
//! transfer ends with a receipt, a class 9 rejection, or
//! `PaymentOutcomeUnknown`; `cancel` says whether the order is at the
//! bank. Everything runs against the synthetic test bank.

use std::sync::Arc;
use std::time::Duration;

use fints_client::jobs::{
    self, BankClient, Cancelled, Credentials, Dialog, Phase, Step, TanKind, TransferReceipt,
};
use fints_client::{
    Account, BankConfig, Category, FinTSError, Instant, Pin, ProductId, SessionState, Transfer,
    UserId,
};
use fints_client_testbank::{
    BankHandle, BankModel, Fault, OnMatch, Payee, PaymentsModel, ResultAs, TanMethod, TestBank,
    VopModel, iban_de,
};
use rust_decimal::Decimal;

const TAN: &str = "123456";
const E2E: &str = "E2E-TEST-1";

fn anna() -> String {
    iban_de("87654321", "0000000042")
}

fn payments(vop: Option<VopModel>) -> PaymentsModel {
    PaymentsModel {
        payees: vec![Payee::new(&anna(), "Anna Beispiel")],
        vop,
        ..PaymentsModel::default()
    }
}

fn vop(on_match: OnMatch) -> Option<VopModel> {
    Some(VopModel {
        hkvpa_on_match: on_match,
        ..VopModel::default()
    })
}

/// A bank with payments; `approve_after` makes its TAN method decoupled,
/// approved on that status request.
fn model(p: PaymentsModel, approve_after: Option<u32>) -> BankModel {
    let mut m = BankModel {
        hispas_national: Some(false),
        ..BankModel::default()
    };
    if let Some(polls) = approve_after {
        m.tan_methods = vec![TanMethod::decoupled(940, "Test-App", polls)];
        m.hitans_version = 7;
    }
    m.enable_payments(p);
    m
}

fn client(handle: &BankHandle) -> BankClient {
    let m = handle.model();
    let config = BankConfig::new(m.name, m.blz, m.bic, "in-process://testbank");
    BankClient::new(config, ProductId::new("TEST-123")).with_transport(Arc::new(handle.clone()))
}

async fn open_with(client: &BankClient, m: &BankModel) -> Dialog {
    let creds = Credentials::new(UserId::new(&m.customer.user_id), Pin::new(&m.customer.pin));
    let state = SessionState::new(
        &fints_client::Blz::new(&m.blz),
        &UserId::new(&m.customer.user_id),
    );
    let (dialog, step) = jobs::open(client, &creds, &state).await.unwrap();
    assert!(matches!(step, Step::Done(_)), "login is exempt");
    dialog
}

async fn open(handle: &BankHandle) -> Dialog {
    open_with(&client(handle), &handle.model()).await
}

fn transfer(m: &BankModel) -> Transfer {
    Transfer {
        from: Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap(),
        debtor_name: None,
        payee_name: "Anna Beispiel".into(),
        payee_iban: anna(),
        payee_bic: None,
        amount: Decimal::new(1250, 2),
        purpose: "Testzahlung".into(),
        end_to_end_id: Some(E2E.into()),
        instant: Instant::No,
    }
}

fn account(m: &BankModel) -> Account {
    Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap()
}

/// The IDs of a `PaymentOutcomeUnknown`; panics on any other error.
fn unknown_ids(err: &FinTSError) -> (String, String) {
    let FinTSError::PaymentOutcomeUnknown {
        end_to_end_id,
        message_id,
    } = err
    else {
        panic!("outcome unknown expected: {err:?}")
    };
    assert_eq!(err.kind(), Category::OutcomeUnknown);
    (end_to_end_id.clone(), message_id.clone())
}

/// The transfer's step, which must be a decoupled `NeedTan`.
fn decoupled(step: Step<TransferReceipt>) -> jobs::Resume<TransferReceipt> {
    match step {
        Step::NeedTan(c, r) if matches!(c.kind, TanKind::Decoupled { .. }) => r,
        other => panic!("decoupled challenge expected: {other:?}"),
    }
}

// ── Cancel ──────────────────────────────────────────────────────────────

/// The user cancels in the client while the app shows the approval, then
/// approves in the app: the bank executes the transfer, and `cancel` said
/// so beforehand with the IDs the bank executed.
#[tokio::test]
async fn cancel_at_a_decoupled_tan_is_order_submitted_and_a_late_approval_executes() {
    for p in [payments(None), payments(vop(OnMatch::Skip3091))] {
        let handle = TestBank::new(model(p, Some(99))).handle();
        let m = handle.model();
        let mut dialog = open(&handle).await;
        let step = jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap();
        let resume = decoupled(step);
        let step = dialog.poll(resume).await.unwrap();
        let resume = decoupled(step);
        let sent = handle.requests().len();

        let cancelled = dialog.cancel(resume).unwrap();
        assert_eq!(handle.requests().len(), sent, "cancel sends nothing");
        let Cancelled::OrderSubmitted {
            end_to_end_id,
            message_id,
        } = cancelled
        else {
            panic!("the order is at the bank: {cancelled:?}")
        };
        assert_eq!(end_to_end_id, E2E);
        assert!(matches!(dialog.phase(), Phase::Open));
        dialog.end().await.unwrap();
        assert!(handle.payments().is_empty());

        assert_eq!(handle.approve_in_app(), 1, "late approval");
        let done = handle.payments();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].end_to_end_id, end_to_end_id);
        assert_eq!(done[0].msg_id, message_id);
        handle.assert_clean();
    }
}

/// At a typed TAN the order is at the bank too.
#[tokio::test]
async fn cancel_at_a_typed_tan_is_order_submitted() {
    let handle = TestBank::new(model(payments(None), None)).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap();
    let Step::NeedTan(_, resume) = step else {
        panic!("{step:?}")
    };
    assert!(matches!(
        dialog.cancel(resume).unwrap(),
        Cancelled::OrderSubmitted { end_to_end_id, .. } if end_to_end_id == E2E
    ));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// While the payee check runs, the bank has no VOP-ID to release and no
/// challenge: nothing can execute.
#[tokio::test]
async fn cancel_while_the_payee_check_runs_is_nothing_sent() {
    let p = payments(Some(VopModel {
        pending_polls: 1,
        ..VopModel::default()
    }));
    let handle = TestBank::new(model(p, Some(1))).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap();
    let Step::VopPending(_, resume) = step else {
        panic!("{step:?}")
    };
    assert_eq!(dialog.cancel(resume).unwrap(), Cancelled::NothingSent);
    // The dialog takes the next job.
    assert!(matches!(
        jobs::balance(&mut dialog, &account(&m)).await.unwrap(),
        Step::Done(_)
    ));
    dialog.end().await.unwrap();
    assert_eq!(handle.approve_in_app(), 0);
    assert!(handle.payments().is_empty());
    handle.assert_clean();
}

// ── Poll limit ──────────────────────────────────────────────────────────

/// The bank's `max_polls` used up on a transfer: the order is at the bank,
/// so the outcome is unknown, not a TAN failure. The dialog stays open.
#[tokio::test]
async fn poll_limit_on_a_transfer_is_outcome_unknown() {
    let mut m = model(payments(None), Some(99));
    m.decoupled_max_polls = 2;
    let handle = TestBank::new(m).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let mut step = jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap();
    let err = loop {
        let resume = decoupled(step);
        match dialog.poll(resume).await {
            Ok(next) => step = next,
            Err(e) => break e,
        }
    };
    let (end_to_end_id, _) = unknown_ids(&err);
    assert_eq!(end_to_end_id, E2E);
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

// ── Unreadable answers after the order ──────────────────────────────────

/// Rewrites the first segment `code` of an answer to the order (HKCCS)
/// with `edit`.
fn tamper_order_answer(m: BankModel, code: &'static [u8], edit: Edit) -> BankHandle {
    TestBank::new(m)
        .tamper_clear(move |req, segments| {
            if !req.has("HKCCS") {
                return segments;
            }
            segments
                .into_iter()
                .map(|s| if s.starts_with(code) { edit(s) } else { s })
                .collect()
        })
        .handle()
}

/// Rewrites one segment of an answer.
type Edit = fn(Vec<u8>) -> Vec<u8>;

/// The segment header (`CODE:n:v[:ref]`) with a body the codec rejects.
fn garbage_body(s: Vec<u8>) -> Vec<u8> {
    let head = s.iter().position(|&b| b == b'+').unwrap_or(s.len());
    let mut out = s[..head].to_vec();
    out.extend_from_slice(b"+X?:Y:Z:W:V+1:2:3:4:5:6:7:8+9+9+9+9+9+9+9+9+9+9+9+9'");
    out
}

/// The pain.002 report with a broken XML tag (same length, so the binary
/// length still fits).
fn broken_pain002(s: Vec<u8>) -> Vec<u8> {
    let at = s
        .windows(9)
        .position(|w| w == b"<Document")
        .expect("pain.002 in HIVPP");
    let mut s = s;
    s[at + 1..at + 9].copy_from_slice(b"Xocument");
    s
}

#[tokio::test]
async fn unreadable_answers_after_the_order_are_outcome_unknown() {
    let cases: [(&str, BankModel, &[u8], Edit); 3] = [
        (
            "HITAN",
            model(payments(None), None),
            b"HITAN:",
            garbage_body,
        ),
        (
            "HIVPP",
            model(payments(vop(OnMatch::Require)), None),
            b"HIVPP:",
            garbage_body,
        ),
        (
            "pain.002",
            model(
                payments(Some(VopModel {
                    result_as: ResultAs::Pain002,
                    ..vop(OnMatch::Require).unwrap()
                })),
                None,
            ),
            b"HIVPP:",
            broken_pain002,
        ),
    ];
    for (name, m, code, edit) in cases {
        let handle = tamper_order_answer(m, code, edit);
        let m = handle.model();
        let mut dialog = open(&handle).await;
        let err = jobs::transfer(&mut dialog, &transfer(&m))
            .await
            .expect_err(name);
        let (end_to_end_id, message_id) = unknown_ids(&err);
        assert_eq!(end_to_end_id, E2E, "{name}");
        assert!(message_id.starts_with("FC"), "{name}: {message_id}");
    }
}

/// A class 9 rejection after the order is definite, not unknown.
#[tokio::test]
async fn wrong_tan_on_a_transfer_is_a_rejection() {
    let mut m = model(payments(None), None);
    m.wrong_tan.kept = 0;
    let handle = TestBank::new(m).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap();
    let Step::NeedTan(_, resume) = step else {
        panic!("{step:?}")
    };
    let err = dialog.submit_tan(resume, "999999").await.unwrap_err();
    assert!(matches!(err, FinTSError::Bank(_)), "{err:?}");
    assert!(handle.payments().is_empty());
}

// ── Dropped future ──────────────────────────────────────────────────────

/// The caller drops `submit_tan` of a transfer while the bank (which has
/// already executed the order) has not answered: the next call reports
/// the payment's IDs once, the one after finds the dialog ended.
#[tokio::test]
async fn dropped_submit_tan_on_a_transfer_reports_the_ids_once() {
    let http = TestBank::new(model(payments(None), None))
        .fault(
            |r| r.has("HKTAN") && !r.has("HKCCS") && !r.has("HKIDN"),
            Fault::Silence(Duration::from_secs(30)),
        )
        .serve()
        .await;
    let m = http.handle().model();
    let mut dialog = open_with(&http.client(&ProductId::new("TEST-123")), &m).await;
    let step = jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap();
    let Step::NeedTan(_, resume) = step else {
        panic!("{step:?}")
    };
    let dropped =
        tokio::time::timeout(Duration::from_millis(300), dialog.submit_tan(resume, TAN)).await;
    assert!(dropped.is_err(), "the bank stays silent");

    let err = jobs::balance(&mut dialog, &account(&m)).await.unwrap_err();
    let (end_to_end_id, message_id) = unknown_ids(&err);
    assert_eq!(end_to_end_id, E2E);
    let done = http.handle().payments();
    assert_eq!(done.len(), 1, "the bank executed it");
    assert_eq!(done[0].msg_id, message_id);

    let err = jobs::balance(&mut dialog, &account(&m)).await.unwrap_err();
    assert!(matches!(err, FinTSError::DialogEnded), "{err:?}");
}

/// A dropped read only ends the dialog.
#[tokio::test]
async fn dropped_read_ends_the_dialog() {
    let http = TestBank::new(model(payments(None), None))
        .fault(|r| r.has("HKSAL"), Fault::Silence(Duration::from_secs(30)))
        .serve()
        .await;
    let m = http.handle().model();
    let mut dialog = open_with(&http.client(&ProductId::new("TEST-123")), &m).await;
    let dropped = tokio::time::timeout(
        Duration::from_millis(300),
        jobs::balance(&mut dialog, &account(&m)),
    )
    .await;
    assert!(dropped.is_err(), "the bank stays silent");
    let err = jobs::balance(&mut dialog, &account(&m)).await.unwrap_err();
    assert!(matches!(err, FinTSError::DialogEnded), "{err:?}");
}
