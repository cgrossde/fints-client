//! Sends one SEPA transfer at a real bank and records the dialog as a
//! masked trace. A human runs this, through `scripts/try-bank.sh <BLZ> --transfer`; it
//! contacts the bank and MOVES REAL MONEY. See `testbank/README.md`.
//!
//! ```text
//! FINTS_BLZ=12030000 FINTS_USER=... FINTS_PIN=... FINTS_PRODUCT_ID=... \
//!   FINTS_PAYEE_NAME=... FINTS_PAYEE_IBAN=... [FINTS_PAYEE_BIC=...] \
//!   [FINTS_AMOUNT=1.00] [FINTS_PURPOSE=...] [FINTS_INSTANT=no|prefer|require] \
//!   cargo run --features trace --example transfer -- testbank/fixtures/replay/local/dkb-12030000.transfer.trace.json
//! ```
//!
//! `FINTS_URL` overrides the bank's URL (e.g. the local `fints-testbank`).
//! An amount above 1.00 is refused unless `FINTS_ALLOW_AMOUNT` is set.
//!
//! The flow: login (with its TAN), the debtor account is picked from the
//! UPD accounts with an IBAN, then `jobs::transfer`. A payee check that
//! is not a clean match is shown (result per payee, the bank's explanation,
//! validity) and asks `confirm? [y/N]`; a running check is polled after the
//! bank's wait; a TAN is typed (hidden) or approved in the app (press
//! Enter). The receipt is printed. The trace is written in every outcome,
//! like `record_trace`: masked by the library (payment orders: see
//! `fints_client::mask`), anonymised by `Trace::from_exchanges`, and
//! searched before writing for the user ID, PIN, product ID, every account
//! identifier and holder name, and the payee's name, IBAN and purpose (and
//! the registered names the bank sent); if one is found nothing is written.
//! A segment fints-client does not know is shown and asked about, and the
//! trace records how the run ended (`client.outcome`), as in `record_trace`.
//! The bank's own references (order ID, reference) are only reported.
//! Review the file before committing it all the same.

use std::io::{BufRead as _, Write as _};
use std::sync::Arc;

use fints_client::jobs::{
    self, BankClient, Cancelled, Completed, Credentials, Dialog, Instant, JobValue, PayeeOutcome,
    SentAs, Step, TanKind, Transfer, TransferReceipt, TransferStatus, VopResult,
};
use fints_client::trace::outcome_label;
use fints_client::transport::HttpTransport;
use fints_client::{Account, BankConfig, Pin, ProductId, SessionState, UserId};
use jiff::Timestamp;
use rust_decimal::Decimal;

mod common;
use common::{recorder, review_unknown};

