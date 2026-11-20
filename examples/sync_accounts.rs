//! A whole run against one bank: log in (with an app confirmation or a
//! typed TAN), read balance and transactions of every account the UPD
//! lists, and keep the session state for the next run.
//!
//! The bank is chosen by bank code; access data and the registered
//! product ID come from the environment. Arguments: the state file
//! (default `fints-state.json`) and how many days of transactions
//! (default 30).
//!
//! ```text
//! FINTS_BLZ=<bank code> FINTS_USER=<login> FINTS_PIN=<PIN> \
//! FINTS_PRODUCT_ID=<product ID> cargo run --example sync_accounts -- state.json 30
//! ```
//!
//! Without a state file the example first synchronises (`jobs::synchronise`,
//! no login), prints the allowed TAN methods and uses the one
//! `FINTS_TAN_METHOD` names, if set.
//!
//! The state file is written in plain JSON here for simplicity. A real
//! application should store it encrypted: it identifies you to the bank.
//!
//! Optional: freeze at a TAN prompt. With `FINTS_FREEZE_FILE=paused.json` set,
//! the example does not wait for the TAN. It freezes the dialog
//! (`Dialog::freeze`) into that file and exits. Run it again with the same
//! variable: it reads the file, thaws the dialog (`Dialog::thaw`), continues
//! with `poll` (decoupled) or `submit_tan` (typed) according to the
//! `AnyResume` variant and deletes the file. A resumed login goes on to
//! sync the accounts; a resumed order only finishes that order and ends the
//! dialog. The bank may have dropped the dialog meanwhile
//! (`PausedDialog::expires_at` is only a hint); then the run fails with the
//! bank's error. The file holds user and system ID: store it like the state.
//! Without the variable nothing changes.

use std::time::Duration;

use fints_client::jobs::{
    self, AnyResume, BankClient, Credentials, Dialog, JobValue, PausedDialog, Range, Resume, Step,
    TanChallenge, TanKind,
};
use fints_client::{Pin, ProductId, SecurityFunction, SessionState, UserId};
use jiff::Timestamp;

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("set {name}"))
}

/// Answers the TANs of a step until the job is done. The library never
/// waits or reads input: the caller sleeps for the bank's interval and
/// asks the user, and decides when to give up (here: after 60 polls).
/// With `freeze` it stops at the first TAN prompt and returns `None`; the
/// caller then freezes the dialog.
async fn complete<T: JobValue>(
    dialog: &mut Dialog,
    mut step: Step<T>,
    freeze: bool,
) -> Result<Option<T>, Box<dyn std::error::Error>> {
    loop {
        step = match step {
            Step::Done(done) => return Ok(Some(done.value)),
            Step::NeedTan(..) if freeze => return Ok(None),
            Step::NeedTan(challenge, resume) => match challenge.kind {
                TanKind::Typed { .. } => {
                    if let Some(r) = &challenge.rejected {
                        println!("The bank rejected the TAN: {} {}", r.code.code, r.code.text);
                    }
                    println!("{}", challenge.challenge);
                    let mut tan = String::new();
                    std::io::stdin().read_line(&mut tan)?;
                    dialog.submit_tan(resume, tan.trim()).await?
                }
                TanKind::Decoupled {
                    polls_done,
                    auto_polling_allowed,
                    ..
                } => {
                    if polls_done == 0 {
                        println!("Confirm in your banking app: {}", challenge.challenge);
                    } else if polls_done >= 60 {
                        // A read: nothing is left at the bank.
                        let _ = dialog.cancel(resume)?;
                        return Err("no confirmation".into());
                    }
                    // Sleep until the bank's wait is over, not for a fixed time.
                    let wait = challenge
                        .not_before
                        .map(|at| {
                            Duration::try_from(at.duration_since(Timestamp::now()))
                                .unwrap_or_default()
                        })
                        .unwrap_or(Duration::from_secs(5));
                    tokio::time::sleep(wait).await;
                    if auto_polling_allowed == Some(false) {
                        // The bank wants the user to trigger each status request.
                        println!("Confirmed in the app? Press Enter to ask the bank.");
                        std::io::stdin().read_line(&mut String::new())?;
                    }
                    dialog.poll(resume).await?
                }
            },
            // Only a transfer pauses on its payee check; this example sends none.
            Step::NeedVopConfirmation(_, resume) | Step::VopPending(_, resume) => {
                let _ = dialog.cancel(resume)?;
                return Err("unexpected payee check".into());
            }
        };
    }
}

