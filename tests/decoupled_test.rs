//! TAN handling through `jobs`: decoupled (pushTAN) polling and TAN entry.
//!
//! The polling tests need an exact sequence of answers (3955 → 3956 … →
//! 0020 and the failure modes), so a [`Script`] serves canned messages in
//! order. TAN entry runs against the synthetic test bank. The library
//! never waits: every `poll` here is one request, called at once.

use fints_client::jobs::{self, BankClient, Credentials, Opened, Phase, Step, TanKind};
use fints_client::{BankConfig, FinTSError, Pin, ProductId, SessionState, UserId};
use fints_client_testbank::{
    BankModel, HttpBank, HttpScript, LoginSca, Script, TestBank, WrongTanModel,
};

/// The bank's answer around `segments`: header (dialog `DLG-1`) and trailer.
fn msg(segments: &str) -> String {
    let header = "HNHBK:1:3+000000000000+300+DLG-1+1'";
    let trailer = "HNHBS:9:1+1'";
    [header, segments, trailer].concat()
}

/// The task reference the challenge hands out; every poll repeats it.
const TASK: &str = "AUF-0815";

const INIT_PUSH_SENT: &str = "HIRMG:2:2+0010::Auftrag entgegengenommen'\
    HIRMS:3:2:4+3955::Freigabe in der Test-App erforderlich'\
    HITAN:4:7:4+4++AUF-0815+Bitte den Login in der Test-App freigeben'";
const PENDING: &str = "HIRMG:2:2+0010::ok'HIRMS:3:2:3+3956::Noch nicht freigegeben'";
const AWAIT_PUSH: &str =
    "HIRMG:2:2+0010::ok'HIRMS:3:2:3+3957::Push-Nachricht noch nicht bestaetigt'";
const CONFIRMED: &str = "HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::Freigabe erteilt'";
const EXPIRED: &str =
    "HIRMG:2:2+9050::Nachricht teilweise fehlerhaft'HIRMS:3:2:3+9999::Freigabefrist abgelaufen'";

/// Serves `answers` one per request; the last one repeats.
async fn scripted_bank(answers: Vec<&'static str>) -> HttpScript {
    let messages = answers.into_iter().map(|a| msg(a).into_bytes()).collect();
    Script::new(messages).serve().await
}

// A saved state whose BPD offers one TAN method (HITANS 5), so
// `jobs::open` needs no synchronisation and the first scripted answer is
// the decoupled challenge.
const BPD_ONE_METHOD: &str = "HIBPA:6:3:4+12+280:12345678+Testbank+1+1+300+500'\
    HIKAZS:10:7:4+1+1+1+90:J:N'HISALS:11:7:4+1+1+1'\
    HITANS:12:5:4+1+1+1+N:N:0:944:2:TSMS:mobileTAN::SMS-TAN Test:6:1:TAN:3:1:J:1:0:N:0:2:N:J:00:1:1'\
    HIPINS:13:1:4+1+1+1+5:20:6:Kennung::HKKAZ:N:HKSAL:N:HKTAN:N'";
const UPD_ONE_ACCOUNT: &str = "HIUPA:14:4:4+kunde7+4+0'\
    HIUPD:15:6:4+4711::280:12345678+DE00123456780000004711+kunde7+1+EUR+Erika Muster++Girokonto++HKSAL:1'";

fn saved_state() -> SessionState {
    let json = serde_json::json!({
        "format": "fints-client-session/2",
        "blz": "12345678",
        "user_id": "kunde7",
        "system_id": "SYS-0042",
        "bpd_version": 12,
        "upd_version": 4,
        "bpd": BPD_ONE_METHOD,
        "upd": UPD_ONE_ACCOUNT,
        "tan_method": "944",
        "tan_medium": null,
        "allowed_security_functions": ["944"],
        "previous_user_id": null,
        "tan_required_learned": [],
        "updated_at": "2026-10-15T00:00:00Z",
    });
    SessionState::from_json(&json.to_string()).unwrap()
}

