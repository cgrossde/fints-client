//! Replay of recorded dialogs (`fints_client_testbank::replay`): the bank
//! side of a masked trace is played back, and every request of the client
//! must match the recorded one in structure.
//!
//! The fixture `testbank/fixtures/replay/synthetic-decoupled.trace.json` was
//! recorded from the synthetic test bank with `record_synthetic_trace` in
//! `replay_record_test` (ignored; run it to regenerate). Traces recorded at
//! real banks with `examples/record_trace.rs` go next to it (see
//! `testbank/README.md`).
//!
//! Traces named `*.transfer.trace.json` (`scripts/try-bank.sh <BLZ> --transfer`, one payment
//! each) are not replayed: the replay runs the standard run, which sends no
//! payment, so such a trace would only report differences.

#[path = "support/replay.rs"]
mod support;

use fints_client::trace::Trace;
use fints_client::{Pin, ProductId, UserId};
use fints_client_testbank::Violation;
use fints_client_testbank::replay::{
    HttpReplay, Replay, RunReport, expect_comments, expect_path, expect_text, expected_violations,
    standard_run,
};
use std::path::Path;
use support::{SYNTHETIC, assert_account_notes, model, options, synthetic};

/// Runs the standard run against the replay of `trace`.
async fn replay(
    trace: Trace,
) -> (
    HttpReplay,
    Result<RunReport, fints_client_testbank::replay::RunError>,
) {
    let bank = Replay::new(trace).serve().await;
    let report = standard_run(
        &bank.client(&ProductId::new("TEST-123")),
        &UserId::new("replayuser1"),
        &Pin::new("replay-pin"),
        &options(),
    )
    .await;
    (bank, report)
}

#[test]
fn synthetic_fixture_holds_no_identifier_of_the_recorded_run() {
    let m = model();
    let account = &m.accounts[0];
    for secret in [
        m.customer.user_id.as_str(),
        m.customer.pin.as_str(),
        account.iban.as_deref().unwrap(),
        account.number.as_str(),
        account.holder.as_str(),
        "TESTSYS0001",
        "TESTDLG0001",
        "TEST-123",
        "1654",
        "7000999",
    ] {
        assert!(!SYNTHETIC.contains(secret), "fixture contains {secret:?}");
    }
    let trace = synthetic();
    assert!(trace.exchanges.len() > 8, "{}", trace.exchanges.len());
}

/// The replay's client is pinned to the time of the recording (first HNSHK).
#[test]
fn recording_time_is_the_first_signature_stamp() {
    let at = fints_client_testbank::replay::recording_time(&synthetic()).expect("HNSHK stamp");
    assert!(at.year() >= 2026, "{at}");
}

#[test]
fn summary_of_the_synthetic_trace_tells_the_story() {
    let summary = synthetic().summary();
    let lines: Vec<String> = summary.iter().map(|s| s.to_string()).collect();
    let kazs: Vec<_> = summary
        .iter()
        .filter(|s| s.jobs == ["HKKAZ:7"] && s.tan.is_some())
        .collect();
    assert_eq!(kazs.len(), 3, "{lines:#?}");
    assert!(kazs.iter().all(|s| !s.failed()), "{lines:#?}");
    assert!(
        lines.iter().any(|l| l.contains("+HKTAN(4,HKKAZ)")),
        "{lines:#?}"
    );
    assert!(summary.iter().all(|s| !s.failed()), "{lines:#?}");
    assert_account_notes(&synthetic());
}

#[tokio::test]
async fn replay_of_the_synthetic_trace_is_clean() {
    let (bank, report) = replay(synthetic()).await;
    bank.assert_clean();
    let report = report.expect("the run succeeds against the replay");
    assert_eq!(report.accounts.len(), 2, "giro and depot");
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    let fetched = report.fetched[0]
        .as_ref()
        .expect("balance and transactions");
    assert!(fetched.balance.is_some());
    assert_eq!(
        fetched.transactions.len(),
        3,
        "one recorded booking per page"
    );
    assert_eq!(bank.served(), synthetic().exchanges.len());
}

