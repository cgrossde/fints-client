//! Masks trace files again with the masking rules of this version of
//! fints-client (`Trace::remask`) and writes them back in place. For
//! traces recorded before the rules hid more; nothing contacts a bank.
//!
//! ```text
//! cargo run --features trace --example trace_remask -- testbank/fixtures/replay/local/*.json
//! ```

use fints_client::trace::Trace;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        return Err("usage: trace_remask <file.trace.json>...".into());
    }
    for path in paths {
        let mut trace = Trace::from_json(&std::fs::read_to_string(&path)?)
            .map_err(|e| format!("{path}: {e}"))?;
        trace.remask();
        std::fs::write(&path, trace.to_json())?;
        println!("masked again: {path} ({} exchanges)", trace.exchanges.len());
    }
    Ok(())
}
