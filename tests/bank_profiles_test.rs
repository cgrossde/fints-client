//! Evidence test: a test bank profile's parameter data agrees with the
//! live trace it was written from (`testbank/fixtures/replay/<bank>.trace.json`).
//! Parameter data (BPD, UPD job lists) is not masked in a trace. Skipped
//! when the trace is absent.

use std::collections::BTreeMap;
use std::path::PathBuf;

use fints_client::trace::Trace;
use fints_client_testbank::{BankModel, profiles};
use serde_json::Value;

/// Splits at an unescaped `sep` (`?` escapes).
fn split(s: &str, sep: char) -> Vec<String> {
    let (mut out, mut cur, mut esc) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '?' {
            esc = true;
            cur.push(c);
        } else if c == sep {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// The data elements of a segment string (first = header).
fn des(segment: &str) -> Vec<String> {
    split(segment.trim_end_matches('\''), '+')
}

/// The segments of the first exchange's answer (BPD) whose code is `code`.
fn segments(trace_json: &Value, exchange: usize, code: &str) -> Vec<Vec<String>> {
    trace_json["exchanges"][exchange]["response"]
        .as_array()
        .expect("response array")
        .iter()
        .filter_map(|s| s.as_str())
        .filter(|s| s.starts_with(&format!("{code}:")))
        .map(des)
        .collect()
}

fn version(header: &str) -> u16 {
    header.split(':').nth(2).expect("version").parse().unwrap()
}

fn check_profile(name: &str, model: &BankModel, trace_path: PathBuf) {
    let Ok(text) = std::fs::read_to_string(&trace_path) else {
        eprintln!("skip {name}: no trace at {}", trace_path.display());
        return;
    };
    let trace = Trace::from_json(&text).expect("trace loads");
    let json: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(trace.blz().as_deref(), Some(model.blz.as_str()), "BLZ");
    assert_eq!(trace.bic().as_deref(), Some(model.bic.as_str()), "BIC");

    // The first exchange that carries the BPD.
    let bpd = (0..trace.exchanges.len())
        .find(|&i| !segments(&json, i, "HIPINS").is_empty())
        .expect("a BPD in the trace");

    // HIPINS: flag per job. DE 4 is `:::::JOB:flag:JOB:flag...` (five
    // leading parameters).
    let pins = &segments(&json, bpd, "HIPINS")[0];
    let parts = split(&pins[4], ':');
    let flags: BTreeMap<&str, bool> = parts[5..]
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| (c[0].as_str(), c[1] == "J"))
        .collect();
    for (job, spec) in &model.jobs {
        assert_eq!(
            flags.get(job).copied(),
            Some(spec.hipins_needs_tan),
            "{name}: HIPINS flag of {job}"
        );
    }

    // Job versions: the model's must be offered by the bank, and the
    // highest must be the bank's highest.
    for (job, parameter) in [
        ("HKSAL", "HISALS"),
        ("HKKAZ", "HIKAZS"),
        ("HKWPD", "HIWPDS"),
    ] {
        let offered: Vec<u16> = segments(&json, bpd, parameter)
            .iter()
            .map(|s| version(&s[0]))
            .collect();
        let ours = &model.jobs[job].versions;
        assert!(
            ours.iter().all(|v| offered.contains(v)),
            "{name}: {job} versions {ours:?} not all offered {offered:?}"
        );
        assert_eq!(ours.iter().max(), offered.iter().max(), "{name}: {job} top");
    }

    // HITANS: version and methods (function, decoupled or not, name). A
    // bank may send several versions; the model is the highest.
    let tans = segments(&json, bpd, "HITANS")
        .into_iter()
        .max_by_key(|s| version(&s[0]))
        .expect("a HITANS");
    let tans = &tans;
    assert_eq!(version(&tans[0]), model.hitans_version, "{name}: HITANS");
    let params = split(&tans[4], ':');
    // A method starts `function:2:technical ID:DK method:`.
    for m in &model.tan_methods {
        let f = m.function.to_string();
        let at = params
            .iter()
            .position(|p| *p == f)
            .unwrap_or_else(|| panic!("{name}: HITANS has no method {f}"));
        let decoupled = params[at + 3] == "Decoupled";
        assert_eq!(decoupled, m.is_decoupled(), "{name}: method {f} kind");
        assert!(
            params.contains(&m.name),
            "{name}: method {f} name {:?}",
            m.name
        );
    }
    let offered_functions = params
        .iter()
        .enumerate()
        .filter(|(i, p)| params.get(i + 1).is_some_and(|n| n == "2") && p.len() == 3)
        .count();
    assert_eq!(
        offered_functions,
        model.tan_methods.len(),
        "{name}: methods"
    );

    // HIUPA: the UPD version (0 = dialog-only).
    let upa = (0..trace.exchanges.len())
        .find_map(|i| segments(&json, i, "HIUPA").into_iter().next())
        .expect("a HIUPA in the trace");
    assert_eq!(
        upa[2].parse::<u32>().unwrap(),
        model.upd_version,
        "{name}: UPD version"
    );

    // HISPAS: the national flag is the second of `J:J:J`.
    let national = segments(&json, bpd, "HISPAS")
        .first()
        .map(|s| split(&s[4], ':')[1] == "J");
    assert_eq!(national, model.hispas_national, "{name}: HISPAS national");

    // UPD: allowed jobs and Kontoart presence per account. The depot is
    // paired with the trace's depot and the other accounts with the trace's
    // other accounts in order (the bank lists the depot among them; the
    // profile has fewer accounts than the trace).
    let is_depot = |class: Option<&str>| class == Some("depot");
    let mut others = trace
        .accounts
        .iter()
        .filter(|a| !is_depot(a.class.as_deref()));
    let depot = trace.accounts.iter().find(|a| is_depot(a.class.as_deref()));
    for (i, ours) in model.accounts.iter().enumerate() {
        let ours_depot = ours.kind.is_none() && ours.allowed_jobs.contains(&"HKWPD");
        let theirs = if ours_depot {
            depot.expect("trace has a depot")
        } else {
            others.next().expect("trace has enough accounts")
        };
        let mut a: Vec<&str> = ours.allowed_jobs.to_vec();
        let mut b: Vec<&str> = theirs.jobs.iter().map(String::as_str).collect();
        a.sort();
        b.sort();
        assert_eq!(
            a,
            b,
            "{name}: account {} allowed jobs vs {}",
            i + 1,
            theirs.alias
        );
        assert_eq!(
            ours.kind.is_some(),
            theirs.kontoart.is_some(),
            "{name}: account {} Kontoart presence vs {}",
            i + 1,
            theirs.alias
        );
        // Only the depot and a credit card (Kontoart 50) lack an IBAN.
        assert_eq!(
            ours.iban.is_some(),
            !ours_depot && ours.kind != Some(50),
            "{name}: only a depot and a card lack an IBAN"
        );
    }
}

/// The read trace of bank code `blz` in `dir`: `<bank>-<BLZ>.trace.json`,
/// as `scripts/try-bank.sh` names it (`<BLZ>.trace.json` for a bank without
/// a name). A path that does not exist when there is none.
fn trace_of(dir: &std::path::Path, blz: &str) -> PathBuf {
    let plain = format!("{blz}.trace.json");
    let named = format!("-{plain}");
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == plain || n.ends_with(&named))
        })
        .unwrap_or_else(|| dir.join(plain))
}

