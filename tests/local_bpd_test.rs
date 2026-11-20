//! The payment parameter segments of the live traces of 2026-11-17 read
//! through their definitions (no raw fallback). The traces are masked and
//! git-ignored (`testbank/fixtures/replay/local/`); a missing trace skips
//! its bank, as `bank_profiles_test` does.

use std::path::PathBuf;

use fints_client::codec::{self, ParsedSegment, wire};
use fints_client::protocol::{Bpd, JobDetail};
use serde_json::Value;

const PAYMENT_PARAMS: [&str; 6] = ["HICCSS", "HIIPZS", "HIIPSS", "HIVPPS", "HIVPAS", "HISPAS"];

fn response_segments(json: &Value) -> Vec<String> {
    json["exchanges"]
        .as_array()
        .expect("exchanges")
        .iter()
        .flat_map(|e| e["response"].as_array().into_iter().flatten())
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect()
}

#[test]
fn live_payment_parameters_read_typed() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testbank/fixtures/replay/local");
    for bank in ["dkb", "comdirect", "consorsbank"] {
        let path = dir.join(format!("{bank}.trace.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("skip {bank}: no trace at {}", path.display());
            continue;
        };
        let json: Value = serde_json::from_str(&text).expect("json");
        let mut seen = Vec::new();
        for wire_text in response_segments(&json) {
            let code = wire_text.split(':').next().unwrap_or_default().to_string();
            if !PAYMENT_PARAMS.contains(&code.as_str()) {
                continue;
            }
            let parsed = codec::parse_message(wire_text.as_bytes()).expect("tokenizes");
            for p in parsed {
                assert!(
                    matches!(p, ParsedSegment::Typed { .. }),
                    "{bank}: {code} stayed raw: {wire_text}"
                );
            }
            seen.push(code);
        }
        // The parameter set a bank sends is the whole BPD: the model reads it
        // without a rejected segment.
        let segments: Vec<_> = response_segments(&json)
            .iter()
            .filter(|s| {
                let c = s.split(':').next().unwrap_or_default();
                c == "HIBPA" || (c.starts_with("HI") && c.ends_with('S') && c.len() == 6)
            })
            .flat_map(|s| wire::tokenize(s.as_bytes()).expect("tokenizes"))
            .collect();
        let bpd = Bpd::from_segments(&segments).expect("BPD");
        for u in bpd.unparsed() {
            assert!(
                !PAYMENT_PARAMS.contains(&u.segment.segment_type()),
                "{bank}: {} unparsed: {}",
                u.segment.segment_type(),
                u.error
            );
        }
        for code in ["HKIPZ", "HKVPP", "HKIPS"] {
            if let Some(job) = bpd.job(code) {
                for (v, jv) in &job.versions {
                    assert!(
                        !matches!(jv.detail, JobDetail::Raw(_)),
                        "{bank}: {code} v{v} read header-only"
                    );
                }
            }
        }
        eprintln!("{bank}: payment parameter segments read typed: {seen:?}");
    }
}
