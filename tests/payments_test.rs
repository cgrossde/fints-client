//! The transfer job against the synthetic test bank (payments design,
//! section 7 "Tests"): the answer table of section 3.2, the payee check
//! with and without HKVPA, polling, instant choice, freeze and thaw at
//! every wait, unknown outcomes and validation. All names, IBANs and
//! amounts are synthetic.

use std::sync::Arc;

use fints_client::codec::Jn;
use fints_client::codec::defs::{HkIpz2, HkTan6, HkTan7, HkVpp1};
use fints_client::jobs::{
    self, AnyResume, BankClient, Cancelled, Credentials, Dialog, InstantStatus, JobKind,
    PausedDialog, Phase, SentAs, SepaOrderStatus, StatusSource, Step, Support, TransferReceipt,
    TransferStatus,
};
use fints_client::{
    Account, BankConfig, Category, FinTSError, Instant, Pin, ProductId, SessionState, Transfer,
    UserId,
};
use fints_client::{PayeeOutcome, TransferField, VopResult};
use fints_client_testbank::{
    BankHandle, BankModel, Booking, Delivery, Fault, InstantStatusModel, JobSpec, OnMatch, Payee,
    PaymentsModel, Request, ResultAs, TanMethod, TestBank, VopModel, iban_de, profiles,
};
use rust_decimal::Decimal;

const TAN: &str = "123456";

fn anna() -> String {
    iban_de("87654321", "0000000042")
}
fn berta() -> String {
    iban_de("87654321", "0000000043")
}
fn no_account() -> String {
    iban_de("87654321", "0000000044")
}
fn unknown() -> String {
    iban_de("87654321", "0000000045")
}

fn payments(vop: Option<VopModel>) -> PaymentsModel {
    PaymentsModel {
        payees: vec![
            Payee::new(&anna(), "Anna Beispiel"),
            Payee::new(&berta(), "Berta Maria Muster"),
        ],
        not_applicable: vec![(no_account(), "Kein Zahlungskonto".into())],
        vop,
        ..PaymentsModel::default()
    }
}

fn vop(on_match: OnMatch) -> Option<VopModel> {
    Some(VopModel {
        hkvpa_on_match: on_match,
        explanation: Some("Bitte den Namen pruefen".into()),
        ..VopModel::default()
    })
}

fn model(p: PaymentsModel, decoupled: bool) -> BankModel {
    let mut m = BankModel {
        hispas_national: Some(false),
        ..BankModel::default()
    };
    if decoupled {
        m.tan_methods = vec![TanMethod::decoupled(940, "Test-App", 2)];
        m.hitans_version = 7;
    }
    m.enable_payments(p);
    m
}

fn bank(p: PaymentsModel, decoupled: bool) -> BankHandle {
    TestBank::new(model(p, decoupled)).handle()
}

fn client(handle: &BankHandle) -> BankClient {
    let m = handle.model();
    let config = BankConfig::new(m.name, m.blz, m.bic, "in-process://testbank");
    BankClient::new(config, ProductId::new("TEST-123")).with_transport(Arc::new(handle.clone()))
}

fn creds(m: &BankModel) -> Credentials {
    Credentials::new(UserId::new(&m.customer.user_id), Pin::new(&m.customer.pin))
}

async fn open_with(client: &BankClient, m: &BankModel) -> Dialog {
    let state = SessionState::new(
        &fints_client::Blz::new(&m.blz),
        &UserId::new(&m.customer.user_id),
    );
    let (dialog, step) = jobs::open(client, &creds(m), &state).await.unwrap();
    assert!(matches!(step, Step::Done(_)), "login is exempt");
    dialog
}

async fn open(handle: &BankHandle) -> Dialog {
    open_with(&client(handle), &handle.model()).await
}

fn transfer(m: &BankModel, iban: &str, name: &str, instant: Instant) -> Transfer {
    Transfer {
        from: Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap(),
        debtor_name: None,
        payee_name: name.into(),
        payee_iban: iban.into(),
        payee_bic: None,
        amount: Decimal::new(1250, 2),
        purpose: "Testzahlung".into(),
        end_to_end_id: Some("E2E-TEST-1".into()),
        instant,
    }
}

/// The business segments of a request (no envelope, no signature).
fn business(r: &Request) -> Vec<&str> {
    r.codes()
        .into_iter()
        .filter(|c| !c.starts_with("HN"))
        .collect()
}

/// Answers every step as a caller would (TAN, poll, confirm) and records
/// the step names on the way.
async fn finish(
    dialog: &mut Dialog,
    mut step: Step<TransferReceipt>,
    seen: &mut Vec<&'static str>,
) -> TransferReceipt {
    loop {
        step = match step {
            Step::Done(d) => {
                seen.push("Done");
                return d.value;
            }
            Step::NeedTan(c, r) => {
                seen.push("NeedTan");
                match c.kind {
                    jobs::TanKind::Typed { .. } => dialog.submit_tan(r, TAN).await.unwrap(),
                    jobs::TanKind::Decoupled { .. } => dialog.poll(r).await.unwrap(),
                }
            }
            Step::NeedVopConfirmation(_, r) => {
                seen.push("NeedVopConfirmation");
                dialog.confirm_vop(r).await.unwrap()
            }
            Step::VopPending(_, r) => {
                seen.push("VopPending");
                dialog.poll(r).await.unwrap()
            }
        };
    }
}

async fn run(handle: &BankHandle, iban: &str, name: &str) -> (TransferReceipt, Vec<&'static str>) {
    let m = handle.model();
    let mut dialog = open(handle).await;
    let step = jobs::transfer(&mut dialog, &transfer(&m, iban, name, Instant::No))
        .await
        .unwrap();
    let mut seen = Vec::new();
    let receipt = finish(&mut dialog, step, &mut seen).await;
    dialog.end().await.unwrap();
    handle.assert_clean();
    (receipt, seen)
}