/// A login against the scripted bank with the saved state: the first
/// answer is the challenge.
async fn open_scripted(responses: Vec<&'static str>) -> (HttpScript, jobs::Dialog, Step<Opened>) {
    let script = scripted_bank(responses).await;
    let client = BankClient::new(
        BankConfig::new("Test Bank", "12345678", "TESTDEFFXXX", script.url()),
        ProductId::new("P"),
    );
    let creds = Credentials::new(UserId::new("kunde7"), Pin::new("12345"));
    let (dialog, step) = jobs::open(&client, &creds, &saved_state())
        .await
        .expect("open");
    (script, dialog, step)
}

fn expect_need_tan(step: Step<Opened>) -> (jobs::TanChallenge, jobs::Resume<Opened>) {
    match step {
        Step::NeedTan(challenge, resume) => (challenge, resume),
        other => panic!("expected NeedTan, got {other:?}"),
    }
}

fn polls_done(challenge: &jobs::TanChallenge) -> u32 {
    match challenge.kind {
        TanKind::Decoupled { polls_done, .. } => polls_done,
        _ => panic!("decoupled expected: {challenge:?}"),
    }
}

#[tokio::test]
async fn poll_until_confirmed_is_one_request_per_call() {
    let (bank, mut dialog, step) =
        open_scripted(vec![INIT_PUSH_SENT, PENDING, PENDING, CONFIRMED]).await;
    let (challenge, mut resume) = expect_need_tan(step);
    assert_eq!(polls_done(&challenge), 0);

    // Two 3956 answers: `NeedTan` again, `polls_done` counting up.
    for expected in [1, 2] {
        let (challenge, next) = expect_need_tan(dialog.poll(resume).await.expect("poll"));
        assert_eq!(polls_done(&challenge), expected);
        resume = next;
    }
    let Step::Done(done) = dialog.poll(resume).await.expect("confirm") else {
        panic!("0020 confirms");
    };
    assert_eq!(done.value.accounts.len(), 1);
    assert!(matches!(dialog.phase(), Phase::Open));

    // Every poll is one HKTAN with the task reference, and nothing else (this
    // BPD has HITANS 5, which polls with process 2; process S is in `jobs_test`).
    let reqs = bank.requests();
    assert_eq!(reqs.len(), 4);
    for poll in &reqs[1..] {
        assert!(
            poll.contains(&format!("HKTAN:3:5+2++++{TASK}++N'")),
            "poll was: {poll}"
        );
        assert!(!poll.contains("HKIDN"), "polls must not re-init: {poll}");
    }
}

/// 3957 ("wait for the push message") without 0020 is as pending as 3956.
#[tokio::test]
async fn await_push_3957_alone_counts_as_still_pending() {
    let (bank, mut dialog, step) =
        open_scripted(vec![INIT_PUSH_SENT, AWAIT_PUSH, AWAIT_PUSH, CONFIRMED]).await;
    let (_, mut resume) = expect_need_tan(step);
    for expected in [1, 2] {
        let (challenge, next) = expect_need_tan(dialog.poll(resume).await.expect("poll"));
        assert_eq!(polls_done(&challenge), expected);
        resume = next;
    }
    assert!(matches!(
        dialog.poll(resume).await.expect("confirm"),
        Step::Done(_)
    ));
    assert_eq!(bank.requests().len(), 4);
}

#[tokio::test]
async fn bank_error_while_pending_is_returned() {
    let (_bank, mut dialog, step) = open_scripted(vec![INIT_PUSH_SENT, PENDING, EXPIRED]).await;
    let (_, resume) = expect_need_tan(step);
    let (_, resume) = expect_need_tan(dialog.poll(resume).await.expect("pending"));
    match dialog.poll(resume).await.expect_err("expired") {
        FinTSError::Bank(e) => {
            let code = e.code();
            assert!(code.text.contains("abgelaufen") || code.text.contains("fehlerhaft"))
        }
        other => panic!("expected bank error, got {other:?}"),
    }
    // A failed login leaves nothing to resume.
    assert!(dialog.pending().is_none());
}