/// Replays `trace` and compares the violations with the `.expect` sidecar
/// next to `path` (none: the replay must be clean). Both are compared as an
/// exact multiset, so a new difference and a difference that went away both
/// fail. With `bless` the sidecar is written from the actual violations
/// (comment lines kept; removed when there are none), unless the run went
/// out of step.
async fn check_trace(path: &Path, trace: Trace, bless: bool) -> Result<(), String> {
    let label = path.display();
    let sidecar = expect_path(path);
    let existing = std::fs::read_to_string(&sidecar).ok();
    let expected = match &existing {
        Some(text) => {
            expected_violations(text).map_err(|e| format!("{}: {e}", sidecar.display()))?
        }
        None => Vec::new(),
    };
    let summary = trace.summary();
    let jobs = summary.iter().filter(|s| !s.jobs.is_empty()).count();
    let (bank, report) = replay(trace).await;
    let actual = bank.violations();
    report.map_err(|e| format!("{label}: the run failed: {e}"))?;
    if bless {
        let stopped = |v: &Violation| {
            v.message.contains("the client stopped after")
                || v.message.contains("the client sent one more")
        };
        let differing = actual
            .iter()
            .filter(|v| v.message.starts_with("segments differ"))
            .count();
        if actual.iter().any(stopped) || differing > jobs {
            return Err(format!(
                "{label}: not blessed, the run went out of step with the trace \
                 (the comparison is meaningless): move the trace to fixtures/replay/stale/ \
                 and re-record it\n{}",
                diff(&summary, &[], &actual)
            ));
        }
        if actual.is_empty() {
            let _ = std::fs::remove_file(&sidecar);
        } else {
            let comments = existing.as_deref().map(expect_comments).unwrap_or_default();
            std::fs::write(&sidecar, expect_text(&comments, &actual))
                .map_err(|e| format!("{}: {e}", sidecar.display()))?;
        }
        return Ok(());
    }
    let (missing, unexpected) = multiset_difference(&expected, &actual);
    if missing.is_empty() && unexpected.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{label}: the violations differ from {}\n{}\
         (REPLAY_BLESS=1 rewrites the sidecar; the commit must explain each line)",
        sidecar.display(),
        diff(&summary, &missing, &unexpected)
    ))
}

/// `(expected but not seen, seen but not expected)`.
fn multiset_difference(
    expected: &[Violation],
    actual: &[Violation],
) -> (Vec<Violation>, Vec<Violation>) {
    let mut unexpected = actual.to_vec();
    let mut missing = Vec::new();
    for e in expected {
        match unexpected.iter().position(|a| a == e) {
            Some(i) => {
                unexpected.remove(i);
            }
            None => missing.push(e.clone()),
        }
    }
    (missing, unexpected)
}

/// Unified-style list: `-` expected but gone, `+` new, each followed by the
/// summary line of its exchange.
fn diff(
    summary: &[fints_client::trace::ExchangeSummary],
    missing: &[Violation],
    unexpected: &[Violation],
) -> String {
    let mut out = String::new();
    for (sign, list) in [('-', missing), ('+', unexpected)] {
        for v in list {
            out.push_str(&format!("{sign} {v}\n"));
            if let Some(s) = summary.get(v.exchange) {
                out.push_str(&format!("    {s}\n"));
            }
        }
    }
    out
}

fn fixture_traces() -> Vec<std::path::PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testbank/fixtures/replay");
    let mut dirs = vec![dir.clone()];
    if std::env::var_os("REPLAY_LOCAL").is_some_and(|v| v == "1") {
        dirs.push(dir.join("local"));
    }
    let mut paths: Vec<_> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".trace.json") && !n.ends_with(".transfer.trace.json"))
        })
        .collect();
    paths.sort();
    paths
}

