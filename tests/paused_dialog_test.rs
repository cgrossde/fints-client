//! Freeze and thaw (`fints-client-paused/4`) against the synthetic test
//! bank in-process: every dialog goes through JSON into a fresh
//! `BankClient` and continues with the same dialog ID and the next message
//! number (the test bank checks both). All data is synthetic.

use std::sync::Arc;

use fints_client::jobs::{
    self, AnyResume, BankClient, Cancelled, Credentials, Dialog, Format, PausedDialog, Phase,
    Range, Step, TanKind,
};
use fints_client::{Account, BankConfig, FinTSError, Pin, ProductId, SessionState, UserId};
use fints_client_testbank::{
    BankHandle, BankModel, Booking, LoginSca, TanRule, TestBank, date, profiles,
};

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

fn done<T: std::fmt::Debug>(step: Step<T>) -> jobs::Completed<T> {
    match step {
        Step::Done(d) => d,
        other => panic!("expected Done, got {other:?}"),
    }
}

/// Freeze, store as JSON, read back and thaw through a new client, as a
/// restarted process would. The next request continues the dialog: same
/// dialog ID, next message number.
fn round_trip(dialog: Dialog, handle: &BankHandle) -> (Dialog, Option<AnyResume>) {
    let last = handle
        .requests()
        .last()
        .cloned()
        .expect("a request was sent");
    let json = dialog.freeze().unwrap().to_json().unwrap();
    let m = handle.model();
    assert!(!json.contains(&m.customer.pin), "no PIN in {json}");
    let paused = PausedDialog::from_json(&json).unwrap();
    let resume = paused.pending();
    let dialog = Dialog::thaw(paused, &client(handle), &creds(&m)).unwrap();
    assert_eq!(
        handle.requests().len(),
        last.index + 1,
        "freeze and thaw send nothing"
    );
    (dialog, resume)
}

/// The request after a thaw continues the dialog the last one was in.
fn assert_continues(handle: &BankHandle, before: usize) {
    let requests = handle.requests();
    let (prev, next) = (&requests[before - 1], &requests[before]);
    // The initialisation goes out with dialog ID 0; the bank assigns one.
    if prev.dialog_id == "0" {
        assert_ne!(next.dialog_id, "0");
    } else {
        assert_eq!(next.dialog_id, prev.dialog_id);
    }
    assert_eq!(next.message_number, prev.message_number + 1);
}