/// The requests after the login.
fn payment_requests(handle: &BankHandle) -> Vec<Vec<String>> {
    handle
        .requests()
        .iter()
        .map(|r| {
            business(r)
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .filter(|c| {
            c.iter()
                .any(|s| matches!(s.as_str(), "HKCCS" | "HKIPZ" | "HKVPP" | "HKVPA" | "HKTAN"))
                && !c.iter().any(|s| s == "HKIDN" || s == "HKEND")
        })
        .collect()
}

fn codes(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn assert_one_payment(handle: &BankHandle, iban: &str) {
    let done = handle.payments();
    assert_eq!(done.len(), 1, "exactly one execution");
    assert_eq!(done[0].payee_iban, iban);
    assert_eq!(done[0].end_to_end_id, "E2E-TEST-1");
}

// ── Clean match ─────────────────────────────────────────────────────────

#[tokio::test]
async fn match_with_3091_typed_sends_no_hkvpa() {
    let handle = bank(payments(vop(OnMatch::Skip3091)), false);
    let (receipt, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["NeedTan", "Done"]);
    assert_eq!(
        payment_requests(&handle),
        [codes(&["HKVPP", "HKCCS", "HKTAN"]), codes(&["HKTAN"])]
    );
    assert_eq!(receipt.sent_as, SentAs::Standard);
    assert_eq!(receipt.status, TransferStatus::Executed);
    assert_eq!(receipt.end_to_end_id, "E2E-TEST-1");
    assert!(receipt.payee_check.as_ref().unwrap().all_match());
    assert_one_payment(&handle, &anna());
}

#[tokio::test]
async fn match_with_3091_decoupled_polls_without_hkvpa() {
    let handle = bank(payments(vop(OnMatch::Skip3091)), true);
    let (_, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["NeedTan", "NeedTan", "Done"]);
    let requests = payment_requests(&handle);
    assert_eq!(requests[0], codes(&["HKVPP", "HKCCS", "HKTAN"]));
    assert!(requests[1..].iter().all(|r| *r == codes(&["HKTAN"])));
    assert_one_payment(&handle, &anna());
}

#[tokio::test]
async fn match_without_3091_typed_sends_hkvpa_with_the_tan() {
    let handle = bank(payments(vop(OnMatch::Require)), false);
    let (receipt, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["NeedTan", "Done"]);
    assert_eq!(
        payment_requests(&handle),
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPA", "HKTAN"])
        ]
    );
    let tan: HkTan6 = handle
        .requests()
        .iter()
        .find(|r| r.has("HKVPA"))
        .and_then(|r| r.get())
        .unwrap();
    assert_eq!(tan.process.as_str(), "2");
    assert_eq!(receipt.status, TransferStatus::Executed);
    assert_one_payment(&handle, &anna());
}

#[tokio::test]
async fn match_without_3091_decoupled_sends_hkvpa_on_the_first_poll_only() {
    let handle = bank(payments(vop(OnMatch::Require)), true);
    let (_, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["NeedTan", "NeedTan", "Done"]);
    let requests = payment_requests(&handle);
    assert_eq!(
        requests,
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPA", "HKTAN"]),
            codes(&["HKTAN"])
        ]
    );
    let first_poll = handle
        .requests()
        .into_iter()
        .find(|r| r.has("HKVPA"))
        .unwrap();
    let tan: HkTan7 = first_poll.get().unwrap();
    assert_eq!(tan.process.as_str(), "S");
    assert_one_payment(&handle, &anna());
}

/// 3945 on a clean match: no confirmation step; the execution order goes
/// out at once.
#[tokio::test]
async fn match_with_3945_sends_the_execution_order_at_once() {
    let handle = bank(payments(vop(OnMatch::Always3945)), false);
    let (receipt, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["NeedTan", "Done"]);
    assert_eq!(
        payment_requests(&handle),
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPA", "HKCCS", "HKTAN"]),
            codes(&["HKTAN"])
        ]
    );
    assert!(receipt.payee_check.unwrap().all_match());
    assert_one_payment(&handle, &anna());
}

/// A match the pain.002 does not list (Anlage 3 lists only non-RCVC
/// transactions) is a clean match by its group counts.
#[tokio::test]
async fn match_not_listed_in_the_report_is_clean() {
    let mut p = payments(vop(OnMatch::Skip3091));
    if let Some(v) = p.vop.as_mut() {
        v.list_matches = false;
    }
    let handle = bank(p, false);
    let (receipt, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["NeedTan", "Done"]);
    assert!(receipt.payee_check.unwrap().all_match());
}

// ── Deviations ──────────────────────────────────────────────────────────

#[tokio::test]
async fn close_no_and_not_applicable_ask_the_user() {
    for (iban, name, result_as) in [
        (berta(), "Berta Muster", ResultAs::Pain002),
        (unknown(), "Carla Probe", ResultAs::Pain002),
        (no_account(), "Dora Test", ResultAs::Pain002),
        (berta(), "Berta Muster", ResultAs::SingleDeg),
    ] {
        let mut p = payments(vop(OnMatch::Require));
        if let Some(v) = p.vop.as_mut() {
            v.result_as = result_as;
        }
        let handle = bank(p, false);
        let m = handle.model();
        let mut dialog = open(&handle).await;
        let step = jobs::transfer(&mut dialog, &transfer(&m, &iban, name, Instant::No))
            .await
            .unwrap();
        let Step::NeedVopConfirmation(c, resume) = step else {
            panic!("confirmation expected: {step:?}")
        };
        let outcome = &c.result.payees[0].outcome;
        match iban.as_str() {
            i if i == berta() => assert_eq!(
                *outcome,
                PayeeOutcome::CloseMatch {
                    registered_name: Some("Berta Maria Muster".into())
                }
            ),
            i if i == unknown() => assert_eq!(*outcome, PayeeOutcome::NoMatch),
            _ => assert!(matches!(outcome, PayeeOutcome::NotApplicable { .. })),
        }
        assert_eq!(c.explanation.as_deref(), Some("Bitte den Namen pruefen"));
        assert!(c.valid_until.is_some());
        assert!(handle.payments().is_empty());
        let step = dialog.confirm_vop(resume).await.unwrap();
        let Step::NeedTan(tan, resume) = step else {
            panic!("TAN expected: {step:?}")
        };
        assert_eq!(tan.payee_check.as_ref(), Some(&c.result));
        let receipt = match dialog.submit_tan(resume, TAN).await.unwrap() {
            Step::Done(d) => d.value,
            other => panic!("{other:?}"),
        };
        assert_eq!(receipt.payee_check, Some(c.result.clone()));
        assert_eq!(
            payment_requests(&handle),
            [
                codes(&["HKVPP", "HKCCS", "HKTAN"]),
                codes(&["HKVPA", "HKCCS", "HKTAN"]),
                codes(&["HKTAN"])
            ]
        );
        assert_one_payment(&handle, &iban);
        handle.assert_clean();
    }
}

