//! Records one dialog with a bank as a replay fixture for the test bank
//! (`fints_client_testbank::replay`): first login (synchronisation, TAN),
//! balance and transactions for every account, dialog end. A human runs
//! this; see `testbank/README.md` for the exact command.
//!
//! ```text
//! FINTS_BLZ=12030000 FINTS_USER=... FINTS_PIN=... FINTS_PRODUCT_ID=... \
//!   cargo run --features trace --example record_trace -- testbank/fixtures/replay/local/dkb.trace.json [days]
//! ```
//!
//! `FINTS_URL` overrides the bank's URL (e.g. the local `fints-testbank`).
//! Account identifiers get aliases of this trace, references pseudonyms
//! keyed by `FINTS_TRACE_SALT` (64 hex digits, e.g. from `.env`) when set,
//! else by a random salt that is never written (`Recorder::new`).
//!
//! What is written: a JSON trace (`Trace::to_json`): each request and response as `fints_client::trace::Recorder`
//! (a transport decorator) records it, redacted by the library and
//! anonymised further by `Trace::from_exchanges`. Before writing,
//! the trace is searched for the user ID, PIN, product ID and every account
//! identifier and holder name of the run; if one is found nothing is
//! written. Review the file before committing it all the same.

use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use fints_client::jobs::{BankClient, TanChallenge};
use fints_client::trace::outcome_label;
use fints_client::transport::HttpTransport;
use fints_client::{BankConfig, Pin, ProductId, UserId};
use fints_client_testbank::replay::{RunError, RunEvent, RunOptions, standard_run};

mod common;
use common::{recorder, review_unknown};

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("set {name}"))
}

fn on_event(event: RunEvent) {
    match event {
        RunEvent::AwaitingConfirmation { challenge } => {
            println!("Confirm in your banking app: {challenge}")
        }
        RunEvent::StillPending { poll } => println!("  still pending (poll {poll})"),
        RunEvent::Confirmed => println!("  confirmed"),
    }
}

fn typed_tan(challenge: &TanChallenge) -> String {
    print!("{}\nTAN: ", challenge.challenge);
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_string()
}