/// Every trace in the fixture directory replays as its `.expect` sidecar
/// says (no sidecar: clean): dropping a trace there makes it a regression
/// test. With `REPLAY_LOCAL=1` the traces in `local/` (gitignored live
/// recordings) are replayed too. `REPLAY_BLESS=1` rewrites the sidecars from
/// the actual violations (see `testbank/README.md`).
#[tokio::test]
async fn every_trace_in_the_fixture_directory_replays_as_expected() {
    let bless = std::env::var_os("REPLAY_BLESS").is_some_and(|v| v == "1");
    let paths = fixture_traces();
    assert!(!paths.is_empty());
    let mut failures = Vec::new();
    for path in paths {
        let json = std::fs::read_to_string(&path).expect("read trace");
        let trace = Trace::from_json(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if let Err(e) = check_trace(&path, trace, bless).await {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fints-replay-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[tokio::test]
async fn expectation_matching_the_edit_passes_and_a_wrong_one_fails() {
    let dir = temp_dir("expect");
    let path = dir.join("edited.trace.json");
    let expect = expect_path(&path);

    // Without a sidecar the edit is a new difference, and the failure shows
    // it with the summary line of the exchange.
    let e = check_trace(&path, edited("HKSAL:3:7", "HKSAL:3:6"), false)
        .await
        .unwrap_err();
    assert!(e.contains("+ 5 HKSAL:3: version 6 recorded, 7 sent"), "{e}");
    assert!(e.contains("    #05  HKSAL:6"), "{e}");

    // The same differences, written as a sidecar by hand.
    let (_, report) = replay(edited("HKSAL:3:7", "HKSAL:3:6")).await;
    report.expect("the run still succeeds");
    let lines = {
        let (bank, _) = replay(edited("HKSAL:3:7", "HKSAL:3:6")).await;
        bank.violations()
    };
    assert!(!lines.is_empty());
    std::fs::write(&expect, expect_text(&["# test".into()], &lines)).unwrap();
    check_trace(&path, edited("HKSAL:3:7", "HKSAL:3:6"), false)
        .await
        .expect("matching expectation");

    // A wrong expectation: one line changed, so one is missing, one is new.
    let wrong = std::fs::read_to_string(&expect)
        .unwrap()
        .replace("6 recorded, 7 sent", "5 recorded, 7 sent");
    std::fs::write(&expect, wrong).unwrap();
    let e = check_trace(&path, edited("HKSAL:3:7", "HKSAL:3:6"), false)
        .await
        .unwrap_err();
    assert!(e.contains("- ") && e.contains("+ "), "{e}");

    // A stale sidecar: the trace replays clean, the expectation is left.
    let e = check_trace(&path, synthetic(), false).await.unwrap_err();
    assert!(e.contains("- "), "{e}");
    assert!(!e.contains("+ "), "{e}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn bless_writes_the_sidecar_keeps_comments_and_refuses_out_of_step() {
    let dir = temp_dir("bless");
    let path = dir.join("edited.trace.json");
    let expect = expect_path(&path);
    std::fs::write(&expect, "# client: HKSAL 7 to 6\n# second line\n").unwrap();

    check_trace(&path, edited("HKSAL:3:7", "HKSAL:3:6"), true)
        .await
        .expect("bless");
    let text = std::fs::read_to_string(&expect).unwrap();
    assert!(
        text.starts_with(
            "# client: HKSAL 7 to 6\n# second line\n5 HKSAL:3: version 6 recorded, 7 sent\n"
        ),
        "{text}"
    );
    check_trace(&path, edited("HKSAL:3:7", "HKSAL:3:6"), false)
        .await
        .expect("the blessed sidecar matches");

    // The client sends more than the trace holds: out of step, not blessed.
    let before = std::fs::read_to_string(&expect).unwrap();
    let mut shorter = synthetic();
    shorter.exchanges.pop();
    let e = check_trace(&path, shorter, true).await.unwrap_err();
    assert!(e.contains("went out of step"), "{e}");
    assert_eq!(std::fs::read_to_string(&expect).unwrap(), before);

    // A clean run blesses the sidecar away.
    check_trace(&path, synthetic(), true).await.expect("bless");
    assert!(!expect.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// The synthetic trace with `from` replaced by `to` in every raw message
/// (requests and responses), as if it had been recorded that way.
fn edited(from: &str, to: &str) -> Trace {
    let mut trace = synthetic();
    let mut changed = false;
    for ex in &mut trace.exchanges {
        for msg in [&mut ex.request, &mut ex.response] {
            let text: String = msg.iter().copied().map(char::from).collect();
            if text.contains(from) {
                changed = true;
                *msg = text.replace(from, to).chars().map(|c| c as u8).collect();
            }
        }
    }
    assert!(changed, "the fixture has {from:?}");
    trace
}

#[test]
fn a_hand_edited_fixture_is_rejected() {
    // The raw request still says HKSAL:3:7.
    let json = SYNTHETIC.replacen("\"HKSAL:7\"", "\"HKSAL:6\"", 1);
    assert_ne!(json, SYNTHETIC);
    let e = Trace::from_json(&json).unwrap_err();
    assert!(
        e.contains(": derived field sent.jobs[0] is \"HKSAL:6\" in the file"),
        "{e}"
    );
}

#[tokio::test]
async fn changed_segment_version_is_a_violation() {
    // As if the client had sent HKSAL 6 when the recording was made.
    let (bank, _) = replay(edited("HKSAL:3:7", "HKSAL:3:6")).await;
    let v = bank.violations();
    assert!(
        v.iter()
            .any(|v| v.message == "HKSAL:3: version 6 recorded, 7 sent"),
        "{v:#?}"
    );
}

#[tokio::test]
async fn missing_hktan_is_a_violation() {
    // The recorded BPD now say HKSAL needs no TAN: the client leaves HKTAN
    // out, the recording has it.
    let (bank, _) = replay(edited("HKSAL:J", "HKSAL:N")).await;
    let v = bank.violations();
    assert!(
        v.iter().any(|v| v.message.starts_with("segments differ")
            && v.message.contains("HKSAL:3:7 HKTAN:4:7 HNSHA:5:2")
            && v.message
                .contains("sent [HNHBK:1:3 HNVSK:998:3 HNVSD:999:1 HNSHK:2:4 HKSAL:3:7 HNSHA:4:2")),
        "{v:#?}"
    );
}

#[tokio::test]
async fn client_stopping_early_or_sending_more_is_a_violation() {
    let mut longer = synthetic();
    let last = longer.exchanges.last().unwrap().clone();
    longer.exchanges.push(last);
    let (bank, _) = replay(longer).await;
    assert!(
        bank.violations()
            .iter()
            .any(|v| v.message.contains("the client stopped after")),
        "{:#?}",
        bank.violations()
    );

    let mut shorter = synthetic();
    shorter.exchanges.pop();
    let (bank, _) = replay(shorter).await;
    assert!(
        bank.violations()
            .iter()
            .any(|v| v.message.contains("the client sent one more")),
        "{:#?}",
        bank.violations()
    );
}
