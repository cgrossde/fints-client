//! End-to-end test through the real HTTP transport.
//!
//! The synthetic test bank (`fints-client-testbank`) plays the bank on
//! 127.0.0.1 (random port). It decodes the base64 FinTS message of every
//! POST (PIN/TAN transport format, FinTS Security "Sicherheitsverfahren
//! PIN/TAN" B.1), reads the request with the codec, checks the message
//! rules, records it and answers from its model.
//!
//! The test drives the library as a caller does: `jobs::open`, `balance`,
//! `transactions`, `Dialog::end`. It covers dialog initialisation with
//! synchronisation (HISYN, HIBPA, HIUPA, HIUPD, HITANS, HIPINS), a balance
//! request (HISAL), a transaction request (HIKAZ with a synthetic MT940) and
//! dialog end, and checks the logs of the whole dialog for identifiers.
//!
//! All data is synthetic: placeholder IBAN, invented IDs, no bank contact.

use std::sync::{Arc, Mutex};

use fints_client::codec::defs::{HkIdn2, HkKaz7, HkSal7, HnSha2};
use fints_client::jobs::{self, Credentials, Format, Range, Step};
use fints_client::{Pin, ProductId, SessionState, UserId};
use fints_client_testbank::{
    Account, BankModel, Booking, Customer, JobSpec, TanMethod, TestBank, date,
};

const BLZ: &str = "37040044";
const BIC: &str = "COBADEFFXXX";
const IBAN: &str = "DE89370400440532013000";
const ACCOUNT_NUMBER: &str = "0532013000";
const USER: &str = "kunde4711x";
const PIN: &str = "geheim-pin-4711";
/// The first system ID the test bank issues.
const SYSTEM_ID: &str = "TESTSYS0001";
const HOLDER: &str = "Erika Beispiel";

// ── The bank ────────────────────────────────────────────────────────────────

/// One Girokonto with an opening balance of 1000.00, one debit, one credit
/// and one pending debit; a typed TAN method; login exempt from SCA;
/// HKSAL 7 and HKKAZ 7 without TAN.
fn model() -> BankModel {
    let mut account = Account::giro(BLZ, ACCOUNT_NUMBER, HOLDER).with_bookings(
        "1000.00".parse().unwrap(),
        vec![
            Booking::new(date(2026, 1, 2), "-12.34", "SVWZ+Beispiel Rechnung 1")
                .posting_text("SEPA-UEBERWEISUNG")
                .counterparty("Max Mustermann"),
            Booking::new(date(2026, 1, 5), "246.90", "SVWZ+Beispiel Gehalt")
                .posting_text("SEPA-GUTSCHRIFT")
                .counterparty("Beispiel GmbH"),
        ],
    );
    account.pending = vec![Booking::new(date(2026, 1, 6), "-10.5", "Vorgemerkt")];
    let mut m = BankModel {
        blz: BLZ.into(),
        bic: BIC.into(),
        name: "Testbank".into(),
        customer: Customer {
            user_id: USER.into(),
            pin: PIN.into(),
            customer_id: USER.into(),
            changed_user_id: None,
        },
        accounts: vec![account],
        tan_methods: vec![TanMethod::typed(921, "Testverfahren Alpha", "123456")],
        bpd_version: 42,
        upd_version: 3,
        ..BankModel::default()
    };
    m.jobs.clear();
    m.jobs.insert("HKSAL", JobSpec::new(&[7]));
    m.jobs.insert("HKKAZ", JobSpec::new(&[7]));
    m
}

// ── Capturing the logs ──────────────────────────────────────────────────────

/// A `tracing` subscriber that keeps every event (all levels) as text.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={:?} ", field.name(), value));
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(format!(
            "{} {}: {}",
            event.metadata().level(),
            event.metadata().target(),
            fields.0
        ));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

