//! Answers identity questions over local traces
//! (`fints_client_testbank::analysis`): booking references that repeat
//! within a statement of an account, references of pending entries booked
//! in another trace, end-to-end IDs of a pain.001 found again in a
//! pain.002 or on an entry. Offline. Prints counts, aliases and
//! pseudonyms only, never values.
//!
//! ```text
//! cargo run -q --features trace --example trace_identities -- testbank/fixtures/replay/local/*.trace.json
//! ```
//!
//! Traces are numbered from 0 in the order given. Accounts are aliases of
//! their own trace and are never matched across traces. References link
//! across traces only when the traces were recorded with the same salt
//! (`Recorder::with_salt`); `record_trace` and `transfer` use a new salt
//! per trace, so their traces answer questions within one trace only.

use fints_client::trace::Trace;
use fints_client_testbank::analysis::analyse;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        return Err("usage: trace_identities <file.trace.json>...".into());
    }
    let mut traces = Vec::with_capacity(files.len());
    for (n, file) in files.iter().enumerate() {
        let trace = Trace::from_json(&std::fs::read_to_string(file)?)
            .map_err(|e| format!("{file}: {e}"))?;
        println!("trace {n}: {file}");
        traces.push(trace);
    }
    print!("{}", analyse(&traces));
    Ok(())
}