#[test]
fn profiles_agree_with_their_traces() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testbank/fixtures/replay");
    // Each model is compared with the committed trace of its bank code
    // and, under `REPLAY_LOCAL=1`, with a newer recording in `local/`.
    let local = std::env::var_os("REPLAY_LOCAL").is_some_and(|v| v == "1");
    for (name, build) in profiles::all() {
        let model = build();
        check_profile(name, &model, trace_of(&dir, &model.blz));
        if local {
            check_profile(name, &model, trace_of(&dir.join("local"), &model.blz));
        }
    }
}

// ── Behaviour against the profile ───────────────────────────────────────
//
// In-process only: a profile never reaches the network. The test bank logs
// requests, not its answers, so "answered 9075" is read from a request that
// carries no HKTAN (the model answers exactly that with 9075) and "no 9010"
// from the fetch results plus the shape of the KTI.

use fints_client_testbank::TestBank;
use fints_client_testbank::replay::{RunError, RunOptions, standard_run};

async fn run_profile(
    model: BankModel,
) -> (
    fints_client_testbank::BankHandle,
    Result<fints_client_testbank::replay::RunReport, RunError>,
) {
    use fints_client::{Pin, ProductId, UserId};
    let bank = TestBank::new(model).handle();
    let m = bank.model();
    let report = standard_run(
        &bank.client(&ProductId::new("TEST-123")),
        &UserId::new(&m.customer.user_id),
        &Pin::new(&m.customer.pin),
        &RunOptions {
            first_wait: std::time::Duration::ZERO,
            next_wait: std::time::Duration::ZERO,
            ..RunOptions::new(30)
        },
    )
    .await;
    (bank, report)
}

