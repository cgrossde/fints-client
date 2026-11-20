//! Turns the FinTS bank list (`data/fints_institutes.csv`) into a Rust table
//! that `src/banks/mod.rs` includes as its bank directory.
//!
//! The list is a `;`-separated Windows-1252 file with one header line. Only
//! banks offering PIN/TAN are kept: a bank code of 8 digits and an http(s)
//! URL in the PIN/TAN column. When a bank code appears twice, its first row
//! wins. The table is sorted by bank code so lookups can binary-search it.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;
use std::{env, fs};

const LIST: &str = "data/fints_institutes.csv";

/// Header names of the columns we read, as the publisher spells them.
const COL_CODE: &str = "BLZ";
const COL_BIC: &str = "BIC";
const COL_NAME: &str = "Institut";
const COL_PINTAN_URL: &str = "PIN/TAN-Zugang URL";

struct Row {
    bic: String,
    name: String,
    url: String,
}

fn column(header: &csv::StringRecord, name: &str) -> Result<usize, Box<dyn Error>> {
    header
        .iter()
        .position(|h| h.trim() == name)
        .ok_or_else(|| format!("{LIST}: no column {name:?}").into())
}

fn read_list(path: &Path) -> Result<BTreeMap<String, Row>, Box<dyn Error>> {
    let raw = fs::read(path)?;
    let text = encoding_rs::WINDOWS_1252.decode(&raw).0;
    let mut csv = csv::ReaderBuilder::new()
        .delimiter(b';')
        .flexible(true)
        .from_reader(text.as_bytes());

    let header = csv.headers()?.clone();
    let code_at = column(&header, COL_CODE)?;
    let bic_at = column(&header, COL_BIC)?;
    let name_at = column(&header, COL_NAME)?;
    let url_at = column(&header, COL_PINTAN_URL)?;

    let mut banks = BTreeMap::new();
    for record in csv.records() {
        let record = record?;
        let cell = |at: usize| record.get(at).map(str::trim).unwrap_or("");
        let code = cell(code_at);
        let url = cell(url_at);
        let is_code = code.len() == 8 && code.chars().all(|c| c.is_ascii_digit());
        if !is_code || !url.starts_with("http") {
            continue;
        }
        banks.entry(code.to_string()).or_insert_with(|| Row {
            bic: cell(bic_at).to_string(),
            name: cell(name_at).to_string(),
            url: url.to_string(),
        });
    }
    Ok(banks)
}

fn render(banks: &BTreeMap<String, Row>) -> String {
    let mut out = String::from(
        "// Written by build.rs from data/fints_institutes.csv. Do not edit.\n\
         static DIRECTORY: &[DirectoryEntry] = &[\n",
    );
    for (code, row) in banks {
        out += &format!(
            "    DirectoryEntry {{ blz: {code:?}, bic: {:?}, name: {:?}, url: {:?} }},\n",
            row.bic, row.name, row.url
        );
    }
    out += "];\n";
    out
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = env::var("CARGO_MANIFEST_DIR")?;
    let out_dir = env::var("OUT_DIR")?;
    println!("cargo:rerun-if-changed={LIST}");

    // Unit tests that read the integration tests or the replay traces run
    // only in a repository checkout; the published package holds neither.
    println!("cargo::rustc-check-cfg=cfg(repo_checkout)");
    if Path::new(&manifest)
        .join("testbank/fixtures/replay")
        .is_dir()
    {
        println!("cargo::rustc-cfg=repo_checkout");
    }

    let banks = read_list(&Path::new(&manifest).join(LIST))?;
    fs::write(
        Path::new(&out_dir).join("bank_directory.rs"),
        render(&banks),
    )?;
    Ok(())
}
