//! Prints the summary of recorded traces for humans: a legend line, the
//! accounts, then one line per exchange. Offline. Scripts and LLMs read the
//! JSON itself.
//!
//! ```text
//! cargo run -q --features trace --example trace_summary -- testbank/fixtures/replay/*.trace.json
//! ```

use fints_client::trace::Trace;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        return Err("usage: trace_summary <file.trace.json>...".into());
    }
    for file in &files {
        let trace = Trace::from_json(&std::fs::read_to_string(file)?)
            .map_err(|e| format!("{file}: {e}"))?;
        println!("{file}");
        print!("{}", trace.summary_table());
    }
    Ok(())
}
