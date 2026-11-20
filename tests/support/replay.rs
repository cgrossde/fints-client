//! Shared by `replay_test` and `replay_record_test`: the synthetic bank
//! the fixture is recorded from, the run options, the fixture.

#![allow(dead_code)]

use std::time::Duration;

use fints_client::trace::Trace;
use fints_client_testbank::replay::RunOptions;
use fints_client_testbank::{self as testbank, BankModel, Booking, date};
use rust_decimal::Decimal;

pub const SYNTHETIC: &str =
    include_str!("../../testbank/fixtures/replay/synthetic-decoupled.trace.json");
pub const SYNTHETIC_PATH: &str = "testbank/fixtures/replay/synthetic-decoupled.trace.json";

const MT535: &str = ":16R:GENL\r\n:98A::STAT//20260410\r\n:16S:GENL\r\n\
:16R:FIN\r\n:35B:ISIN XX0000TEST01\r\nTESTFONDS WELT\r\n:93B::AGGR//UNIT/40,\r\n:16S:FIN\r\n-";

pub fn options() -> RunOptions {
    RunOptions {
        bank_waits: false,
        timeout: Duration::from_secs(5),
        first_wait: Duration::from_millis(10),
        next_wait: Duration::from_millis(10),
        ..RunOptions::new(90)
    }
}

/// Decoupled login (approved on the second poll), HKSAL and HKKAZ with
/// HKTAN, three bookings paged one per page (touchdown); then HKWPD for a
/// depot without IBAN and Kontoart.
pub fn model() -> BankModel {
    let mut m = BankModel {
        tan_methods: vec![testbank::TanMethod::decoupled(902, "Test-App", 2)],
        hitans_version: 7,
        login_sca: testbank::LoginSca::Tan,
        ..BankModel::default()
    };
    let number = m.accounts[0].number.clone();
    m.accounts[0] = testbank::Account::giro(&m.blz, &number, "Testkundin Eins").with_bookings(
        Decimal::new(50_000, 2),
        (1..=3)
            .map(|i| {
                Booking::new(
                    date(2026, 3, i),
                    &format!("-{i}.50"),
                    &format!("SVWZ+Testbuchung {i}"),
                )
            })
            .collect(),
    );
    // A depot as some banks list it: no IBAN, no Kontoart, only HKWPD.
    let mut depot = testbank::Account::depot(&m.blz, "0007000999", "Testkundin Eins", MT535);
    depot.iban = None;
    depot.kind = None;
    m.accounts.push(depot);
    m.job_mut("HKSAL").hipins_needs_tan = true;
    let kaz = m.job_mut("HKKAZ");
    kaz.hipins_needs_tan = true;
    kaz.page_size = Some(1);
    m
}

pub fn synthetic() -> Trace {
    Trace::from_json(SYNTHETIC).expect("fixture loads")
}

/// Every HKSAL, HKKAZ and HKWPD exchange of a recording of [`model`] names
/// its account: the giro as `#1`, the depot as `#2`.
pub fn assert_account_notes(trace: &Trace) {
    let described: Vec<String> = trace.accounts.iter().map(|a| a.to_string()).collect();
    assert_eq!(
        described,
        [
            "account #1 (class checking; Kontoart 1; jobs HKSAL HKKAZ)",
            "account #2 (class depot; Kontoart -; jobs HKWPD)",
        ]
    );
    let mut jobs = Vec::new();
    for s in trace.summary() {
        let Some(job) = s.jobs.iter().find_map(|j| {
            ["HKSAL", "HKKAZ", "HKWPD"]
                .into_iter()
                .find(|c| j.starts_with(&format!("{c}:")))
        }) else {
            continue;
        };
        let expected = if job == "HKWPD" { "#2" } else { "#1" };
        assert_eq!(s.account.as_deref(), Some(expected), "{s}");
        jobs.push(job);
    }
    for job in ["HKSAL", "HKKAZ", "HKWPD"] {
        assert!(jobs.contains(&job), "no {job} in {jobs:?}");
    }
}