#[tokio::test]
async fn cancel_at_the_confirmation_executes_nothing() {
    let handle = bank(payments(vop(OnMatch::Require)), false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &unknown(), "Carla Probe", Instant::No),
    )
    .await
    .unwrap();
    let Step::NeedVopConfirmation(_, resume) = step else {
        panic!("{step:?}")
    };
    let before = handle.requests().len();
    assert_eq!(dialog.cancel(resume).unwrap(), Cancelled::NothingSent);
    assert_eq!(handle.requests().len(), before, "cancel sends nothing");
    assert!(matches!(dialog.phase(), Phase::Open));
    // The dialog takes the next job.
    let account = Account::new(m.accounts[0].iban.as_deref().unwrap(), &m.bic).unwrap();
    assert!(matches!(
        jobs::balance(&mut dialog, &account).await.unwrap(),
        Step::Done(_)
    ));
    dialog.end().await.unwrap();
    assert!(handle.payments().is_empty());
    handle.assert_clean();
}

// ── Polling ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn polling_with_complete_delivery() {
    let p = payments(Some(VopModel {
        pending_polls: 2,
        wait_secs: 3,
        delivery: Delivery::Complete,
        ..VopModel::default()
    }));
    let handle = bank(p, false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let started = jiff::Timestamp::now();
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::No),
    )
    .await
    .unwrap();
    let Step::VopPending(wait, _) = &step else {
        panic!("{step:?}")
    };
    let not_before = wait.not_before.expect("wait from HIVPP");
    assert!(not_before >= started + jiff::SignedDuration::from_secs(3));
    assert_eq!(wait.polls_done, 0);
    let mut seen = Vec::new();
    let receipt = finish(&mut dialog, step, &mut seen).await;
    assert_eq!(seen, ["VopPending", "VopPending", "NeedTan", "Done"]);
    assert_eq!(
        payment_requests(&handle),
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPP"]),
            codes(&["HKVPP"]),
            codes(&["HKVPA", "HKCCS", "HKTAN"]),
            codes(&["HKTAN"])
        ]
    );
    let polls: Vec<HkVpp1> = handle
        .requests()
        .iter()
        .filter(|r| business(r) == ["HKVPP"])
        .filter_map(|r| r.get())
        .collect();
    assert_eq!(
        polls
            .iter()
            .map(|p| p.touchdown.as_ref().unwrap().as_str().to_string())
            .collect::<Vec<_>>(),
        ["VP-1", "VP-2"]
    );
    assert!(receipt.payee_check.unwrap().all_match());
    assert_one_payment(&handle, &anna());
    handle.assert_clean();
}

#[tokio::test]
async fn polling_with_stepwise_delivery_merges_the_parts() {
    let p = payments(Some(VopModel {
        pending_polls: 1,
        delivery: Delivery::Stepwise,
        ..VopModel::default()
    }));
    let handle = bank(p, false);
    let (receipt, seen) = run(&handle, &berta(), "Berta Muster").await;
    assert_eq!(
        seen,
        ["VopPending", "NeedVopConfirmation", "NeedTan", "Done"]
    );
    assert!(matches!(
        receipt.payee_check.unwrap().payees[0].outcome,
        PayeeOutcome::CloseMatch { .. }
    ));
    assert_one_payment(&handle, &berta());
}

// ── Without payee check ─────────────────────────────────────────────────

#[tokio::test]
async fn bank_without_hivpps_sends_no_payee_check() {
    let handle = bank(payments(None), false);
    let (receipt, seen) = run(&handle, &unknown(), "Carla Probe").await;
    assert_eq!(seen, ["NeedTan", "Done"]);
    assert_eq!(
        payment_requests(&handle),
        [codes(&["HKCCS", "HKTAN"]), codes(&["HKTAN"])]
    );
    assert_eq!(receipt.payee_check, None);
    assert_one_payment(&handle, &unknown());
}

// ── Instant ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn instant_require_not_offered_sends_nothing() {
    let handle = bank(
        PaymentsModel {
            hkipz_versions: Vec::new(),
            ..payments(None)
        },
        false,
    );
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let before = handle.requests().len();
    let err = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Require),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FinTSError::VersionMismatch(_)), "{err:?}");
    assert_eq!(handle.requests().len(), before);
    // Prefer falls back before sending.
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Prefer),
    )
    .await
    .unwrap();
    let mut seen = Vec::new();
    let receipt = finish(&mut dialog, step, &mut seen).await;
    assert_eq!(receipt.sent_as, SentAs::Standard);
    assert_eq!(handle.payments()[0].order, "HKCCS");
    handle.assert_clean();
}

#[tokio::test]
async fn instant_prefer_sends_hkipz_when_offered() {
    let handle = bank(payments(None), false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Prefer),
    )
    .await
    .unwrap();
    let receipt = finish(&mut dialog, step, &mut Vec::new()).await;
    assert_eq!(receipt.sent_as, SentAs::Instant);
    assert_eq!(receipt.order_id.as_deref(), Some("INST0001"));
    assert!(handle.payments()[0].instant);
    handle.assert_clean();
}

#[tokio::test]
async fn instant_limit_rejection_has_no_fallback() {
    let handle = bank(
        PaymentsModel {
            instant_limit: Some(Decimal::new(1000, 2)),
            ..payments(None)
        },
        false,
    );
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let err = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Prefer),
    )
    .await
    .unwrap_err();
    let FinTSError::Bank(e) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(e.code().code.as_str(), "9255");
    assert_eq!(err.kind(), Category::Rejected);
    assert_eq!(payment_requests(&handle), [codes(&["HKIPZ", "HKTAN"])]);
    assert!(handle.payments().is_empty());
    assert!(matches!(dialog.phase(), Phase::Open));
    handle.assert_clean();
}

#[tokio::test]
async fn instant_conversion_flag_and_3270() {
    let handle = bank(
        PaymentsModel {
            hkipz_versions: vec![2],
            hkipz_conversion_allowed: true,
            instant_limit: Some(Decimal::new(1000, 2)),
            ..payments(None)
        },
        false,
    );
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Require),
    )
    .await
    .unwrap();
    let receipt = finish(&mut dialog, step, &mut Vec::new()).await;
    assert_eq!(receipt.sent_as, SentAs::InstantConverted);
    let order: HkIpz2 = handle
        .requests()
        .iter()
        .find(|r| r.has("HKIPZ"))
        .and_then(|r| r.get())
        .unwrap();
    assert_eq!(order.convert_to_sepa_allowed, Some(Jn(true)));
    assert!(handle.payments()[0].converted);
    handle.assert_clean();
}

#[tokio::test]
async fn instant_3045_ends_with_instant_pending() {
    let handle = bank(
        PaymentsModel {
            instant_status_after: 1,
            hkips: Some(InstantStatusModel {
                min_wait: 5,
                formats: Vec::new(),
            }),
            ..payments(None)
        },
        false,
    );
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let started = jiff::Timestamp::now();
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Require),
    )
    .await
    .unwrap();
    let receipt = finish(&mut dialog, step, &mut Vec::new()).await;
    let TransferStatus::InstantPending {
        order_id,
        not_before,
    } = receipt.status
    else {
        panic!("{receipt:?}")
    };
    assert_eq!(order_id.as_deref(), Some("INST0001"));
    assert!(not_before.unwrap() >= started + jiff::SignedDuration::from_secs(5));
}