/// The bank does not allow automatic polling: the user asks for each
/// status request.
fn wait_for_user(_: &TanChallenge) {
    print!("Confirmed in the app? Press Enter to ask the bank: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let blz = env("FINTS_BLZ")?;
    let user = env("FINTS_USER")?;
    let pin = env("FINTS_PIN")?;
    let product = env("FINTS_PRODUCT_ID")?;
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .ok_or("usage: record_trace <out.trace.json> [days]")?;
    let days: u32 = args.next().and_then(|d| d.parse().ok()).unwrap_or(30);

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
    // The replay runs the same jobs, so recording and replay drive the
    // client the same way.
    let recorder = recorder()?;
    let client = BankClient::new(config, ProductId::new(&product))
        .with_transport(recorder.wrap(Arc::new(HttpTransport::new()?)));

    let opts = RunOptions {
        timeout: Duration::from_secs(300),
        on_event,
        typed_tan,
        wait_for_user,
        ..RunOptions::new(days)
    };
    let report = standard_run(&client, &UserId::new(&user), &Pin::new(&pin), &opts).await;
    let keep = review_unknown(&recorder);
    let keep: Vec<&str> = keep.iter().map(String::as_str).collect();
    let mut trace = recorder.trace_keeping(&keep);
    // How the run ended, as kinds of error only: no error text.
    let label = |e: &RunError| match e {
        RunError::Fints(e) => outcome_label(e),
        RunError::TanTimeout => "TanTimeout".to_string(),
    };
    trace.client.outcome = Some(match &report {
        Err(e) => label(e),
        Ok(report) => {
            let failed: Vec<String> = report
                .fetched
                .iter()
                .flat_map(|fetched| match fetched {
                    Ok(r) => r
                        .errors
                        .iter()
                        .map(|(job, e)| format!("{job:?} {}", label(e)))
                        .collect(),
                    Err(e) => vec![format!("account {}", label(e))],
                })
                .collect();
            if failed.is_empty() {
                "completed".to_string()
            } else {
                format!("completed; failed: {}", failed.join(", "))
            }
        }
    });

    println!("Summary of the trace:");
    print!("{}", trace.summary_table());

    let mut secrets = vec![
        ("user ID", user.clone()),
        ("PIN", pin),
        ("product ID", product),
    ];
    match &report {
        Ok(report) => {
            println!("{} accounts", report.accounts.len());
            for (acc, fetched) in report.accounts.iter().zip(&report.fetched) {
                match fetched {
                    Ok(r) => println!(
                        "  {} {}: balance {}, {} transactions, pending {}, {} holdings{}",
                        recorder
                            .account_of(acc)
                            .map_or_else(|| "(no alias)".to_string(), |a| a.alias),
                        acc.product_name.as_deref().unwrap_or("?"),
                        if r.balance.is_some() { "yes" } else { "no" },
                        r.transactions.len(),
                        pending_summary(r.pending.as_ref()),
                        r.holdings.len(),
                        r.errors
                            .iter()
                            .map(|(job, e)| format!("; {job:?} failed: {e}"))
                            .collect::<String>()
                    ),
                    Err(e) => println!(
                        "  {} [{}]: {e}",
                        acc.product_name.as_deref().unwrap_or("?"),
                        recorder
                            .account_of(acc)
                            .map_or_else(|| "no alias".to_string(), |a| a.to_string())
                    ),
                }
                if let Some(iban) = &acc.iban {
                    secrets.push(("IBAN", iban.as_str().to_string()));
                }
                if let Some(number) = &acc.number {
                    secrets.push(("account number", number.clone()));
                }
                for owner in &acc.owner {
                    secrets.push(("holder name", owner.clone()));
                }
            }
        }
        Err(e) => println!("The run failed (the trace is written all the same): {e}"),
    }

    trace.comment = format!(
        "Recorded with examples/record_trace.rs at BLZ {blz}, {days} days, {} exchanges.\n\
         Masked by the library, anonymised by Trace::from_exchanges.",
        trace.exchanges.len()
    );
    let text = trace.to_json();
    let leaks = leaks(&trace, &secrets);
    if !leaks.is_empty() {
        return Err(format!(
            "not written: the trace still contains a {}",
            leaks.join(", ")
        )
        .into());
    }
    std::fs::write(&out, text)?;
    println!("{} exchanges written to {out}", trace.exchanges.len());
    let sidecar = fints_client_testbank::replay::expect_path(std::path::Path::new(&out));
    if sidecar.exists() {
        println!(
            "Note: {} lists expected differences for the old trace. Replay the new one \
             (REPLAY_LOCAL=1 cargo test --test replay_test); if it replays clean, delete the \
             sidecar, otherwise it is stale.",
            sidecar.display()
        );
    }
    Ok(())
}

/// The kinds of secret that still show in the trace. The messages are
/// searched as text without line breaks and without the `?nn` subfield
/// codes of MT940, ignoring case, so a value the bank wrapped is found too;
/// each word of a name is searched on its own.
fn leaks<'a>(trace: &fints_client::trace::Trace, secrets: &'a [(&'a str, String)]) -> Vec<&'a str> {
    let squash = |text: &str| -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' || c == '\n' {
                continue;
            }
            if c == '?' {
                let code: String = chars.clone().take(2).collect();
                if code.len() == 2 && code.chars().all(|d| d.is_ascii_digit()) {
                    chars.nth(1);
                    continue;
                }
            }
            out.extend(c.to_lowercase());
        }
        out
    };
    let haystack: String = trace
        .exchanges
        .iter()
        .flat_map(|ex| [&ex.request, &ex.response])
        .map(|m| squash(&m.iter().copied().map(char::from).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = Vec::new();
    for (what, value) in secrets {
        let words: Vec<String> = if *what == "holder name" {
            value.split_whitespace().map(squash).collect()
        } else {
            vec![squash(value)]
        };
        if words
            .iter()
            .any(|w| w.chars().count() >= 4 && haystack.contains(w.as_str()))
            && !found.contains(what)
        {
            found.push(*what);
        }
    }
    found
}

/// The pending part in a few words: whether the bank sent one (MT942,
/// camt), how many entries, and when it says it made the list.
fn pending_summary(
    pending: Option<&(fints_client::jobs::Pending, Option<fints_client::BankTime>)>,
) -> String {
    use fints_client::jobs::Pending;
    match pending {
        None => "-".into(),
        Some((Pending::Reported(t), at)) => format!(
            "{} reported{}",
            t.len(),
            at.map(|a| format!(
                " (made {}{})",
                a.datetime,
                a.offset.map(|o| format!(" {o}")).unwrap_or_default()
            ))
            .unwrap_or_default()
        ),
        Some((Pending::NotSent, _)) => "not sent".into(),
        Some((Pending::Unreadable { message, .. }, _)) => format!("UNREADABLE ({message})"),
        Some((other, _)) => format!("{other:?}"),
    }
}