/// Continues a thawed dialog's pending job: `poll` for a decoupled
/// challenge, `submit_tan` for a typed one.
async fn resume_job<T: JobValue>(
    dialog: &mut Dialog,
    challenge: &TanChallenge,
    resume: Resume<T>,
    freeze: bool,
) -> Result<Option<T>, Box<dyn std::error::Error>> {
    let step = match challenge.kind {
        TanKind::Decoupled { .. } => dialog.poll(resume).await?,
        _ => {
            println!("{}", challenge.challenge);
            let mut tan = String::new();
            std::io::stdin().read_line(&mut tan)?;
            dialog.submit_tan(resume, tan.trim()).await?
        }
    };
    complete(dialog, step, freeze).await
}

/// Freezes the dialog into `path`. The example writes the file, not the library.
fn save_frozen(dialog: Dialog, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let paused = dialog.freeze()?;
    std::fs::write(path, paused.to_json()?)?;
    println!("Dialog frozen to {path}; confirm, then run again");
    Ok(())
}

/// Fetches balance and transactions of every account. `Ok(false)`: a TAN
/// prompt froze the run (only with `freeze`) and the dialog must be frozen.
async fn sync_accounts(
    dialog: &mut Dialog,
    opened: &jobs::Opened,
    days: i64,
    freeze: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    for acc in &opened.accounts {
        let label = acc.key.to_string();
        let account = match acc.account() {
            Ok(a) => a,
            Err(e) => {
                println!("{label}: skipped ({e})");
                continue;
            }
        };
        let today = jiff::ZonedDateTime::now().date();
        let range = Range::new(today - jiff::Span::new().days(days), today);
        let result = async {
            let step = jobs::balance(dialog, &account).await?;
            let Some(balance) = complete(dialog, step, freeze).await? else {
                return Ok::<_, Box<dyn std::error::Error>>(None);
            };
            let step = jobs::transactions(dialog, &account, range, Default::default()).await?;
            let Some(statement) = complete(dialog, step, freeze).await? else {
                return Ok(None);
            };
            Ok(Some((balance, statement)))
        }
        .await;
        match result {
            Ok(Some((balance, statement))) => println!(
                "{label} ({}): balance {:?}, {} transactions",
                acc.product_name.as_deref().unwrap_or("?"),
                balance.map(|b| format!("{} {}", b.amount, b.currency)),
                statement.booked.len(),
            ),
            Ok(None) => return Ok(false),
            Err(e) => println!("{label}: {e}"),
        }
    }
    Ok(true)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (blz, user, pin, product_id) = (
        env("FINTS_BLZ")?,
        env("FINTS_USER")?,
        env("FINTS_PIN")?,
        env("FINTS_PRODUCT_ID")?,
    );
    let creds = Credentials::new(UserId::new(user), Pin::new(pin));
    let product = ProductId::new(product_id);
    let args: Vec<String> = std::env::args().collect();
    let state_file = args
        .get(1)
        .map_or("fints-state.json", String::as_str)
        .to_owned();
    let days = match args.get(2) {
        Some(text) => text
            .parse::<i64>()
            .map_err(|_| format!("days: {text:?} is no number"))?,
        None => 30,
    };

    let config = fints_client::bank_by_blz(&blz).ok_or("BLZ is not in the bank directory")?;
    let client = BankClient::new(config.clone(), product);
    let state = match std::fs::read_to_string(&state_file) {
        Ok(json) => {
            println!("Using session state from {state_file}");
            SessionState::from_json(&json)?
        }
        Err(_) => {
            // No state yet: learn the allowed TAN methods without a login,
            // then log in with the one FINTS_TAN_METHOD names (e.g. 902),
            // else with the library's choice.
            let fresh = SessionState::new(&config.blz, &creds.user_id);
            let synced = jobs::synchronise(&client, &creds, &fresh).await?;
            for notice in &synced.notices {
                println!("Bank notice: {notice}");
            }
            let state = synced.value;
            for m in state.tan_methods() {
                if state
                    .allowed_security_functions
                    .contains(&m.security_function)
                {
                    println!("TAN method {}: {}", m.security_function, m.name);
                }
            }
            match std::env::var("FINTS_TAN_METHOD") {
                Ok(sf) => state.with_tan_method(&SecurityFunction::new(sf)),
                Err(_) => state,
            }
        }
    };

    let freeze_file = std::env::var("FINTS_FREEZE_FILE").ok();
    let freeze = freeze_file.is_some();
    let frozen = freeze_file
        .as_deref()
        .and_then(|path| Some((path, std::fs::read_to_string(path).ok()?)));

    let (mut dialog, opened) = if let Some((path, json)) = frozen {
        let paused = PausedDialog::from_json(&json)?;
        let challenge = paused
            .challenge()
            .cloned()
            .ok_or("no TAN challenge pending")?;
        let pending = paused
            .pending()
            .ok_or("nothing pending in the frozen dialog")?;
        let mut dialog = Dialog::thaw(paused, &client, &creds)?;
        println!("Thawed dialog from {path}");
        let opened = match pending {
            AnyResume::Open(resume) => resume_job(&mut dialog, &challenge, resume, freeze).await?,
            // A resumed order: finish it, then end the dialog.
            other => {
                let done = match other {
                    AnyResume::Balance(r) => resume_job(&mut dialog, &challenge, r, freeze)
                        .await?
                        .is_some(),
                    AnyResume::Transactions(r) => resume_job(&mut dialog, &challenge, r, freeze)
                        .await?
                        .is_some(),
                    AnyResume::Holdings(r) => resume_job(&mut dialog, &challenge, r, freeze)
                        .await?
                        .is_some(),
                    AnyResume::DepotTransactions(r) => {
                        resume_job(&mut dialog, &challenge, r, freeze)
                            .await?
                            .is_some()
                    }
                    AnyResume::Open(_) => unreachable!(),
                    AnyResume::Transfer(_) => return Err("a frozen transfer".into()),
                    AnyResume::InstantStatus(_) => return Err("a frozen instant status".into()),
                };
                if !done {
                    return save_frozen(dialog, path);
                }
                std::fs::remove_file(path)?;
                println!("Resumed order finished");
                let state = dialog.end().await?;
                std::fs::write(&state_file, state.to_json()?)?;
                return Ok(());
            }
        };
        let Some(opened) = opened else {
            return save_frozen(dialog, path);
        };
        std::fs::remove_file(path)?;
        (dialog, opened)
    } else {
        let (mut dialog, step) = jobs::open(&client, &creds, &state).await?;
        match complete(&mut dialog, step, freeze).await? {
            Some(opened) => (dialog, opened),
            None => return save_frozen(dialog, freeze_file.as_deref().unwrap()),
        }
    };

    if !sync_accounts(&mut dialog, &opened, days, freeze).await? {
        return save_frozen(dialog, freeze_file.as_deref().unwrap());
    }

    let state = dialog.end().await?;
    if let Some(old) = &state.previous_user_id {
        println!(
            "Bank changed the user ID from {old} to {} — update your credentials",
            state.user_id
        );
    }
    std::fs::write(&state_file, state.to_json()?)?;
    println!("Session state saved to {state_file}");
    Ok(())
}