// ── The test ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dialog_over_http_returns_balance_and_transactions_and_logs_no_identifiers() {
    let logs = Capture::default();
    let _guard = tracing::subscriber::set_default(logs.clone());

    let bank = TestBank::new(model()).serve().await;

    // ── As a caller: open, list accounts, fetch, end ──
    let client = bank.client(&ProductId::new("TestProdukt"));
    let creds = Credentials::new(UserId::new(USER), Pin::new(PIN));
    let state = SessionState::new(&fints_client::Blz::new(BLZ), &UserId::new(USER));
    let (mut dialog, step) = jobs::open(&client, &creds, &state).await.expect("open");
    let Step::Done(opened) = step else {
        panic!("3076: no strong authentication");
    };

    let accounts = opened.value.accounts;
    assert_eq!(accounts.len(), 1, "UPD lists one account");
    assert_eq!(accounts[0].iban.as_ref().map(|i| i.as_str()), Some(IBAN));
    assert_eq!(accounts[0].number.as_deref(), Some(ACCOUNT_NUMBER));

    let account = fints_client::protocol::Account::new(IBAN, BIC).unwrap();
    let Step::Done(balance) = jobs::balance(&mut dialog, &account).await.expect("balance") else {
        panic!("HKSAL needs no TAN");
    };
    let today = jiff::ZonedDateTime::now().date();
    let range = Range::new(today - jiff::Span::new().days(30), today);
    let Step::Done(statement) = jobs::transactions(&mut dialog, &account, range, Format::Mt940)
        .await
        .expect("transactions")
    else {
        panic!("HKKAZ needs no TAN");
    };
    let session = dialog.end().await.expect("end");
    assert_eq!(session.system_id.as_str(), SYSTEM_ID, "HISYN is stored");

    // ── What the caller got ──
    let balance = balance.value.expect("balance");
    assert_eq!(balance.amount.to_string(), "1234.56");
    assert_eq!(balance.currency.as_str(), "EUR");
    assert_eq!(balance.date.unwrap().to_string(), "2026-01-05");
    assert_eq!(
        balance.pending_amount.map(|d| d.to_string()).as_deref(),
        Some("-10.5")
    );
    assert_eq!(statement.value.booked.len(), 2);
    let debit = &statement.value.booked[0];
    assert_eq!(debit.amount.to_string(), "-12.34");
    assert_eq!(debit.date.to_string(), "2026-01-02");
    assert_eq!(debit.posting_text.as_deref(), Some("SEPA-UEBERWEISUNG"));
    assert_eq!(debit.purpose.as_deref(), Some("Beispiel Rechnung 1"));
    let credit = &statement.value.booked[1];
    assert_eq!(credit.amount.to_string(), "246.90");
    assert_eq!(credit.date.to_string(), "2026-01-05");

    // ── What the bank received ──
    // Every segment of every request is one the codec reads, and the
    // client kept the message rules (numbering, envelope, dialog IDs).
    bank.assert_clean();
    let received = bank.requests();
    let sequence: Vec<(String, u32, Vec<&str>)> = received
        .iter()
        .map(|r| (r.dialog_id.clone(), r.message_number, r.codes()))
        .collect();
    let dump = format!("{sequence:#?}");
    assert!(
        received
            .iter()
            .all(|r| r.http_content_type.as_deref() == Some("text/plain")),
        "{dump}"
    );
    for r in &received {
        let unread: Vec<_> = r.segments.iter().filter(|s| s.error().is_some()).collect();
        assert!(unread.is_empty(), "codec cannot read {unread:?}: {dump}");
    }

    // Dialog 1: synchronisation (system ID 0) and end. Dialog 2: business.
    let codes: Vec<Vec<&str>> = received.iter().map(|r| r.codes()).collect();
    assert_eq!(
        codes[0],
        ["HNSHK", "HKIDN", "HKVVB", "HKSYN", "HNSHA"],
        "{dump}"
    );
    assert_eq!(codes[1], ["HNSHK", "HKEND", "HNSHA"], "{dump}");
    assert_eq!(codes[2][..3], ["HNSHK", "HKIDN", "HKVVB"], "{dump}");
    assert!(codes[2].contains(&"HKTAN"), "{dump}");
    let business: Vec<&str> = codes[3..codes.len() - 1]
        .iter()
        .flatten()
        .copied()
        .collect();
    assert!(
        business.contains(&"HKSAL") && business.contains(&"HKKAZ"),
        "{dump}"
    );
    assert_eq!(codes.last().unwrap()[1], "HKEND", "{dump}");

    // Dialog and message numbering.
    assert_eq!((&*sequence[0].0, sequence[0].1), ("0", 1));
    assert_eq!((&*sequence[1].0, sequence[1].1), ("TESTDLG0001", 2));
    assert_eq!((&*sequence[2].0, sequence[2].1), ("0", 1));
    for (i, (dialog, number, _)) in sequence[2..].iter().enumerate() {
        if i > 0 {
            assert_eq!(dialog, "TESTDLG0002", "{dump}");
        }
        assert_eq!(*number as usize, i + 1, "{dump}");
    }

    // Credentials and IDs in the requests.
    let first = &received[0];
    let idn: HkIdn2 = first.get().expect("HKIDN");
    assert_eq!(idn.customer_id.as_str(), USER);
    assert_eq!(idn.bank.bank_code.as_ref().unwrap().as_str(), BLZ);
    assert_eq!(idn.system_id.as_str(), "0", "no system ID before the sync");
    let sha: HnSha2 = first.get().expect("HNSHA");
    assert_eq!(sha.user_signature.as_ref().unwrap().pin.as_str(), PIN);
    let second_idn: HkIdn2 = received[2].get().expect("HKIDN of dialog 2");
    assert_eq!(
        second_idn.system_id.as_str(),
        SYSTEM_ID,
        "system ID from HISYN"
    );
    let sal: HkSal7 = received.iter().find_map(|r| r.get()).expect("HKSAL 7");
    assert_eq!(sal.account.iban.as_ref().unwrap().as_str(), IBAN);
    let kaz: HkKaz7 = received.iter().find_map(|r| r.get()).expect("HKKAZ 7");
    assert_eq!(kaz.account.iban.as_ref().unwrap().as_str(), IBAN);

    // ── The logs of the whole dialog name no credential or identifier ──
    let logged = logs.0.lock().unwrap().join("\n");
    assert!(
        logged.contains("Request (masked)"),
        "trace of requests missing"
    );
    assert!(
        logged.contains("Response (masked)"),
        "trace of responses missing"
    );
    assert!(
        logged.contains("HKIDN:"),
        "masked trace still shows segments"
    );
    assert!(
        !logged.contains("Quirk"),
        "synthetic responses need no quirk:\n{logged}"
    );
    for secret in [USER, PIN, SYSTEM_ID, IBAN, ACCOUNT_NUMBER, HOLDER] {
        assert!(
            !logged.contains(secret),
            "{secret} is in the logs:\n{logged}"
        );
    }
}
