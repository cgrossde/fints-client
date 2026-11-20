//! What the recording examples share: the recorder and the question about
//! segments fints-client does not know.

use std::io::Write as _;

use fints_client::trace::{Recorder, Salt};

/// The recorder: keyed by `FINTS_TRACE_SALT` (64 hex digits) when it is
/// set, so traces of several runs share their reference pseudonyms; else by
/// a random salt that is never written. Account identifiers get aliases of
/// each trace either way.
pub fn recorder() -> Result<Recorder, String> {
    match std::env::var("FINTS_TRACE_SALT") {
        Ok(hex) if !hex.trim().is_empty() => Salt::from_hex(&hex)
            .map(Recorder::with_salt)
            .map_err(|e| format!("FINTS_TRACE_SALT: {e}")),
        _ => Ok(Recorder::new()),
    }
}

/// Asks, for each kind of segment the library has no field list for,
/// whether the trace may show it. Such a segment is what a new bank adds,
/// and a trace that shows only its header and field lengths is rarely
/// enough to support it; but nobody knows what it holds, so a person looks
/// first. Returns the codes to keep. Without a terminal nothing is kept.
pub fn review_unknown(recorder: &Recorder) -> Vec<String> {
    use std::io::IsTerminal as _;
    let unknown = recorder.unknown_segments();
    if unknown.is_empty() {
        return Vec::new();
    }
    println!(
        "\nThe bank sent {} kind(s) of segment fints-client does not know.",
        unknown.len()
    );
    if !std::io::stdin().is_terminal() {
        println!("No terminal to ask: the trace keeps only their header and field lengths.");
        return Vec::new();
    }
    println!(
        "By default the trace keeps only their header and field lengths. Each is shown\n\
         below exactly as it was sent. Keep one only if you see no personal data in it\n\
         (name, account, amount, address, text you wrote)."
    );
    let mut keep = Vec::new();
    for segment in unknown {
        let shown: String = segment.example.chars().take(600).collect();
        let cut = if segment.example.chars().count() > 600 {
            " … (cut)"
        } else {
            ""
        };
        println!(
            "\n{}:{} ({} in this run):\n  {shown}{cut}",
            segment.code, segment.version, segment.count
        );
        print!("Keep {} readable in the trace? [y/N] ", segment.code);
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_ok()
            && matches!(answer.trim(), "y" | "Y" | "yes")
        {
            keep.push(segment.code);
        }
    }
    keep
}
