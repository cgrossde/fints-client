//! The bank directory built from `data/fints_institutes.csv`.

use std::collections::BTreeSet;

use fints_client::InitMode;
use fints_client::banks::{BankConfig, all_banks, bank_by_blz};

fn known(blz: &str) -> BankConfig {
    bank_by_blz(blz).unwrap_or_else(|| panic!("{blz} is missing from the directory"))
}

#[test]
fn directory_covers_the_pin_tan_banks() {
    // The published list has well over a thousand PIN/TAN entries.
    assert!(all_banks().len() > 1000);
}

#[test]
fn every_entry_is_complete() {
    for bank in all_banks() {
        let code = bank.blz.as_str();
        assert!(
            code.len() == 8 && code.bytes().all(|b| b.is_ascii_digit()),
            "bank code {code:?}"
        );
        assert!(!bank.name.as_str().is_empty(), "{code}: no name");
        assert!(
            bank.url.as_str().starts_with("http"),
            "{code}: url {:?}",
            bank.url.as_str()
        );
    }
}

#[test]
fn bank_codes_are_unique() {
    let banks = all_banks();
    let codes: BTreeSet<_> = banks.iter().map(|b| b.blz.as_str()).collect();
    assert_eq!(codes.len(), banks.len());
}

#[test]
fn every_listed_bank_can_be_looked_up() {
    for bank in all_banks() {
        let found = known(bank.blz.as_str());
        assert_eq!(found.url.as_str(), bank.url.as_str());
    }
}

#[test]
fn dkb_entry() {
    let dkb = known("12030000");
    assert_eq!(dkb.bic.as_str(), "BYLADEM1001");
    assert!(dkb.url.as_str().starts_with("https://"));
    // Standard login: synchronisation dialog first, then the business dialog.
    assert_eq!(dkb.profile.init, InitMode::Standard);
}

#[test]
fn ing_entry() {
    let ing = known("50010517");
    assert_eq!(ing.bic.as_str(), "INGDDEFFXXX");
    assert_eq!(ing.url.as_str(), "https://fints.ing.de/fints/");
    // No synchronisation dialog; the business dialog opens with function 900.
    let direct = InitMode::Direct {
        security_function: "900".into(),
    };
    assert_eq!(ing.profile.init, direct);
}

#[test]
fn postbank_is_listed() {
    assert_eq!(known("10010010").blz.as_str(), "10010010");
}

#[test]
fn unknown_codes_give_none() {
    for code in ["00000000", "", "not-a-blz", "1203000", "120300000"] {
        assert!(bank_by_blz(code).is_none(), "{code:?}");
    }
}