// ── Freeze and thaw ─────────────────────────────────────────────────────

/// Freeze through JSON into a new client; returns the transfer ticket.
fn round_trip(dialog: Dialog, handle: &BankHandle) -> (Dialog, PausedDialog) {
    let json = dialog.freeze().unwrap().to_json().unwrap();
    assert!(json.contains("fints-client-paused/4"));
    let paused = PausedDialog::from_json(&json).unwrap();
    let copy = PausedDialog::from_json(&json).unwrap();
    let m = handle.model();
    (
        Dialog::thaw(paused, &client(handle), &creds(&m)).unwrap(),
        copy,
    )
}

fn ticket(paused: &PausedDialog) -> jobs::Resume<TransferReceipt> {
    match paused.pending() {
        Some(AnyResume::Transfer(r)) => r,
        other => panic!("transfer ticket expected: {other:?}"),
    }
}

#[tokio::test]
async fn freeze_and_thaw_at_every_wait() {
    let p = payments(Some(VopModel {
        pending_polls: 1,
        explanation: Some("Bitte den Namen pruefen".into()),
        ..VopModel::default()
    }));
    let handle = bank(p, false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &berta(), "Berta Muster", Instant::No),
    )
    .await
    .unwrap();
    assert!(matches!(step, Step::VopPending(..)));

    // VoP poll.
    let (mut dialog, paused) = round_trip(dialog, &handle);
    assert!(paused.challenge().is_none() && paused.vop().is_none());
    let step = dialog.poll(ticket(&paused)).await.unwrap();
    let Step::NeedVopConfirmation(c, _) = &step else {
        panic!("{step:?}")
    };
    let shown = c.clone();

    // Confirmation.
    let (mut dialog, paused) = round_trip(dialog, &handle);
    assert_eq!(paused.vop(), Some(&shown));
    let step = dialog.confirm_vop(ticket(&paused)).await.unwrap();
    assert!(matches!(step, Step::NeedTan(..)));

    // TAN.
    let (mut dialog, paused) = round_trip(dialog, &handle);
    assert_eq!(
        paused.challenge().and_then(|c| c.payee_check.as_ref()),
        Some(&shown.result)
    );
    let receipt = match dialog.submit_tan(ticket(&paused), TAN).await.unwrap() {
        Step::Done(d) => d.value,
        other => panic!("{other:?}"),
    };
    assert_eq!(receipt.status, TransferStatus::Executed);
    dialog.end().await.unwrap();
    assert_one_payment(&handle, &berta());
    handle.assert_clean();
}

/// A ticket of another wait does not resume the job.
#[tokio::test]
async fn confirm_vop_on_a_tan_wait_is_refused() {
    let handle = bank(payments(vop(OnMatch::Require)), false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::No),
    )
    .await
    .unwrap();
    assert!(matches!(step, Step::NeedTan(..)));
    let (mut dialog, paused) = round_trip(dialog, &handle);
    let err = dialog.confirm_vop(ticket(&paused)).await.unwrap_err();
    assert!(matches!(err, FinTSError::Dialog(_)), "{err:?}");
    assert!(matches!(dialog.phase(), Phase::Paused(_)));
    let receipt = match dialog.submit_tan(ticket(&paused), TAN).await.unwrap() {
        Step::Done(d) => d.value,
        other => panic!("{other:?}"),
    };
    assert_eq!(receipt.status, TransferStatus::Executed);
}

// ── Unknown outcome ─────────────────────────────────────────────────────

#[tokio::test]
async fn cut_wire_after_message_1_is_outcome_unknown_and_nothing_executed() {
    let http = TestBank::new(model(payments(vop(OnMatch::Require)), false))
        .fault(|r| r.has("HKCCS"), Fault::CutWire(60))
        .serve()
        .await;
    let m = http.handle().model();
    let client = http.client(&ProductId::new("TEST-123"));
    let mut dialog = open_with(&client, &m).await;
    let err = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::No),
    )
    .await
    .unwrap_err();
    let FinTSError::PaymentOutcomeUnknown {
        end_to_end_id,
        message_id,
    } = &err
    else {
        panic!("{err:?}")
    };
    assert_eq!(end_to_end_id, "E2E-TEST-1");
    assert!(message_id.starts_with("FC"));
    assert_eq!(err.kind(), Category::OutcomeUnknown);
    assert!(matches!(dialog.phase(), Phase::Ended));
    assert!(http.handle().payments().is_empty());
}

#[tokio::test]
async fn cut_wire_after_the_tan_is_outcome_unknown_and_executed_once() {
    let http = TestBank::new(model(payments(vop(OnMatch::Require)), false))
        .serve()
        .await;
    let m = http.handle().model();
    let client = http.client(&ProductId::new("TEST-123"));
    let mut dialog = open_with(&client, &m).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::No),
    )
    .await
    .unwrap();
    let Step::NeedTan(_, resume) = step else {
        panic!("{step:?}")
    };
    http.handle()
        .inject(|r| r.has("HKTAN") && !r.has("HKCCS"), Fault::CutWire(60));
    let err = dialog.submit_tan(resume, TAN).await.unwrap_err();
    assert!(
        matches!(err, FinTSError::PaymentOutcomeUnknown { .. }),
        "{err:?}"
    );
    assert!(matches!(dialog.phase(), Phase::Ended));
    assert_eq!(http.handle().payments().len(), 1, "executed exactly once");
}

/// A cut HKVPP poll is a plain transport error: nothing was ordered.
#[tokio::test]
async fn cut_wire_on_a_payee_check_poll_is_a_plain_error() {
    let p = payments(Some(VopModel {
        pending_polls: 1,
        ..VopModel::default()
    }));
    let http = TestBank::new(model(p, false)).serve().await;
    let m = http.handle().model();
    let client = http.client(&ProductId::new("TEST-123"));
    let mut dialog = open_with(&client, &m).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::No),
    )
    .await
    .unwrap();
    let Step::VopPending(_, resume) = step else {
        panic!("{step:?}")
    };
    http.handle()
        .inject(|r| r.has("HKVPP") && !r.has("HKCCS"), Fault::CutWire(60));
    let err = dialog.poll(resume).await.unwrap_err();
    assert!(
        !matches!(err, FinTSError::PaymentOutcomeUnknown { .. }),
        "{err:?}"
    );
    assert!(http.handle().payments().is_empty());
}