#[tokio::test]
async fn submit_tan_refuses_decoupled_challenges() {
    let (_bank, mut dialog, step) = open_scripted(vec![INIT_PUSH_SENT]).await;
    let (challenge, resume) = expect_need_tan(step);
    assert!(matches!(challenge.kind, TanKind::Decoupled { .. }));
    let err = dialog
        .submit_tan(resume, "123456")
        .await
        .expect_err("decoupled needs poll");
    assert!(matches!(err, FinTSError::Dialog(_)), "{err:?}");
    assert!(dialog.pending().is_some(), "the challenge is still pending");
}

// ── TAN entry (chipTAN / photoTAN / smsTAN) ───────────────────────────────

const TAN_USER: &str = "tanuser3";
const TAN_PIN: &str = "tan-pin-3";

/// A bank whose login needs a typed TAN (method 942, TAN 654321).
async fn typed_tan_bank() -> HttpBank {
    typed_tan_bank_with(WrongTanModel::default()).await
}

/// [`typed_tan_bank`] with this answer to a wrong TAN.
async fn typed_tan_bank_with(wrong_tan: WrongTanModel) -> HttpBank {
    let mut m = BankModel {
        wrong_tan,
        ..BankModel::default()
    };
    m.customer.user_id = TAN_USER.into();
    m.customer.pin = TAN_PIN.into();
    m.customer.customer_id = TAN_USER.into();
    m.tan_methods = vec![fints_client_testbank::TanMethod::typed(
        942,
        "mobileTAN",
        "654321",
    )];
    m.login_sca = LoginSca::Tan;
    TestBank::new(m).serve().await
}

async fn dialog_waiting_for_tan(
    bank: &HttpBank,
) -> (jobs::Dialog, jobs::TanChallenge, jobs::Resume<Opened>) {
    let client = bank.client(&ProductId::new("P"));
    let creds = Credentials::new(UserId::new(TAN_USER), Pin::new(TAN_PIN));
    let state = SessionState::new(
        &fints_client::Blz::new(&bank.model().blz),
        &UserId::new(TAN_USER),
    );
    let (dialog, step) = jobs::open(&client, &creds, &state).await.expect("open");
    let (challenge, resume) = expect_need_tan(step);
    assert!(
        matches!(challenge.kind, TanKind::Typed { .. }),
        "expected a TAN-entry challenge: {challenge:?}"
    );
    assert_eq!(challenge.challenge.as_str(), "Bitte geben Sie die TAN ein");
    (dialog, challenge, resume)
}

#[tokio::test]
async fn submit_tan_authenticates_the_dialog() {
    let bank = typed_tan_bank().await;
    let (mut dialog, _, resume) = dialog_waiting_for_tan(&bank).await;
    let Step::Done(done) = dialog
        .submit_tan(resume, "654321")
        .await
        .expect("TAN accepted")
    else {
        panic!("the TAN opens the dialog");
    };
    assert!(matches!(dialog.phase(), Phase::Open));

    let sent = bank.requests().last().cloned().unwrap();
    let text = sent.text();
    assert!(
        sent.has("HKTAN") && text.contains("TASK0001"),
        "process 2 for the task: {text}"
    );
    assert_eq!(
        (sent.pin.as_deref(), sent.tan.as_deref()),
        (Some(TAN_PIN), Some("654321")),
        "PIN and TAN go into HNSHA: {text}"
    );
    assert!(
        text.contains(&format!("{TAN_PIN}:654321")),
        "PIN and TAN go into HNSHA: {text}"
    );
    assert_eq!(done.value.accounts.len(), 1);
    dialog.end().await.unwrap();
    bank.assert_clean();
}

/// A wrong login TAN the bank does not keep the login for ends the dialog.
#[tokio::test]
async fn rejected_tan_ends_the_dialog() {
    let bank = typed_tan_bank_with(WrongTanModel {
        kept: 0,
        ..WrongTanModel::default()
    })
    .await;
    let (mut dialog, challenge, resume) = dialog_waiting_for_tan(&bank).await;
    assert!(dialog.submit_tan(resume, "000000").await.is_err());
    // The failed login dropped the job and ended the dialog.
    assert!(dialog.pending().is_none());
    assert!(matches!(dialog.phase(), Phase::Ended));
    let _ = challenge;
    bank.assert_clean();
}