fn decoupled_model(approve_after: u32) -> BankModel {
    let mut m = BankModel {
        decoupled_wait_secs: 7,
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

/// Decoupled login paused, frozen and thawed; polling goes on.
#[tokio::test]
async fn decoupled_login_survives_freeze_and_thaw() {
    let handle = TestBank::new(decoupled_model(2)).handle();
    let m = handle.model();
    let (dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::NeedTan(_, _) = step else {
        panic!("TAN expected")
    };
    let before_freeze = dialog
        .pending()
        .and_then(|p| p.challenge().and_then(|c| c.not_before));
    let (mut dialog, resume) = round_trip(dialog, &handle);
    assert!(matches!(dialog.phase(), Phase::Paused(_)));
    assert_eq!(
        dialog
            .pending()
            .and_then(|p| p.challenge().and_then(|c| c.not_before)),
        before_freeze
    );
    let Some(AnyResume::Open(resume)) = resume else {
        panic!("login ticket expected: {resume:?}")
    };

    let before = handle.requests().len();
    let Step::NeedTan(challenge, resume) = dialog.poll(resume).await.unwrap() else {
        panic!("still pending expected")
    };
    assert_continues(&handle, before);
    assert!(matches!(
        challenge.kind,
        TanKind::Decoupled { polls_done: 1, .. }
    ));

    // Frozen again, now after one poll.
    let (mut dialog, _) = round_trip(dialog, &handle);
    let opened = done(dialog.poll(resume).await.unwrap()).value;
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

/// A typed TAN on a balance, answered after a thaw.
#[tokio::test]
async fn typed_tan_on_balance_after_thaw() {
    let handle = TestBank::new(order_tan_model()).handle();
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let Step::NeedTan(challenge, _) = jobs::balance(&mut dialog, &account(&m)).await.unwrap()
    else {
        panic!("TAN expected")
    };
    let paused = dialog.freeze().unwrap();
    assert_eq!(
        paused.challenge(),
        Some(&challenge),
        "shown without thawing"
    );
    let paused = PausedDialog::from_json(&paused.to_json().unwrap()).unwrap();
    let Some(AnyResume::Balance(resume)) = paused.pending() else {
        panic!("balance ticket expected")
    };
    let mut dialog = Dialog::thaw(paused, &client(&handle), &creds(&m)).unwrap();

    let before = handle.requests().len();
    let balance = done(dialog.submit_tan(resume, "123456").await.unwrap());
    assert_continues(&handle, before);
    assert_eq!(balance.value.unwrap().amount.to_string(), "1654.33");
    assert!(matches!(dialog.phase(), Phase::Open));
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// A TAN on touchdown page 2: the pages so far and the touchdown point
/// survive the freeze.
#[tokio::test]
async fn tan_on_touchdown_page_two_keeps_the_pages() {
    let mut m = BankModel::default();
    m.accounts[0] = fints_client_testbank::Account::giro(&m.blz, "0001234567", "Testkundin Eins")
        .with_bookings(
            "500.00".parse().unwrap(),
            vec![
                Booking::new(date(2026, 3, 2), "-10.00", "SVWZ+Seite eins"),
                Booking::new(date(2026, 3, 3), "-20.00", "SVWZ+Seite zwei"),
            ],
        );
    m.job_mut("HKKAZ").page_size = Some(1);
    m.job_mut("HKKAZ").server_tan = TanRule::OnPage(2);
    let handle = TestBank::new(m).handle();
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let range = Range::new(date(2026, 1, 1), date(2026, 12, 31));
    let Step::NeedTan(_, _) = jobs::transactions(&mut dialog, &account(&m), range, Format::Auto)
        .await
        .unwrap()
    else {
        panic!("TAN expected on page 2")
    };
    let (mut dialog, resume) = round_trip(dialog, &handle);
    let Some(AnyResume::Transactions(resume)) = resume else {
        panic!("transactions ticket expected")
    };
    let before = handle.requests().len();
    let statement = done(dialog.submit_tan(resume, "123456").await.unwrap()).value;
    assert_continues(&handle, before);
    let amounts: Vec<String> = statement
        .booked
        .iter()
        .map(|t| t.amount.to_string())
        .collect();
    assert_eq!(
        amounts,
        ["-10.00", "-20.00"],
        "page 1 came before the freeze"
    );
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// An open dialog frozen between two jobs (a stateless server).
#[tokio::test]
async fn open_dialog_frozen_between_jobs() {
    let handle = TestBank::new(BankModel::default()).handle();
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    done(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    let (mut dialog, resume) = round_trip(dialog, &handle);
    assert!(resume.is_none());
    assert!(matches!(dialog.phase(), Phase::Open));
    let before = handle.requests().len();
    let balance = done(jobs::balance(&mut dialog, &account(&m)).await.unwrap());
    assert_continues(&handle, before);
    assert_eq!(balance.value.unwrap().amount.to_string(), "1654.33");
    let state = dialog.end().await.unwrap();
    assert_eq!(state.system_id.as_str(), "TESTSYS0001");
    handle.assert_clean();
}

/// A dialog-only UPD (version 0, as Consorsbank) is no part of any
/// `SessionState` but survives in the paused dialog.
#[tokio::test]
async fn dialog_only_upd_survives() {
    let handle = TestBank::new(profiles::consorsbank()).handle();
    let m = handle.model();
    let (dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let Step::Done(_) = step else {
        panic!("{step:?}")
    };
    assert!(dialog.bank_params().upd().unwrap().is_dialog_only());
    let (dialog, _) = round_trip(dialog, &handle);
    let upd = dialog.bank_params().upd().expect("UPD kept");
    assert!(upd.is_dialog_only());
    assert_eq!(
        dialog.bank_params().upd().map_or(0, |u| u.accounts().len()),
        m.accounts.len()
    );
    dialog.end().await.unwrap();
    handle.assert_clean();
}

/// Another bank, URL or user: refused before anything is sent. An ended
/// dialog cannot be frozen.
#[tokio::test]
async fn mismatch_and_ended_are_refused() {
    let handle = TestBank::new(BankModel {
        login_sca: LoginSca::Tan,
        ..BankModel::default()
    })
    .handle();
    let m = handle.model();
    let (dialog, _step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    let paused = dialog.freeze().unwrap();
    let sent = handle.requests().len();

    let other_user = Credentials::new(UserId::new("someone-else"), Pin::new(&m.customer.pin));
    let other_blz = BankConfig::new(
        m.name.clone(),
        "99999999",
        m.bic.clone(),
        "in-process://testbank",
    );
    let other_url = BankConfig::new(
        m.name.clone(),
        m.blz.clone(),
        m.bic.clone(),
        "in-process://other",
    );
    let with = |config| BankClient::new(config, ProductId::new("TEST-123"));
    for (client, creds, field) in [
        (client(&handle), other_user, "user_id"),
        (with(other_blz), creds(&m), "blz"),
        (with(other_url), creds(&m), "url"),
    ] {
        match Dialog::thaw(paused.clone(), &client, &creds) {
            Err(FinTSError::PausedDialogMismatch { field: f }) => assert_eq!(f, field),
            r => panic!("{field}: {r:?}"),
        }
    }
    assert_eq!(handle.requests().len(), sent, "nothing sent");

    let Some(AnyResume::Open(resume)) = paused.pending() else {
        panic!("login ticket expected")
    };
    let mut dialog = Dialog::thaw(paused, &client(&handle), &creds(&m)).unwrap();
    assert_eq!(dialog.cancel(resume).unwrap(), Cancelled::NothingSent);
    assert!(matches!(dialog.phase(), Phase::Ended));
    assert!(matches!(dialog.freeze(), Err(FinTSError::DialogEnded)));
}

/// `Debug` of a real frozen dialog shows no PIN, user ID, system ID or
/// IBAN.
#[tokio::test]
async fn debug_shows_no_secrets() {
    let handle = TestBank::new(order_tan_model()).handle();
    let m = handle.model();
    let (mut dialog, step) = jobs::open(&client(&handle), &creds(&m), &empty_state(&m))
        .await
        .unwrap();
    done(step);
    let Step::NeedTan(..) = jobs::balance(&mut dialog, &account(&m)).await.unwrap() else {
        panic!("TAN expected")
    };
    let out = format!("{:?}", dialog.freeze().unwrap());
    let iban = m.accounts[0].iban.clone().unwrap();
    for secret in [
        m.customer.pin.as_str(),
        m.customer.user_id.as_str(),
        "TESTSYS0001",
        iban.as_str(),
    ] {
        assert!(!out.contains(secret), "{secret} in {out}");
    }
}