// ── Validation ──────────────────────────────────────────────────────────

#[tokio::test]
async fn invalid_transfers_send_nothing() {
    let handle = bank(payments(vop(OnMatch::Require)), false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let base = transfer(&m, &anna(), "Anna Beispiel", Instant::No);
    let cases: Vec<(TransferField, Transfer)> = vec![
        (
            TransferField::PayeeIban,
            Transfer {
                payee_iban: "DE00123456780000000042".into(),
                ..base.clone()
            },
        ),
        (
            TransferField::PayeeName,
            Transfer {
                payee_name: String::new(),
                ..base.clone()
            },
        ),
        (
            TransferField::Amount,
            Transfer {
                amount: Decimal::ZERO,
                ..base.clone()
            },
        ),
        (
            TransferField::Purpose,
            Transfer {
                purpose: "x".repeat(141),
                ..base.clone()
            },
        ),
        (
            TransferField::PayeeBic,
            Transfer {
                payee_bic: Some("BIC".into()),
                ..base.clone()
            },
        ),
    ];
    let before = handle.requests().len();
    for (field, t) in cases {
        match jobs::transfer(&mut dialog, &t).await {
            Err(FinTSError::InvalidTransfer(e)) => assert_eq!(e.field, field),
            other => panic!("{field:?}: {other:?}"),
        }
    }
    assert_eq!(handle.requests().len(), before, "nothing sent");
    assert!(matches!(dialog.phase(), Phase::Open));
}

// ── Spec readings (review findings) ─────────────────────────────────────

fn vop_with(on_match: OnMatch, set: impl FnOnce(&mut VopModel)) -> Option<VopModel> {
    let mut v = vop(on_match).unwrap();
    set(&mut v);
    Some(v)
}

/// Runs a transfer to its end and returns the receipt, the step names and
/// the notices of `Done`.
async fn run_with_notices(
    handle: &BankHandle,
    iban: &str,
    name: &str,
) -> (TransferReceipt, Vec<&'static str>, Vec<String>) {
    let m = handle.model();
    let mut dialog = open(handle).await;
    let mut step = jobs::transfer(&mut dialog, &transfer(&m, iban, name, Instant::No))
        .await
        .unwrap();
    let mut seen = Vec::new();
    let done = loop {
        step = match step {
            Step::Done(d) => {
                seen.push("Done");
                break d;
            }
            Step::NeedTan(_, r) => {
                seen.push("NeedTan");
                dialog.submit_tan(r, TAN).await.unwrap()
            }
            Step::NeedVopConfirmation(_, r) => {
                seen.push("NeedVopConfirmation");
                dialog.confirm_vop(r).await.unwrap()
            }
            Step::VopPending(_, r) => {
                seen.push("VopPending");
                dialog.poll(r).await.unwrap()
            }
        };
    };
    dialog.end().await.unwrap();
    handle.assert_clean();
    let notices = done
        .notices
        .iter()
        .filter_map(|n| n.code().map(|c| c.code.to_string()))
        .collect();
    (done.value, seen, notices)
}

/// A pending answer without a polling ID is still pending (VoP C.10.7.1.1:
/// HKVPP field 3 only when the bank sent one, next to a touchdown point).
#[tokio::test]
async fn pending_without_a_polling_id_polls_with_the_touchdown_only() {
    let handle = bank(
        payments(vop_with(OnMatch::Require, |v| {
            v.pending_polls = 1;
            v.polling_id = false;
        })),
        false,
    );
    let (receipt, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["VopPending", "NeedTan", "Done"]);
    let poll: HkVpp1 = handle
        .requests()
        .iter()
        .find(|r| business(r) == ["HKVPP"])
        .and_then(|r| r.get())
        .unwrap();
    assert_eq!(poll.polling_id, None);
    assert_eq!(poll.touchdown.unwrap().as_str(), "VP-1");
    assert!(receipt.payee_check.unwrap().all_match());
    assert_one_payment(&handle, &anna());
}

/// A final answer without a result (3095) and without 3090 or a text is
/// clean: no confirmation; with 3945 the execution order goes at once.
#[tokio::test]
async fn empty_result_is_clean() {
    for (on_match, requests) in [
        (
            OnMatch::Always3945,
            vec![
                codes(&["HKVPP", "HKCCS", "HKTAN"]),
                codes(&["HKVPA", "HKCCS", "HKTAN"]),
                codes(&["HKTAN"]),
            ],
        ),
        (
            OnMatch::Require,
            vec![
                codes(&["HKVPP", "HKCCS", "HKTAN"]),
                codes(&["HKVPA", "HKTAN"]),
            ],
        ),
    ] {
        let handle = bank(
            payments(vop_with(on_match, |v| v.empty_result = true)),
            false,
        );
        let (receipt, seen, notices) = run_with_notices(&handle, &anna(), "Anna Beispiel").await;
        assert_eq!(seen, ["NeedTan", "Done"], "{on_match:?}");
        assert_eq!(payment_requests(&handle), requests, "{on_match:?}");
        assert_eq!(receipt.payee_check, Some(VopResult::empty()));
        assert!(notices.iter().any(|c| c == "3095"), "{notices:?}");
        assert_one_payment(&handle, &anna());
    }
}

/// A payee that did not match goes through the execution order after the
/// user confirms, even when message 1 brought a challenge (VoP E.8.1
/// point 4).
#[tokio::test]
async fn deviation_with_a_challenge_still_sends_the_execution_order() {
    let handle = bank(
        payments(vop_with(OnMatch::Require, |v| {
            v.challenge_on_deviation = true
        })),
        false,
    );
    let (receipt, seen) = run(&handle, &unknown(), "Carla Probe").await;
    assert_eq!(seen, ["NeedVopConfirmation", "NeedTan", "Done"]);
    assert_eq!(
        payment_requests(&handle),
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPA", "HKCCS", "HKTAN"]),
            codes(&["HKTAN"])
        ]
    );
    assert_eq!(
        receipt.payee_check.unwrap().payees[0].outcome,
        PayeeOutcome::NoMatch
    );
    assert_one_payment(&handle, &unknown());
}

/// A match with 3090 asks the user; the challenge of message 1 is answered
/// after the confirmation, without a new order.
#[tokio::test]
async fn match_with_3090_reuses_the_first_challenge() {
    let handle = bank(
        payments(vop_with(OnMatch::Require, |v| v.deviation_on_match = true)),
        false,
    );
    let (receipt, seen) = run(&handle, &anna(), "Anna Beispiel").await;
    assert_eq!(seen, ["NeedVopConfirmation", "NeedTan", "Done"]);
    assert_eq!(
        payment_requests(&handle),
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPA", "HKTAN"])
        ]
    );
    assert!(receipt.payee_check.unwrap().all_match());
    assert_one_payment(&handle, &anna());
}