/// Largest amount without `FINTS_ALLOW_AMOUNT`.
const CAP: Decimal = Decimal::from_parts(100, 0, 0, false, 2);

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn env(name: &str) -> Res<String> {
    std::env::var(name).map_err(|_| format!("set {name}").into())
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Whether `text` holds `value`. A value of digits only (an account
/// number) counts only where no digit touches it, so it is not found inside
/// a longer number such as the message size in HNHBK.
fn contains_value(text: &str, value: &str) -> bool {
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return text.contains(value);
    }
    let bytes = text.as_bytes();
    text.match_indices(value).any(|(i, _)| {
        let before = i.checked_sub(1).map(|j| bytes[j]);
        let after = bytes.get(i + value.len()).copied();
        !before.is_some_and(|b| b.is_ascii_digit()) && !after.is_some_and(|b| b.is_ascii_digit())
    })
}

fn read_line() -> String {
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_string()
}

fn ask(prompt: &str) -> String {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    read_line()
}

/// Reads a line without echo (`stty`), falling back to visible input when
/// there is no terminal.
fn ask_hidden(prompt: &str) -> String {
    let hidden = std::process::Command::new("stty")
        .arg("-echo")
        .status()
        .is_ok_and(|s| s.success());
    let line = ask(prompt);
    if hidden {
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
    }
    line
}

async fn sleep_until(at: Option<Timestamp>) {
    if let Some(at) = at
        && let Ok(wait) = std::time::Duration::try_from(at.duration_since(Timestamp::now()))
    {
        eprintln!("  waiting {} s as the bank asks", wait.as_secs() + 1);
        tokio::time::sleep(wait).await;
    }
}

fn describe(outcome: &PayeeOutcome) -> String {
    match outcome {
        PayeeOutcome::Match => "match".into(),
        PayeeOutcome::CloseMatch { registered_name } => format!(
            "close match; registered name: {}",
            registered_name.as_deref().unwrap_or("(not given)")
        ),
        PayeeOutcome::NoMatch => "NO MATCH".into(),
        PayeeOutcome::NotApplicable { reason } => format!(
            "no check possible: {}",
            reason.as_deref().unwrap_or("(no reason given)")
        ),
        PayeeOutcome::Pending => "still pending".into(),
        PayeeOutcome::OtherId { id_name, outcome } => {
            format!("checked against {id_name}: {}", describe(outcome))
        }
    }
}

fn show_vop(result: &VopResult) {
    if result.payees.is_empty() {
        eprintln!("  the bank reported no payee");
    }
    for p in &result.payees {
        eprintln!(
            "  payee {} (name sent: {}): {}",
            p.iban,
            p.requested_name,
            describe(&p.outcome)
        );
    }
    for note in &result.notes {
        eprintln!("  note: {note}");
    }
}

/// Registered names of a result: they are payee data too.
fn registered_names(result: &VopResult, into: &mut Vec<(&'static str, String)>) {
    fn walk(outcome: &PayeeOutcome, into: &mut Vec<(&'static str, String)>) {
        match outcome {
            PayeeOutcome::CloseMatch {
                registered_name: Some(n),
            } => into.push(("registered name", n.clone())),
            PayeeOutcome::OtherId { id_name, outcome } => {
                into.push(("other ID", id_name.clone()));
                walk(outcome, into);
            }
            _ => {}
        }
    }
    for p in &result.payees {
        walk(&p.outcome, into);
    }
}

/// Everything of the run that must not be in the trace.
#[derive(Default)]
struct Seen {
    secrets: Vec<(&'static str, String)>,
    references: Vec<(&'static str, String)>,
}

/// Answers every step until the job is done: typed TAN, app approval, payee
/// check confirmation and polling. The library never sleeps or asks; this
/// is the loop a caller writes.
async fn drive<T: JobValue>(
    dialog: &mut Dialog,
    mut step: Step<T>,
    seen: &mut Seen,
) -> fints_client::Result<Option<Completed<T>>> {
    loop {
        step = match step {
            Step::Done(done) => return Ok(Some(done)),
            Step::VopPending(wait, resume) => {
                eprintln!(
                    "The payee check is not finished (poll {}).",
                    wait.polls_done
                );
                sleep_until(wait.not_before).await;
                dialog.poll(resume).await?
            }
            Step::NeedVopConfirmation(confirmation, resume) => {
                eprintln!("\nThe payee check is not a clean match:");
                show_vop(&confirmation.result);
                registered_names(&confirmation.result, &mut seen.secrets);
                if let Some(text) = &confirmation.explanation {
                    eprintln!(
                        "Bank's explanation{}:\n{text}",
                        if confirmation.explanation_markup {
                            " (with markup)"
                        } else {
                            ""
                        }
                    );
                }
                if let Some(until) = confirmation.valid_until {
                    eprintln!("The check is valid until {until} (bank time).");
                }
                if ask("confirm? [y/N] ").eq_ignore_ascii_case("y") {
                    dialog.confirm_vop(resume).await?
                } else {
                    match dialog.cancel(resume)? {
                        Cancelled::NothingSent => {
                            eprintln!("Cancelled; nothing was sent for execution.")
                        }
                        Cancelled::OrderSubmitted {
                            end_to_end_id,
                            message_id,
                        } => eprintln!(
                            "Cancelled, but the order is at the bank and may still execute: \
                             check the account (end-to-end ID {end_to_end_id}, message ID {message_id})."
                        ),
                    }
                    return Ok(None);
                }
            }
            Step::NeedTan(challenge, resume) => {
                // Once per challenge, not again on every app poll.
                let repeat = challenge.rejected.is_some()
                    || matches!(challenge.kind, TanKind::Decoupled { polls_done, .. } if polls_done > 0);
                if let Some(check) = challenge.payee_check.as_ref().filter(|_| !repeat) {
                    eprintln!("\nPayee check of this order:");
                    show_vop(check);
                    registered_names(check, &mut seen.secrets);
                }
                match challenge.kind {
                    TanKind::Typed { .. } => {
                        if let Some(r) = &challenge.rejected {
                            eprintln!(
                                "\nThe bank rejected the TAN: {} {}",
                                r.code.code, r.code.text
                            );
                            if let Some(left) = r.attempts_left {
                                eprintln!("Attempts left: {left}");
                            }
                        }
                        eprintln!("\n{}", challenge.challenge);
                        let tan = ask_hidden("TAN: ");
                        dialog.submit_tan(resume, &tan).await?
                    }
                    TanKind::Decoupled {
                        auto_polling_allowed,
                        polls_done,
                        ..
                    } => {
                        if polls_done == 0 {
                            eprintln!("\nConfirm in your banking app: {}", challenge.challenge);
                        }
                        sleep_until(challenge.not_before).await;
                        if auto_polling_allowed == Some(false) || polls_done == 0 {
                            ask("Approved in the app? Press Enter to ask the bank: ");
                        }
                        dialog.poll(resume).await?
                    }
                }
            }
        };
    }
}

fn print_receipt(r: &TransferReceipt) {
    println!("Receipt:");
    println!(
        "  sent as: {}",
        match r.sent_as {
            SentAs::Standard => "standard transfer (HKCCS)",
            SentAs::Instant => "instant transfer (HKIPZ)",
            SentAs::InstantConverted => "instant transfer converted to standard by the bank",
        }
    );
    println!("  status: {:?}", r.status);
    println!("  message ID: {}", r.message_id);
    println!("  end-to-end ID: {}", r.end_to_end_id);
    println!("  order ID: {}", r.order_id.as_deref().unwrap_or("-"));
    println!(
        "  bank reference: {}",
        r.bank_reference.as_deref().unwrap_or("-")
    );
    match &r.payee_check {
        Some(check) => {
            println!("  payee check:");
            for p in &check.payees {
                println!("    {}", describe(&p.outcome));
            }
            for note in &check.notes {
                println!("    note: {note}");
            }
        }
        None => println!("  payee check: none"),
    }
}

/// The debtor account: one of the UPD accounts with an IBAN, by number.
fn pick_account(accounts: &[jobs::AccountInfo]) -> Res<Account> {
    let usable: Vec<_> = accounts
        .iter()
        .filter(|a| a.iban.is_some() && !a.is_depot() && a.allows(jobs::JobKind::Transfer))
        .collect();
    if usable.is_empty() {
        return Err("no account with an IBAN may send HKCCS".into());
    }
    eprintln!("\nDebtor account:");
    for (n, a) in usable.iter().enumerate() {
        eprintln!(
            "  {}) {}  {}",
            n + 1,
            a.iban.as_ref().map_or("", |i| i.as_str()),
            a.product_name.as_deref().unwrap_or("")
        );
    }
    let n: usize = ask("Number: ")
        .parse()
        .ok()
        .filter(|n| (1..=usable.len()).contains(n))
        .ok_or("not a number of the list")?;
    Ok(usable[n - 1].account()?)
}

struct Order {
    payee_name: String,
    payee_iban: String,
    payee_bic: Option<String>,
    amount: Decimal,
    purpose: String,
    instant: Instant,
}

fn order_from_env() -> Res<Order> {
    let amount: Decimal = env_opt("FINTS_AMOUNT")
        .unwrap_or_else(|| "1.00".into())
        .parse()
        .map_err(|_| "FINTS_AMOUNT is not a decimal number")?;
    // The script checks this too; the example is run alone as well.
    if amount > CAP && env_opt("FINTS_ALLOW_AMOUNT").is_none() {
        return Err(
            format!("amount {amount} is above 1.00; set FINTS_ALLOW_AMOUNT to send it").into(),
        );
    }
    let instant = match env_opt("FINTS_INSTANT").as_deref() {
        None | Some("no") => Instant::No,
        Some("prefer") => Instant::Prefer,
        Some("require") => Instant::Require,
        Some(other) => {
            return Err(format!("FINTS_INSTANT is {other}: use no, prefer or require").into());
        }
    };
    Ok(Order {
        payee_name: env("FINTS_PAYEE_NAME")?,
        payee_iban: env("FINTS_PAYEE_IBAN")?,
        payee_bic: env_opt("FINTS_PAYEE_BIC"),
        amount,
        purpose: env_opt("FINTS_PURPOSE").unwrap_or_else(|| "fints-client test".into()),
        instant,
    })
}

/// Login, account choice, transfer, status of a pending instant payment,
/// dialog end. A failure is returned; the trace is written by the caller.
async fn run(
    client: &BankClient,
    user: &UserId,
    pin: &Pin,
    order: &Order,
    seen: &mut Seen,
) -> Res<()> {
    let creds = Credentials::new(user.clone(), pin.clone());
    let state = SessionState::new(&client.config().blz, user);
    let (mut dialog, step) = jobs::open(client, &creds, &state).await?;
    let result = flow(&mut dialog, step, order, seen).await;
    // End the dialog in every outcome; a failed end changes nothing.
    let _ = dialog.end().await;
    result
}

async fn flow(
    dialog: &mut Dialog,
    step: Step<jobs::Opened>,
    order: &Order,
    seen: &mut Seen,
) -> Res<()> {
    let Some(opened) = drive(dialog, step, seen).await? else {
        return Err("login cancelled".into());
    };
    for acc in &opened.value.accounts {
        if let Some(iban) = &acc.iban {
            seen.secrets.push(("IBAN", iban.as_str().to_string()));
        }
        if let Some(number) = &acc.number {
            seen.secrets.push(("account number", number.clone()));
        }
        for owner in &acc.owner {
            seen.secrets.push(("holder name", owner.clone()));
        }
    }
    let from = pick_account(&opened.value.accounts)?;

    seen.secrets.extend([
        ("payee name", order.payee_name.clone()),
        (
            "payee IBAN",
            order.payee_iban.replace(' ', "").to_uppercase(),
        ),
        ("purpose", order.purpose.clone()),
    ]);
    let transfer = Transfer {
        from: from.clone(),
        debtor_name: None,
        payee_name: order.payee_name.clone(),
        payee_iban: order.payee_iban.clone(),
        payee_bic: order.payee_bic.clone(),
        amount: order.amount,
        purpose: order.purpose.clone(),
        end_to_end_id: None,
        instant: order.instant,
    };
    eprintln!(
        "\nSending {} EUR to {} ({}), {:?}.",
        order.amount, order.payee_name, order.payee_iban, order.instant
    );
    let step = jobs::transfer(dialog, &transfer).await?;
    let Some(done) = drive(dialog, step, seen).await? else {
        return Err("cancelled at the payee check".into());
    };
    let receipt = done.value;
    if let Some(check) = &receipt.payee_check {
        registered_names(check, &mut seen.secrets);
    }
    seen.references
        .push(("message ID", receipt.message_id.clone()));
    seen.references
        .extend(receipt.order_id.clone().map(|o| ("order ID", o)));
    seen.references.extend(
        receipt
            .bank_reference
            .clone()
            .map(|o| ("bank reference", o)),
    );
    print_receipt(&receipt);
    for notice in &done.notices {
        println!("  notice: {notice}");
    }

    // An instant payment the bank has not settled yet: ask for its status
    // (HKIPS, or the statement when the bank has no HKIPS), not before
    // `not_before`, up to three times.
    if let TransferStatus::InstantPending { not_before, .. } = &receipt.status {
        let mut again = *not_before;
        for _ in 0..3 {
            sleep_until(again).await;
            let step = jobs::instant_status(dialog, &from, &receipt).await?;
            let Some(done) = drive(dialog, step, seen).await? else {
                break;
            };
            println!("Instant status: {:?}", done.value.status);
            again = done.value.again_after;
            if done.value.status.is_some() {
                break;
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Res<()> {
    let blz = env("FINTS_BLZ")?;
    let user = env("FINTS_USER")?;
    let pin = env("FINTS_PIN")?;
    let product = env("FINTS_PRODUCT_ID")?;
    let out = std::env::args()
        .nth(1)
        .ok_or("usage: transfer <out.transfer.trace.json>")?;
    let order = order_from_env()?;

    let config = match (
        fints_client::bank_by_blz(&blz),
        std::env::var("FINTS_URL").ok(),
    ) {
        (Some(c), None) => c,
        (Some(c), Some(url)) => c.with_url(url),
        (None, Some(url)) => BankConfig::new("Testbank", &blz, "TESTDEFFXXX", url),
        (None, None) => {
            return Err(format!("BLZ {blz} is not in the registry; set FINTS_URL").into());
        }
    };
    let recorder = recorder()?;
    let client = BankClient::new(config, ProductId::new(&product))
        .with_transport(recorder.wrap(Arc::new(HttpTransport::new()?)));

    let mut seen = Seen::default();
    let outcome = run(
        &client,
        &UserId::new(&user),
        &Pin::new(&pin),
        &order,
        &mut seen,
    )
    .await;
    let keep = review_unknown(&recorder);
    let keep: Vec<&str> = keep.iter().map(String::as_str).collect();
    let mut trace = recorder.trace_keeping(&keep);
    // How the run ended, as a kind of error only: no error text. An error
    // of this example (a cancelled step, no usable account) is `stopped`.
    trace.client.outcome = Some(match &outcome {
        Ok(()) => "completed".to_string(),
        Err(e) => match e.downcast_ref::<fints_client::FinTSError>() {
            Some(e) => outcome_label(e),
            None if e.to_string().contains("cancelled") => "cancelled".to_string(),
            None => "stopped".to_string(),
        },
    });

    println!("Summary of the trace:");
    print!("{}", trace.summary_table());
    if let Err(e) = &outcome {
        // `scripts/try-bank.sh` names the file after the first words.
        if trace.client.outcome.as_deref() == Some("cancelled") {
            println!("The run was cancelled (the trace is written all the same): {e}");
        } else {
            println!("The run failed (the trace is written all the same): {e}");
        }
    }

    trace.comment = format!(
        "Recorded with examples/transfer.rs at BLZ {blz}, one transfer ({:?}), {} exchanges.\n\
         Masked by the library, anonymised by Trace::from_exchanges; reviewed by hand.",
        order.instant,
        trace.exchanges.len()
    );
    let text = trace.to_json();
    let mut secrets = vec![("user ID", user), ("PIN", pin), ("product ID", product)];
    secrets.extend(seen.secrets);
    let leaks: Vec<&str> = secrets
        .iter()
        .filter(|(_, v)| v.len() >= 4 && contains_value(&text, v))
        .map(|(what, _)| *what)
        .collect();
    if !leaks.is_empty() {
        // The run may have moved money: keep the trace for a review by hand,
        // under a name the replay test does not pick up.
        let held = format!("{out}.leak");
        std::fs::write(&held, &text)?;
        return Err(format!(
            "not written to {out}: the trace still contains: {}. Kept as {held}; \
             review and mask it by hand, never share it as is",
            leaks.join(", ")
        )
        .into());
    }
    for (what, v) in &seen.references {
        if v.len() >= 4 && v != "NOTPROVIDED" && contains_value(&text, v) {
            println!("Note: the trace contains the {what}; review it before sharing.");
        }
    }
    std::fs::write(&out, text)?;
    println!("{} exchanges written to {out}", trace.exchanges.len());
    Ok(())
}