/// The DKB model's camt under a bank code without a profile: the entry
/// references stay `bank_reference`, no booking gets a `booking_id`.
#[tokio::test]
async fn camt_references_without_the_profile_rule_give_no_booking_id() {
    let mut model = profiles::dkb();
    model.blz = "10000000".into();
    let (bank, report) = run_profile(model).await;
    let report = report.expect("run");
    let client = bank.client(&fints_client::ProductId::new("TEST-123"));
    assert_eq!(client.config().profile.id, ProfileId::Generic);
    let txs = &report.fetched[0].as_ref().unwrap().transactions;
    assert_eq!(txs.len(), 2);
    assert_eq!(
        txs[0].bank_reference.as_deref(),
        Some("TESTREF00000000000000000201")
    );
    assert!(txs.iter().all(|t| t.booking_id.is_none()));
}

/// Requests carrying `code`, in order.
fn requests_with(
    bank: &fints_client_testbank::BankHandle,
    code: &str,
) -> Vec<fints_client_testbank::Request> {
    bank.requests()
        .into_iter()
        .filter(|r| r.has(code))
        .collect()
}

/// Live 2026-11-16 (`testbank/fixtures/replay/consorsbank-76030080.trace.json`): the
/// depot gets HKWPD only, without HKTAN; HKSAL and HKKAZ are answered 9075
/// once and then sent with HKTAN from the start; the 6-part KTI is accepted.
#[tokio::test]
async fn consorsbank_standard_run_learns_hktan_and_skips_the_depot() {
    let (bank, report) = run_profile(profiles::consorsbank()).await;
    let report = report.expect("run");
    assert_eq!(report.fetched.len(), 4);
    assert!(report.errors().is_empty(), "{:?}", report.errors());

    // UPD version 0 (dialog-only): the run got the accounts, the client
    // stores no UPD.
    let m = bank.model();
    let other = TestBank::new(profiles::consorsbank()).handle();
    let product = fints_client::ProductId::new("TEST-123");
    let creds = fints_client::jobs::Credentials::new(
        fints_client::UserId::new(&m.customer.user_id),
        fints_client::Pin::new(&m.customer.pin),
    );
    let fresh = fints_client::SessionState::new(
        &fints_client::Blz::new(&m.blz),
        &fints_client::UserId::new(&m.customer.user_id),
    );
    let (dialog, step) = fints_client::jobs::open(&other.client(&product), &creds, &fresh)
        .await
        .expect("login");
    let fints_client::jobs::Step::Done(opened) = step else {
        panic!("the profile's login needs no TAN");
    };
    assert_eq!(
        opened.value.accounts.len(),
        4,
        "accounts from the dialog-only UPD"
    );
    let state = dialog.end().await.expect("end");
    assert!(state.upd().is_none(), "no UPD stored");
    assert_eq!(state.upd_version, 0);

    let depot_number = "0000000900";
    for code in ["HKSAL", "HKKAZ"] {
        let reqs = requests_with(&bank, code);
        assert!(
            reqs.iter().all(|r| !r.text().contains(depot_number)),
            "{code} names the depot"
        );
        // Three accounts: the first request has no HKTAN (answered 9075),
        // its retry and every later request carry it.
        let without: Vec<_> = reqs.iter().filter(|r| !r.has("HKTAN")).collect();
        assert_eq!(without.len(), 1, "{code}: 9075 exactly once");
        assert_eq!(without[0].index, reqs[0].index, "{code}: the first one");
        assert!(reqs.len() >= 4, "{code}: retry plus two learned");
        assert!(reqs[1..].iter().all(|r| r.has("HKTAN")), "{code}");
    }

    let wpd = requests_with(&bank, "HKWPD");
    assert!(!wpd.is_empty());
    assert!(
        wpd.iter().all(|r| !r.has("HKTAN")),
        "HKWPD carries no HKTAN"
    );

    for r in requests_with(&bank, "HKKAZ") {
        assert!(r.text().contains(":280:76030080"), "6-part KTI");
    }
    bank.assert_clean();
}

