//! A wrong typed TAN (docs/jobs-step-design.md §2, return-codes-design §1):
//! where the bank answers 9941 alone and keeps the order, `submit_tan`
//! returns `NeedTan` again for the same task with `rejected` set; where
//! it deletes the order (9210) or ends the dialog (9800), the job fails
//! with a typed error and a transfer is not executed. The test bank
//! models both ([`WrongTanModel`]). All data is synthetic.

use std::sync::Arc;

use fints_client::codec::defs::HkTan6;
use fints_client::jobs::{
    self, AnyResume, BankClient, Credentials, Dialog, PausedDialog, Phase, Resume, Step,
    TanChallenge,
};
use fints_client::{
    Account, BankConfig, Category, FinTSError, Instant, Pin, ProductId, SessionState, Transfer,
    UserId,
};
use fints_client_testbank::{
    BankHandle, BankModel, LoginSca, OnMatch, Payee, PaymentsModel, TanRule, TestBank, VopModel,
    WrongTanEnd, WrongTanModel, iban_de,
};
use rust_decimal::Decimal;

const TAN: &str = "123456";
const WRONG: &str = "999999";

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

fn anna() -> String {
    iban_de("87654321", "0000000042")
}

/// HKSAL needs a typed TAN; payments with a payee check whose clean match
/// sends HKVPA with the TAN.
fn model(wrong_tan: WrongTanModel) -> BankModel {
    let mut m = BankModel {
        hispas_national: Some(false),
        wrong_tan,
        ..BankModel::default()
    };
    let job = m.job_mut("HKSAL");
    job.hipins_needs_tan = true;
    job.server_tan = TanRule::Required;
    m.enable_payments(PaymentsModel {
        payees: vec![Payee::new(&anna(), "Anna Beispiel")],
        vop: Some(VopModel {
            hkvpa_on_match: OnMatch::Require,
            ..VopModel::default()
        }),
        ..PaymentsModel::default()
    });
    m
}

fn retry(kept: u32, announce_attempts: bool) -> WrongTanModel {
    WrongTanModel {
        kept,
        announce_attempts,
        end: WrongTanEnd::OrderDeleted,
    }
}

fn transfer(m: &BankModel) -> Transfer {
    Transfer {
        from: account(m),
        debtor_name: None,
        payee_name: "Anna Beispiel".into(),
        payee_iban: anna(),
        payee_bic: None,
        amount: Decimal::new(1250, 2),
        purpose: "Testzahlung".into(),
        end_to_end_id: Some("E2E-TEST-1".into()),
        instant: Instant::No,
    }
}

