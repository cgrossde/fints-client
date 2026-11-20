//! Recording a dialog with `fints_client::trace::Recorder`, a decorator
//! around the transport: it records exactly what goes through the wrapped
//! transport, whatever thread or test runs in parallel.

#[path = "support/replay.rs"]
mod support;

use std::sync::Arc;

use fints_client::trace::{Recorder, Salt, Trace};
use fints_client::transport::HttpTransport;
use fints_client::{Iban, Pin, ProductId, UserId};
use fints_client_testbank::TestBank;
use fints_client_testbank::replay::{Replay, standard_run};
use support::{SYNTHETIC_PATH, assert_account_notes, model, options, synthetic};

/// Runs the standard run against the synthetic test bank through a
/// [`Recorder`]; returns the trace and the number of requests the bank saw.
async fn record() -> (Trace, usize) {
    record_with(Recorder::new()).await
}

/// Like [`record`], through `recorder`.
async fn record_with(recorder: Recorder) -> (Trace, usize) {
    let bank = TestBank::new(model()).serve().await;
    let m = bank.model();
    let client = bank
        .client(&ProductId::new("TEST-123"))
        .with_transport(recorder.wrap(Arc::new(HttpTransport::new().expect("transport"))));
    let report = standard_run(
        &client,
        &UserId::new(&m.customer.user_id),
        &Pin::new(&m.customer.pin),
        &options(),
    )
    .await
    .expect("run");
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    bank.assert_clean();
    (recorder.trace(), bank.requests().len())
}

#[tokio::test]
async fn recorder_keeps_every_exchange_masked() {
    let (trace, requests) = record().await;
    assert_eq!(trace.exchanges.len(), requests);
    let text = trace.to_json();
    for secret in [
        "testuser1",
        "test-pin-1",
        "0001234567",
        "TEST-123",
        "TESTSYS0001",
        "Girokonto",
    ] {
        assert!(!text.contains(secret), "trace contains {secret:?}");
    }
    assert_eq!(
        Trace::from_json(&text).expect("loads").exchanges.len(),
        synthetic().exchanges.len(),
        "the committed fixture has as many exchanges as a new recording"
    );
}

#[tokio::test]
async fn recorder_notes_name_the_account_of_each_job() {
    let (trace, _) = record().await;
    assert_account_notes(&trace);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_recordings_stay_apart() {
    let (a, b) = tokio::join!(tokio::spawn(record()), tokio::spawn(record()));
    for (trace, requests) in [a.expect("task a"), b.expect("task b")] {
        // Each recording holds exactly its own run, not the other's.
        assert_eq!(trace.exchanges.len(), requests);
        assert_eq!(trace.exchanges.len(), synthetic().exchanges.len());
    }
}

/// The value of the first `<code>:…+value'` segment in the trace's
/// answers (inside HNVSD too: the answers are searched as text).
fn first_value(trace: &Trace, code: &str) -> String {
    let text: String = trace
        .exchanges
        .iter()
        .flat_map(|ex| ex.response.iter().map(|&b| char::from(b)))
        .collect();
    let at = text.find(&format!("{code}:")).expect("segment recorded");
    let seg = &text[at..];
    let seg = &seg[..seg.find('\'').expect("segment end")];
    seg.split('+').nth(1).expect("data element 1").to_string()
}

/// Account identifiers get aliases of one trace: two recordings with the
/// same salt give the same IBAN other aliases. References get pseudonyms
/// keyed by the salt: the same salt links them, another salt does not. A
/// recorded trace replays clean and the client reads the aliases as valid
/// values.
#[tokio::test]
async fn recording_aliases_accounts_links_references_and_replays_clean() {
    let salt = Salt::from_bytes([7; 32]);
    let (a, _) = record_with(Recorder::with_salt(salt.clone())).await;
    let (b, _) = record_with(Recorder::with_salt(salt)).await;
    let (c, _) = record_with(Recorder::with_salt(Salt::from_bytes([8; 32]))).await;
    let system_id = first_value(&a, "HISYN");
    let a = Trace::from_json(&a.to_json()).expect("the trace loads");

    // The replayed UPD holds the aliases: a valid IBAN and an account
    // number with the leading zeros of the original, matching each other.
    let replay = Replay::new(a).serve().await;
    let report = standard_run(
        &replay.client(&ProductId::new("TEST-123")),
        &UserId::new("replayuser1"),
        &Pin::new("replay-pin"),
        &options(),
    )
    .await
    .expect("the run succeeds against the replay");
    replay.assert_clean();
    assert!(report.errors().is_empty(), "{:?}", report.errors());
    let giro = &report.accounts[0];
    let m = model();
    let iban = giro.iban.as_ref().expect("IBAN").as_str().to_string();
    assert!(Iban::parse(&iban).is_ok(), "{iban}");
    assert_ne!(Some(iban.as_str()), m.accounts[0].iban.as_deref());
    let real = m.accounts[0].iban.as_deref().expect("model IBAN");
    assert_eq!(&iban[..2], &real[..2], "country kept");
    assert_eq!(&iban[4..12], &real[4..12], "bank code kept");
    let number = giro.number.clone().expect("number");
    assert_eq!(number.len(), m.accounts[0].number.len());
    assert!(number.starts_with("000"), "{number}");
    assert_ne!(number, m.accounts[0].number);
    assert!(
        iban.ends_with(&number),
        "IBAN {iban} and KTV {number} match"
    );
    let fetched = report.fetched[0].as_ref().expect("giro fetched");
    assert_eq!(fetched.transactions.len(), 3);

    // Same salt: other account aliases, the same reference pseudonyms;
    // another salt: other pseudonyms.
    let (b, c) = (b.to_json(), c.to_json());
    for value in [&iban, &number] {
        assert!(!b.contains(value.as_str()), "{value} in the second trace");
        assert!(!c.contains(value.as_str()), "{value} in the third trace");
    }
    assert_ne!(system_id, "TESTSYS0001", "system ID masked");
    assert!(
        b.contains(&system_id),
        "{system_id} not in the second trace"
    );
    assert!(
        !c.contains(&system_id),
        "{system_id} in the trace of another salt"
    );
}

#[tokio::test]
#[ignore = "writes the fixture; run to regenerate it"]
async fn record_synthetic_trace() {
    let (mut trace, _) = record().await;
    trace.comment =
        "Synthetic: recorded from fints-client-testbank (tests/replay_record_test.rs,\n\
         record_synthetic_trace). Decoupled login approved on the second poll,\n\
         HKSAL and HKKAZ with HKTAN, three bookings paged one per page;\n\
         HKWPD for a depot without IBAN and Kontoart."
            .into();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SYNTHETIC_PATH);
    std::fs::write(path, trace.to_json()).expect("write fixture");
}