/// Live (earlier, `docs/banks.md`; same bank profile as the 2026-11-16
/// trace): with HISPAS "national account not allowed" the client sends
/// `IBAN:BIC` and the bank answers HKKAZ with 9010.
#[tokio::test]
async fn consorsbank_without_hispas_national_gets_9010_on_hkkaz() {
    let model = BankModel {
        hispas_national: Some(false),
        ..profiles::consorsbank()
    };
    let (bank, report) = run_profile(model).await;
    let report = report.expect("run");
    let kinds: Vec<_> = report
        .fetched
        .iter()
        .map(|r| match r.as_ref().unwrap().errors.first() {
            Some((_, RunError::Fints(fints_client::FinTSError::Bank(e)))) => Some(e.code().kind()),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds[0],
        Some(fints_client::return_code::Kind::GeneralError),
        "{kinds:?}"
    );
    for r in requests_with(&bank, "HKKAZ") {
        assert!(!r.text().contains(":280:"), "IBAN:BIC only");
    }
    bank.assert_clean();
}

// ── DKB and comdirect (live 2026-11-17) ─────────────────────────────────
//
// These tests tell the same story as the summaries (`trace_summary`) of the
// banks' traces, against the models.

use fints_client::banks::{ProfileId, bank_by_blz};
use fints_client::{AccountClass, ClassSource, FinTSError};

/// The orders the client sent, in order: code, version, whether HKTAN
/// went with it.
fn orders(bank: &fints_client_testbank::BankHandle) -> Vec<(String, u16, bool)> {
    bank.requests()
        .iter()
        .flat_map(|r| {
            let tan = r.has("HKTAN");
            r.typed()
                .filter(|t| matches!(t.code(), "HKSAL" | "HKKAZ" | "HKCAZ" | "HKWPD"))
                .map(|t| (t.code().to_string(), t.version(), tan))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The profile of the bank code in the directory is the one the client of
/// the model's handle gets.
fn assert_profile(blz: &str, id: ProfileId, bank: &fints_client_testbank::BankHandle) {
    let config = bank_by_blz(blz).expect("in the directory");
    assert_eq!(config.profile.id, id, "{blz}");
    assert_eq!(
        config.with_url("in-process://testbank").profile.id,
        id,
        "with_url keeps the profile"
    );
    let client = bank.client(&fints_client::ProductId::new("TEST-123"));
    assert_eq!(client.config().profile.id, id, "the handle's client");
}

/// Live 2026-11-17 (DKB): decoupled login (3955, 3956 while pending, then
/// confirmed), then per account HKSAL 8 and HKCAZ 1, each with HKTAN and
/// answered 3076. The credit card has no IBAN: its HKSAL and HKCAZ carry the
/// national-only KTI (HISPAS allows national data; the card is not in the
/// live trace, so this is unverified at the bank).
#[tokio::test]
async fn dkb_standard_run_sends_hksal_8_and_hkcaz_1_per_account() {
    let (bank, report) = run_profile(profiles::dkb()).await;
    let report = report.expect("run");
    assert_profile("12030000", ProfileId::Dkb, &bank);

    // Four current accounts (Kontoart 1), then the card (50).
    let classes: Vec<_> = report.accounts.iter().map(|a| a.class).collect();
    assert_eq!(
        classes[..4],
        [Some((AccountClass::Checking, ClassSource::Kontoart)); 4]
    );
    assert_eq!(
        classes[4],
        Some((AccountClass::CreditCard, ClassSource::Kontoart))
    );

    // Orders: HKSAL 8 and HKCAZ 1 for each of the five accounts,
    // always with HKTAN (HIPINS `J`), so never a 9075.
    let expected: Vec<_> = (0..5)
        .flat_map(|_| {
            [
                ("HKSAL".to_string(), 8, true),
                ("HKCAZ".to_string(), 1, true),
            ]
        })
        .collect();
    assert_eq!(orders(&bank), expected);

    // The BPD lists HITANS 6 (910, 913) and 7 (940, 910, 913): 940 is only
    // in 7, so HKTAN goes in version 7.
    assert!(
        bank.requests()
            .iter()
            .any(|r| r.text().contains("HKTAN:") && r.text().contains(":7+")),
        "HKTAN 7"
    );
    assert!(
        bank.requests()
            .iter()
            .all(|r| !r.text().contains("HKTAN:4:6")),
        "no HKTAN 6"
    );

    // Login: HKTAN for HKIDN, then two status requests (3956, then 3920 +
    // 0900), none of them an order.
    let status = bank
        .requests()
        .iter()
        .filter(|r| r.has("HKTAN") && !r.has("HKIDN") && orders_in(r) == 0)
        .count();
    assert_eq!(status, 2, "decoupled: one pending status, one confirmed");

    // The four current accounts ran without error; the first has bookings
    // (camt).
    assert_eq!(report.fetched.len(), 5);
    for run in &report.fetched[..4] {
        let run = run.as_ref().expect("addressable");
        assert!(run.errors.is_empty(), "{:?}", run.errors);
        assert!(run.balance.is_some());
    }
    assert_eq!(report.fetched[0].as_ref().unwrap().transactions.len(), 2);
    // The profile's booking ID: the entry reference of each booked camt
    // entry (docs/banks/dkb.md, Transactions).
    let ids: Vec<_> = report.fetched[0]
        .as_ref()
        .unwrap()
        .transactions
        .iter()
        .map(|t| t.booking_id.as_deref())
        .collect();
    assert_eq!(
        ids,
        [
            Some("TESTREF00000000000000000201"),
            Some("TESTREF00000000000000000202")
        ]
    );

    // The card: both orders went out with the national-only KTI.
    let card = report.fetched[4].as_ref().expect("the run goes on");
    assert!(card.errors.is_empty(), "{:?}", card.errors);
    assert!(card.balance.is_some());
    let card_orders: Vec<String> = bank
        .requests()
        .iter()
        .flat_map(|r| r.text().split('\'').map(str::to_string).collect::<Vec<_>>())
        .filter(|seg| seg.contains("5000000000000001"))
        .collect();
    assert_eq!(card_orders.len(), 2, "{card_orders:?}");
    assert!(
        card_orders[0].starts_with("HKSAL:")
            && card_orders[0].contains("+::5000000000000001::280:12030000+"),
        "{card_orders:?}"
    );
    assert!(
        card_orders[1].starts_with("HKCAZ:")
            && card_orders[1].contains("+::5000000000000001::280:12030000+"),
        "{card_orders:?}"
    );
    bank.assert_clean();
}

fn orders_in(r: &fints_client_testbank::Request) -> usize {
    r.typed()
        .filter(|t| matches!(t.code(), "HKSAL" | "HKKAZ" | "HKCAZ" | "HKWPD"))
        .count()
}

/// Live 2026-11-17 (comdirect): the clearing account gets HKSAL 5 and HKCAZ
/// 1 with HKTAN (3076); the depot, which has no Kontoart and allows HKWPD
/// but not HKSAL, HKKAZ or HKCAZ, gets HKWPD 5 without HKTAN.
#[tokio::test]
async fn comdirect_standard_run_takes_the_depot_from_its_allowed_jobs() {
    let (bank, report) = run_profile(profiles::comdirect()).await;
    let report = report.expect("run");
    assert_profile("20041155", ProfileId::Comdirect, &bank);

    assert_eq!(
        orders(&bank),
        [
            ("HKSAL".to_string(), 5, true),
            ("HKCAZ".to_string(), 1, true),
            ("HKWPD".to_string(), 5, false),
        ]
    );

    // No Kontoart on either account.
    assert!(report.accounts.iter().all(|a| a.account_type.is_none()));
    assert_eq!(report.accounts[0].class, None);
    assert_eq!(
        report.accounts[1].class,
        Some((AccountClass::Depot, ClassSource::AllowedJobs))
    );
    assert_eq!(report.skipped[1], ["HKSAL", "HKKAZ"]);

    assert!(report.errors().is_empty(), "{:?}", report.errors());
    let clearing = report.fetched[0].as_ref().unwrap();
    assert!(clearing.balance.is_some());
    assert_eq!(clearing.transactions.len(), 2);
    let depot = report.fetched[1].as_ref().unwrap();
    assert_eq!(depot.holdings.len(), 1);
    assert!(depot.balance.is_none());

    // HISPAS says `N`: HKCAZ names the account by IBAN and BIC only.
    for r in bank.requests().iter().filter(|r| r.has("HKCAZ")) {
        assert!(!r.text().contains(":280:"), "{}", r.text());
    }
    bank.assert_clean();
}

/// Live 2026-11-17 (comdirect): a bank code that does not match the user
/// gets one HIRMG with 9050, 9800, 9210 and 9340; the client reports 9210.
#[tokio::test]
async fn comdirect_wrong_bank_code_is_reported_as_9210() {
    use fints_client::jobs::{self, Credentials};
    let bank = TestBank::new(profiles::comdirect()).handle();
    let m = bank.model();
    // Another code of the same host: the directory gives it the same profile.
    let config = bank_by_blz("20041111")
        .expect("in the directory")
        .with_url("in-process://testbank");
    assert_eq!(config.profile.id, ProfileId::Comdirect);
    let client =
        fints_client::jobs::BankClient::new(config, fints_client::ProductId::new("TEST-123"))
            .with_transport(std::sync::Arc::new(bank.clone()));
    let creds = Credentials::new(
        fints_client::UserId::new(&m.customer.user_id),
        fints_client::Pin::new(&m.customer.pin),
    );
    let state = fints_client::SessionState::new(
        &fints_client::Blz::new("20041111"),
        &fints_client::UserId::new(&m.customer.user_id),
    );
    let Err(err) = jobs::open(&client, &creds, &state).await else {
        panic!("the login must be refused");
    };
    let FinTSError::Bank(e) = &err else {
        panic!("a bank error, got {err:?}");
    };
    assert_eq!(e.code().code.to_string(), "9210", "{err}");
    assert!(orders(&bank).is_empty());
    bank.assert_clean();
}