async fn open(handle: &BankHandle) -> Dialog {
    let m = handle.model();
    let (dialog, step) = jobs::open(&client(handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    assert!(matches!(step, Step::Done(_)), "login is exempt");
    dialog
}

fn need_tan<T: std::fmt::Debug>(step: Step<T>) -> (TanChallenge, Resume<T>) {
    match step {
        Step::NeedTan(c, r) => (c, r),
        other => panic!("NeedTan expected, got {other:?}"),
    }
}

fn done<T: std::fmt::Debug>(step: Step<T>) -> jobs::Completed<T> {
    match step {
        Step::Done(d) => d,
        other => panic!("Done expected, got {other:?}"),
    }
}

/// The task references of the TAN messages (HKTAN process 2) sent from
/// request `from` on.
fn tan_messages(handle: &BankHandle, from: usize) -> Vec<String> {
    handle.requests()[from..]
        .iter()
        .filter_map(|r| r.get::<HkTan6>())
        .filter(|t| t.process.as_str() == "2")
        .map(|t| t.order_reference.map(|r| r.as_str().to_string()).unwrap())
        .collect()
}

/// Read job: a wrong TAN gives the same challenge again with the bank's
/// 9941; the right one then gives the balance. Both TANs go to the same
/// task, nothing else is sent.
#[tokio::test]
async fn wrong_then_right_tan_on_a_read() {
    let handle = TestBank::new(model(retry(2, false))).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let before = handle.requests().len();
    let (first, resume) = need_tan(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    assert!(first.rejected.is_none());

    let (again, resume) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());
    let rejected = again.rejected.as_ref().expect("rejected is set");
    assert_eq!(rejected.code.code.as_str(), "9941");
    assert_eq!(rejected.code.text, "TAN ungültig");
    assert_eq!(rejected.attempts_left, None, "the bank gave no count");
    assert_eq!(rejected.count, 1);
    assert_eq!(
        again.challenge, first.challenge,
        "same order, same challenge"
    );
    assert!(matches!(dialog.phase(), Phase::Paused(_)));

    let balance = done(dialog.submit_tan(resume, TAN).await.unwrap()).value;
    assert!(balance.is_some());
    let tasks = tan_messages(&handle, before);
    assert_eq!(tasks.len(), 2, "two TAN messages");
    assert_eq!(tasks[0], tasks[1], "for the same task");
    assert_eq!(handle.requests().len(), before + 3, "HKSAL, two TANs");
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// 9360 next to 9941 gives the attempts left; the count goes up.
#[tokio::test]
async fn remaining_attempts_come_from_9360() {
    let handle = TestBank::new(model(retry(2, true))).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let (_, resume) = need_tan(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    let (c, resume) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());
    let r = c.rejected.unwrap();
    assert_eq!((r.count, r.attempts_left), (1, Some(2)));
    let (c, resume) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());
    let r = c.rejected.unwrap();
    assert_eq!((r.count, r.attempts_left), (2, Some(1)));
    done(dialog.submit_tan(resume, TAN).await.unwrap());
    handle.assert_clean();
}

/// Transfer: a wrong TAN, then the right one. HKVPA goes with the first
/// TAN message only; the payment is executed exactly once, and the payee
/// check result stays on the challenge.
#[tokio::test]
async fn wrong_then_right_tan_on_a_transfer_executes_once() {
    let handle = TestBank::new(model(retry(2, false))).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let (first, resume) = need_tan(jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap());
    let before = handle.requests().len();
    let (again, resume) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());
    assert!(again.rejected.is_some());
    assert_eq!(again.payee_check, first.payee_check);
    assert!(handle.payments().is_empty(), "nothing executed yet");

    let receipt = done(dialog.submit_tan(resume, TAN).await.unwrap()).value;
    let payments = handle.payments();
    assert_eq!(payments.len(), 1, "executed exactly once");
    assert_eq!(payments[0].msg_id, receipt.message_id);
    let requests = handle.requests();
    assert!(requests[before].has("HKVPA"), "HKVPA with the first TAN");
    assert!(!requests[before + 1].has("HKVPA"), "not with the second");
    assert!(
        requests[before..].iter().all(|r| !r.has("HKCCS")),
        "the order is never resent"
    );
    assert_eq!(tan_messages(&handle, before).len(), 2);
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Wrong TANs until the bank deletes the order (9941 + 9210): a class 9
/// bank error, the transfer not executed, the dialog open for the next
/// job, nothing resent.
#[tokio::test]
async fn wrong_tans_until_the_bank_deletes_the_transfer() {
    let handle = TestBank::new(model(retry(2, false))).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let (_, mut resume) = need_tan(jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap());
    for count in 1..=2 {
        let (c, r) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());
        assert_eq!(c.rejected.unwrap().count, count);
        resume = r;
    }
    let before = handle.requests().len();
    let err = dialog.submit_tan(resume, WRONG).await.unwrap_err();
    let FinTSError::Bank(bank) = &err else {
        panic!("a class 9 rejection, not an unknown outcome: {err:?}")
    };
    assert_eq!(bank.code().code.as_str(), "9941");
    assert!(bank.codes().iter().any(|c| c.code.as_str() == "9210"));
    assert_eq!(err.kind(), Category::Tan);
    assert!(handle.payments().is_empty(), "not executed");
    assert!(matches!(dialog.phase(), Phase::Open));
    assert_eq!(handle.requests().len(), before + 1, "nothing resent");
    // The dialog takes the next job.
    let (_, resume) = need_tan(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    done(dialog.submit_tan(resume, TAN).await.unwrap());
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// A bank that ends the dialog on a wrong TAN (9941 + 9800): a class 9
/// bank error, the transfer not executed, the dialog ended.
#[tokio::test]
async fn wrong_tan_where_the_bank_ends_the_dialog() {
    let handle = TestBank::new(model(WrongTanModel {
        kept: 0,
        announce_attempts: false,
        end: WrongTanEnd::DialogEnded,
    }))
    .handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let (_, resume) = need_tan(jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap());
    let err = dialog.submit_tan(resume, WRONG).await.unwrap_err();
    assert!(matches!(err, FinTSError::Bank(_)), "{err:?}");
    assert!(handle.payments().is_empty(), "not executed");
    assert!(matches!(dialog.phase(), Phase::Ended));
    let err = jobs::balance(&mut dialog, &account(&m)).await.unwrap_err();
    assert!(matches!(err, FinTSError::DialogEnded), "{err:?}");
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Wrong TANs on a read until the bank refuses: a TAN error, the dialog
/// open.
#[tokio::test]
async fn wrong_tans_until_the_bank_refuses_a_read() {
    let handle = TestBank::new(model(retry(1, false))).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let (_, resume) = need_tan(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    let (_, resume) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());
    let err = dialog.submit_tan(resume, WRONG).await.unwrap_err();
    assert_eq!(err.kind(), Category::Tan, "{err}");
    assert!(err.needs_user());
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Login: a wrong TAN, then the right one opens the dialog.
#[tokio::test]
async fn wrong_then_right_tan_on_the_login() {
    let mut m = model(retry(2, false));
    m.login_sca = LoginSca::Tan;
    let handle = TestBank::new(m).handle();
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let (_, resume) = need_tan(step);
    let (c, resume) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());
    assert!(c.rejected.is_some());
    let opened = done(dialog.submit_tan(resume, TAN).await.unwrap()).value;
    assert_eq!(opened.accounts.len(), 1);
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Freeze and thaw between the wrong TAN and the retry: the rejection
/// survives, and the right TAN executes the transfer once.
#[tokio::test]
async fn wrong_tan_survives_freeze_and_thaw() {
    let handle = TestBank::new(model(retry(2, true))).handle();
    let m = handle.model();
    let mut dialog = open(&handle).await;
    let (_, resume) = need_tan(jobs::transfer(&mut dialog, &transfer(&m)).await.unwrap());
    let (again, _) = need_tan(dialog.submit_tan(resume, WRONG).await.unwrap());

    let json = dialog.freeze().unwrap().to_json().unwrap();
    let paused = PausedDialog::from_json(&json).unwrap();
    let Some(AnyResume::Transfer(resume)) = paused.pending() else {
        panic!("a transfer waits")
    };
    let mut dialog = Dialog::thaw(paused, &client(&handle), &creds(&m)).unwrap();
    let challenge = dialog.pending().and_then(|p| p.challenge()).cloned();
    assert_eq!(
        challenge.as_ref(),
        Some(&again),
        "challenge and rejection kept"
    );
    assert_eq!(again.rejected.unwrap().attempts_left, Some(2));

    let receipt = done(dialog.submit_tan(resume, TAN).await.unwrap()).value;
    assert_eq!(handle.payments().len(), 1, "executed exactly once");
    assert_eq!(handle.payments()[0].msg_id, receipt.message_id);
    dialog.end().await.unwrap();
    handle.assert_clean();
}