/// A pain.002 with the status only as `PmtInfSts` (no `TxInfAndSts`, no
/// counts): the block status is the single transaction's status.
#[tokio::test]
async fn report_with_the_block_status_only() {
    for (iban, name, outcome) in [
        (unknown(), "Carla Probe", PayeeOutcome::NoMatch),
        (anna(), "Anna Beispiel", PayeeOutcome::Match),
    ] {
        let handle = bank(
            payments(vop_with(OnMatch::Require, |v| v.block_status_only = true)),
            false,
        );
        let (receipt, _) = run(&handle, &iban, name).await;
        let result = receipt.payee_check.unwrap();
        assert_eq!(result.payees[0].outcome, outcome);
        assert_eq!(result.payees[0].iban, iban);
        assert_one_payment(&handle, &iban);
    }
}

/// HKIPZ needs `LclInstrm INST`, which pain.001.003.03 lacks: a bank that
/// offers only 001.003.03 gets no instant order (Require: mismatch with
/// both format lists; Prefer: HKCCS). The test bank keeps 9150 for an
/// HKIPZ without INST.
#[tokio::test]
async fn instant_skips_pain_001_003_03() {
    const P03: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.003.03";
    let handle = bank(
        PaymentsModel {
            hkccs_formats: vec![P03],
            hkipz_formats: vec![P03],
            ..payments(vop(OnMatch::Skip3091))
        },
        false,
    );
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let before = handle.requests().len();
    let err = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Require),
    )
    .await
    .unwrap_err();
    let FinTSError::VersionMismatch(mismatch) = &err else {
        panic!("{err:?}")
    };
    let formats = mismatch.formats.as_ref().expect("format lists");
    assert_eq!(formats.offered, [P03]);
    assert_eq!(formats.implemented, ["pain.001.001.09"]);
    assert_eq!(handle.requests().len(), before, "nothing sent");
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::Prefer),
    )
    .await
    .unwrap();
    let receipt = finish(&mut dialog, step, &mut Vec::new()).await;
    assert_eq!(receipt.sent_as, SentAs::Standard);
    assert!(receipt.payee_check.unwrap().all_match());
    assert_eq!(handle.payments()[0].order, "HKCCS");
    dialog.end().await.unwrap();
    handle.assert_clean();
}

// ── Instant status (HKIPS) ──────────────────────────────────────────────

fn instant_bank(after: u32, tan_for_status: bool) -> BankHandle {
    let p = PaymentsModel {
        instant_status_after: after,
        hkips: Some(InstantStatusModel {
            min_wait: 5,
            formats: Vec::new(),
        }),
        ..payments(None)
    };
    let mut m = model(p, false);
    if tan_for_status {
        m.jobs.insert(
            "HKIPS",
            JobSpec {
                hipins_needs_tan: true,
                ..JobSpec::new(&[])
            },
        );
    }
    TestBank::new(m).handle()
}

/// An instant transfer that ends with 3045; returns its receipt.
async fn pending_instant(dialog: &mut Dialog, m: &BankModel) -> TransferReceipt {
    let step = jobs::transfer(
        dialog,
        &transfer(m, &anna(), "Anna Beispiel", Instant::Require),
    )
    .await
    .unwrap();
    let receipt = finish(dialog, step, &mut Vec::new()).await;
    match &receipt.status {
        TransferStatus::InstantPending { order_id, .. } => assert!(order_id.is_some()),
        other => panic!("{other:?}"),
    }
    receipt
}

/// A receipt of an instant payment sent today, still pending.
fn instant_receipt(order_id: Option<&str>, end_to_end_id: &str) -> TransferReceipt {
    TransferReceipt {
        sent_as: SentAs::Instant,
        status: TransferStatus::InstantPending {
            order_id: order_id.map(str::to_string),
            not_before: None,
        },
        sent_on: jiff::ZonedDateTime::now().date(),
        message_id: "MSG-0001".into(),
        end_to_end_id: end_to_end_id.into(),
        order_id: order_id.map(str::to_string),
        bank_reference: None,
        payee_check: None,
    }
}

fn done_status(step: Step<InstantStatus>) -> InstantStatus {
    match step {
        Step::Done(d) => d.value,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn instant_status_asks_again_then_is_final() {
    let handle = instant_bank(2, false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let receipt = pending_instant(&mut dialog, &m).await;
    let from = transfer(&m, &anna(), "Anna Beispiel", Instant::No).from;

    let before = jiff::Timestamp::now();
    let first = done_status(
        jobs::instant_status(&mut dialog, &from, &receipt)
            .await
            .unwrap(),
    );
    assert_eq!(first.status, None);
    assert!(first.again_after.unwrap() >= before + jiff::SignedDuration::from_secs(5));

    let second = done_status(
        jobs::instant_status(&mut dialog, &from, &receipt)
            .await
            .unwrap(),
    );
    assert_eq!(
        second,
        InstantStatus {
            status: Some(SepaOrderStatus::Executed),
            again_after: None,
            source: StatusSource::StatusRequest,
        }
    );
    dialog.end().await.unwrap();
    handle.assert_clean();
}

#[tokio::test]
async fn instant_status_of_an_unknown_order_is_a_bank_error() {
    let handle = instant_bank(1, false);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let from = transfer(&m, &anna(), "Anna Beispiel", Instant::No).from;
    let err = jobs::instant_status(&mut dialog, &from, &instant_receipt(Some("NOPE"), "E2E-X"))
        .await
        .unwrap_err();
    assert!(matches!(err, FinTSError::Bank(_)), "{err:?}");
}

/// A bank with HKIPZ but without HKIPS (as Consorsbank's BPD): the
/// account's statement, searched for the payment's end-to-end ID.
fn bank_without_hkips(bookings: Vec<Booking>) -> BankHandle {
    let mut m = model(payments(None), false);
    m.accounts[0] = m.accounts[0]
        .clone()
        .with_bookings(Decimal::new(100, 0), bookings);
    TestBank::new(m).handle()
}

fn debit(purpose: &str) -> Booking {
    Booking::new(jiff::ZonedDateTime::now().date(), "-12.50", purpose)
}

#[tokio::test]
async fn instant_status_without_hkips_reads_the_statement() {
    let handle = bank_without_hkips(vec![
        debit("EREF+E2E-OTHER SVWZ+Andere Zahlung"),
        debit("EREF+E2E-TEST-1 SVWZ+Testzahlung"),
    ]);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let from = transfer(&m, &anna(), "Anna Beispiel", Instant::No).from;
    let status = done_status(
        jobs::instant_status(
            &mut dialog,
            &from,
            &instant_receipt(Some("INST0001"), "E2E-TEST-1"),
        )
        .await
        .unwrap(),
    );
    assert_eq!(
        status,
        InstantStatus {
            status: Some(SepaOrderStatus::Executed),
            again_after: None,
            source: StatusSource::Statement,
        }
    );
    let requests = handle.requests();
    let codes: Vec<_> = requests.iter().flat_map(|r| r.codes()).collect();
    assert!(codes.contains(&"HKKAZ"), "{codes:?}");
    assert!(!codes.contains(&"HKIPS"), "{codes:?}");
    dialog.end().await.unwrap();
    handle.assert_clean();
}

#[tokio::test]
async fn instant_status_from_a_statement_without_the_payment_is_unknown() {
    let handle = bank_without_hkips(vec![debit("EREF+E2E-OTHER SVWZ+Andere Zahlung")]);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let from = transfer(&m, &anna(), "Anna Beispiel", Instant::No).from;
    let status = done_status(
        jobs::instant_status(&mut dialog, &from, &instant_receipt(None, "E2E-TEST-1"))
            .await
            .unwrap(),
    );
    assert_eq!(status.status, None);
    assert_eq!(status.source, StatusSource::Statement);
    dialog.end().await.unwrap();
}

/// HKIPS with a TAN (HIPINS `J`): the status comes with the TAN answer,
/// also across a freeze and thaw.
#[tokio::test]
async fn instant_status_with_a_tan_pauses_and_resumes() {
    let handle = instant_bank(2, true);
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let receipt = pending_instant(&mut dialog, &m).await;
    let from = transfer(&m, &anna(), "Anna Beispiel", Instant::No).from;

    // 3045 arrives with the TAN answer, without HIIPS.
    let step = jobs::instant_status(&mut dialog, &from, &receipt)
        .await
        .unwrap();
    let Step::NeedTan(_, resume) = step else {
        panic!("{step:?}")
    };
    let first = match dialog.submit_tan(resume, TAN).await.unwrap() {
        Step::Done(d) => d.value,
        other => panic!("{other:?}"),
    };
    assert_eq!(first.status, None);
    assert!(first.again_after.is_some());

    // The second query pauses, is frozen into JSON and thawed.
    let step = jobs::instant_status(&mut dialog, &from, &receipt)
        .await
        .unwrap();
    assert!(matches!(step, Step::NeedTan(..)));
    let (mut dialog, paused) = round_trip(dialog, &handle);
    assert!(paused.challenge().is_some());
    let Some(AnyResume::InstantStatus(resume)) = paused.pending() else {
        panic!("instant status ticket expected")
    };
    let second = match dialog.submit_tan(resume, TAN).await.unwrap() {
        Step::Done(d) => d.value,
        other => panic!("{other:?}"),
    };
    assert_eq!(second.status, Some(SepaOrderStatus::Executed));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

// ── Capabilities ────────────────────────────────────────────────────────

/// Opens a dialog on a profile, answering a login TAN when it asks for one.
async fn open_profile(handle: &BankHandle) -> Dialog {
    let m = handle.model();
    let state = SessionState::new(
        &fints_client::Blz::new(&m.blz),
        &UserId::new(&m.customer.user_id),
    );
    let (mut dialog, mut step) = jobs::open(&client(handle), &creds(&m), &state)
        .await
        .unwrap();
    loop {
        step = match step {
            Step::Done(_) => return dialog,
            Step::NeedTan(c, r) => match c.kind {
                jobs::TanKind::Typed { .. } => dialog.submit_tan(r, TAN).await.unwrap(),
                jobs::TanKind::Decoupled { .. } => dialog.poll(r).await.unwrap(),
            },
            other => panic!("{other:?}"),
        };
    }
}

fn available(caps: &jobs::AccountCapabilities, job: JobKind) -> (&'static str, u16) {
    match &caps.job(job).unwrap().support {
        Support::Available { code, version, .. } => (code, *version),
        other => panic!("{job:?}: {other:?}"),
    }
}

#[tokio::test]
async fn capabilities_report_payments_per_profile() {
    let required = |c: &jobs::AccountCapabilities| -> Vec<String> {
        c.payee_check
            .as_ref()
            .expect("payee check")
            .required_for
            .iter()
            .map(|s| s.as_str().to_string())
            .collect()
    };

    let handle = TestBank::new(profiles::dkb()).handle();
    let dialog = open_profile(&handle).await;
    let caps = dialog.capabilities(&[]);
    let giro = &caps.accounts[0];
    assert_eq!(available(giro, JobKind::Transfer), ("HKCCS", 1));
    assert_eq!(available(giro, JobKind::InstantTransfer), ("HKIPZ", 2));
    assert_eq!(available(giro, JobKind::InstantStatus), ("HKIPS", 1));
    assert_eq!(
        giro.job(JobKind::Transfer).unwrap().tan_required,
        Some(true)
    );
    assert_eq!(
        giro.job(JobKind::InstantStatus).unwrap().tan_required,
        Some(false)
    );
    let r = required(giro);
    assert!(r.contains(&"HKCCS".to_string()) && r.contains(&"HKIPZ".to_string()));

    let handle = TestBank::new(profiles::comdirect()).handle();
    let dialog = open_profile(&handle).await;
    let caps = dialog.capabilities(&[]);
    let giro = &caps.accounts[0];
    assert_eq!(available(giro, JobKind::Transfer), ("HKCCS", 1));
    assert_eq!(available(giro, JobKind::InstantTransfer), ("HKIPZ", 1));
    assert!(required(giro).contains(&"HKCCS".to_string()));

    let handle = TestBank::new(profiles::consorsbank()).handle();
    let dialog = open_profile(&handle).await;
    let caps = dialog.capabilities(&[]);
    let giro = &caps.accounts[0];
    assert_eq!(giro.payee_check, None);
    assert_eq!(available(giro, JobKind::Transfer), ("HKCCS", 1));
    assert_eq!(available(giro, JobKind::InstantTransfer), ("HKIPZ", 1));
    assert_eq!(
        giro.job(JobKind::InstantStatus).unwrap().support,
        Support::NotOffered
    );
}

// ── Live payment behaviour per profile (2026-11-18) ─────────────────────

/// A transfer on a profile to its registered payee "Anna Beispiel" with a
/// mistyped name (a close match). Returns the receipt, the step names and
/// the notice codes of `Done` with their scope.
async fn profile_transfer(
    handle: &BankHandle,
    name: &str,
) -> (TransferReceipt, Vec<&'static str>, Vec<(String, String)>) {
    let m = handle.model();
    let iban = iban_de(&m.blz, "0000000501");
    let mut dialog = open_profile(handle).await;
    let mut step = jobs::transfer(&mut dialog, &transfer(&m, &iban, name, Instant::No))
        .await
        .unwrap();
    let mut seen = Vec::new();
    let done = loop {
        step = match step {
            Step::Done(d) => {
                seen.push("Done");
                break d;
            }
            Step::NeedTan(c, r) => {
                seen.push("NeedTan");
                match c.kind {
                    jobs::TanKind::Typed { .. } => dialog.submit_tan(r, TAN).await.unwrap(),
                    jobs::TanKind::Decoupled { .. } => dialog.poll(r).await.unwrap(),
                }
            }
            Step::NeedVopConfirmation(_, r) => {
                seen.push("NeedVopConfirmation");
                dialog.confirm_vop(r).await.unwrap()
            }
            Step::VopPending(_, r) => {
                seen.push("VopPending");
                dialog.poll(r).await.unwrap()
            }
        };
    };
    dialog.end().await.unwrap();
    handle.assert_clean();
    let notices = done
        .notices
        .iter()
        .filter_map(|n| {
            n.code()
                .map(|c| (c.code.to_string(), format!("{:?}", c.scope)))
        })
        .collect();
    (done.value, seen, notices)
}

/// DKB: HKVPP answered 3090, HKTAN 3945 (close match); then HKVPA with
/// HKCCS and the decoupled 3955; the poll gives the receipt. 3060 is
/// reported once although both submissions carried it.
#[tokio::test]
async fn dkb_close_match_confirms_then_decoupled_tan() {
    let handle = TestBank::new(profiles::dkb()).handle();
    let (receipt, seen, notices) = profile_transfer(&handle, "Anna Beispel").await;
    assert_eq!(
        seen,
        ["NeedVopConfirmation", "NeedTan", "NeedTan", "Done"],
        "{seen:?}"
    );
    assert_eq!(receipt.status, TransferStatus::Executed);
    let requests = payment_requests(&handle);
    let first = requests.iter().position(|c| c[0] == "HKVPP").unwrap();
    assert_eq!(
        requests[first..first + 2],
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPA", "HKCCS", "HKTAN"])
        ]
    );
    let n3060 = notices.iter().filter(|(c, _)| c == "3060").count();
    assert_eq!(n3060, 1, "{notices:?}");
    assert_one_payment(&handle, &iban_de("12030000", "0000000501"));
}

/// comdirect: HIRMG 3060 and 3905 on the check; the order after HKVPA runs
/// without a TAN (3076): no `NeedTan`.
#[tokio::test]
async fn comdirect_close_match_executes_without_tan() {
    let handle = TestBank::new(profiles::comdirect()).handle();
    let (receipt, seen, notices) = profile_transfer(&handle, "Anna Beispel").await;
    assert_eq!(seen, ["NeedVopConfirmation", "Done"], "{seen:?}");
    assert_eq!(receipt.status, TransferStatus::Executed);
    assert_eq!(
        payment_requests(&handle),
        [
            codes(&["HKVPP", "HKCCS", "HKTAN"]),
            codes(&["HKVPA", "HKCCS", "HKTAN"])
        ]
    );
    assert!(notices.iter().any(|(c, _)| c == "3905"), "{notices:?}");
    // Live 2026-11-18: HKVPA gets 0020, the order itself 0010 (not 0020).
    // The client reports no per-order success codes, so the model's setting
    // is what is asserted here.
    let vop = handle.model().payments.unwrap().vop.unwrap();
    assert_eq!(vop.vpa_answer.map(|a| a.0), Some("0020"));
    assert_eq!(vop.order_answer_after_vpa.map(|a| a.0), Some("0010"));
    assert_one_payment(&handle, &iban_de("20041155", "0000000501"));
}

/// Consorsbank: no payee check; the TAN codes 0030 come on HKCCS, not on
/// HKTAN; the client still sees the challenge.
#[tokio::test]
async fn consorsbank_tan_codes_on_the_order_are_a_challenge() {
    let handle = TestBank::new(profiles::consorsbank()).handle();
    let (receipt, seen, _) = profile_transfer(&handle, "Anna Beispiel").await;
    assert_eq!(seen, ["NeedTan", "NeedTan", "Done"], "{seen:?}");
    assert_eq!(receipt.status, TransferStatus::Executed);
    let requests = payment_requests(&handle);
    assert!(
        requests.contains(&codes(&["HKCCS", "HKTAN"])),
        "{requests:?}"
    );
    assert!(!requests.iter().any(|c| c.contains(&"HKVPP".to_string())));
    assert_one_payment(&handle, &iban_de("76030080", "0000000501"));
}

/// DKB, no match (RVNM, 3090 + 3945): the user cancels at the confirmation
/// and nothing is executed.
#[tokio::test]
async fn dkb_no_match_cancelled_at_the_confirmation_executes_nothing() {
    let handle = TestBank::new(profiles::dkb()).handle();
    let m = handle.model();
    let iban = iban_de(&m.blz, "0000000999");
    let mut dialog = open_profile(&handle).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &iban, "Zzz Unbekannt", Instant::No),
    )
    .await
    .unwrap();
    let Step::NeedVopConfirmation(c, _resume) = step else {
        panic!("confirmation expected: {step:?}")
    };
    assert_eq!(c.result.payees[0].outcome, PayeeOutcome::NoMatch);
    dialog.end().await.unwrap();
    assert!(handle.payments().is_empty());
    handle.assert_clean();
}

/// The pinned clock reaches the HNSHK stamp and the pain.001 `CreDtTm`.
#[tokio::test]
async fn pinned_clock_stamps_signature_and_creation_time() {
    let handle = bank(payments(None), false);
    let m = handle.model();
    let pinned = client(&handle).with_now(jiff::civil::datetime(2031, 1, 15, 13, 45, 1, 0));
    let mut dialog = open_with(&pinned, &m).await;
    let step = jobs::transfer(
        &mut dialog,
        &transfer(&m, &anna(), "Anna Beispiel", Instant::No),
    )
    .await
    .unwrap();
    finish(&mut dialog, step, &mut Vec::new()).await;
    let requests = handle.requests();
    assert!(
        requests
            .iter()
            .all(|r| r.text().contains(":20310115:134501+")),
        "every HNSHK carries the pinned stamp"
    );
    let order = requests.iter().find(|r| r.has("HKCCS")).unwrap();
    assert!(
        order
            .text()
            .contains("<CreDtTm>2031-01-15T13:45:01</CreDtTm>"),
        "{}",
        order.text()
    );
}
