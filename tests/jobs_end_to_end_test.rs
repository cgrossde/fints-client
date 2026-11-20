//! `jobs` end to end: open, balance, transactions, holdings and session
//! state against the synthetic test bank (`fints-client-testbank`) over
//! loopback HTTP, plus account validation and result serialisation.

use fints_client::jobs::{self, Credentials, Format, Range, Step};
use fints_client::protocol::Account;
use fints_client::{Bic, Pin, ProductId, SessionState, UserId};
use fints_client_testbank::{self as testbank, BankModel, Booking, HttpBank, TestBank, date};
use rust_decimal::Decimal;

// ═══════════════════════════════════════════════════════════════════════════
// Pure unit tests (no network)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn account_validation_requires_iban_and_bic() {
    assert!(Account::new("DE92123456780007000111", "TESTDEFFXXX").is_ok());
    assert!(Account::new("", "TESTDEFFXXX").is_err());
    assert!(Account::new("DE92123456780007000111", "").is_err());
}

#[test]
fn transaction_and_balance_serde_roundtrip() {
    use fints_client::{AccountBalance, Bic, Currency, Iban, Transaction, TransactionStatus};
    use jiff::civil::Date;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    let txn = Transaction {
        date: Date::new(2025, 1, 15).unwrap(),
        valuta_date: None,
        amount: Decimal::from_str("182.34").unwrap(),
        currency: Currency::new("EUR"),
        applicant_name: Some("Acme".to_string()),
        applicant_iban: Some(Iban::new("DE92123456780007000111")),
        applicant_bic: Some(Bic::new("TESTDEFFXXX")),
        purpose: Some("Rechnung".to_string()),
        posting_text: None,
        customer_reference: Some("REF-123".to_string()),
        bank_reference: None,
        booking_id: None,
        gvc: Some("166".to_string()),
        prima_nota: None,
        purpose_code: None,
        mandate_id: None,
        end_to_end_id: None,
        creditor_id: None,
        debtor_id: None,
        ultimate_debtor: None,
        ultimate_creditor: None,
        reversal: false,
        original_amount: None,
        fees: None,
        return_reason: None,
        sequence_type: None,
        raw: serde_json::json!({"date": "2025-01-15"}),
        status: TransactionStatus::Booked,
    };

    let balance = AccountBalance {
        amount: Decimal::from_str("948.27").unwrap(),
        date: Some(Date::new(2025, 1, 15).unwrap()),
        currency: Currency::new("EUR"),
        credit_line: None,
        available: None,
        pending_amount: None,
        pending_date: None,
    };

    let json = serde_json::to_string(&(&txn, &balance)).expect("serialize");
    let (back_txn, back_balance): (Transaction, AccountBalance) =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        back_txn.applicant_iban.unwrap().as_str(),
        "DE92123456780007000111"
    );
    assert_eq!(back_txn.amount.to_string(), "182.34");
    assert_eq!(back_balance.amount.to_string(), "948.27");
    assert_eq!(back_balance.currency.as_str(), "EUR");
}

#[test]
fn currency_conversion_and_display() {
    use fints_client::Currency;
    let eur = Currency::new("EUR");
    assert_eq!(eur.as_str(), "EUR");
    assert_eq!(eur.to_string(), "EUR");
}

// ═══════════════════════════════════════════════════════════════════════════
// End-to-end tests (jobs → synthetic test bank)
// ═══════════════════════════════════════════════════════════════════════════

const BLZ: &str = "12345678";
const BIC: &str = "TESTDEFFXXX";
const USER: &str = "flowuser7";
const PIN: &str = "flow-pin-7";
const MAIN_NUMBER: &str = "0007000111";
const SECOND_NUMBER: &str = "0007000222";

fn creds() -> Credentials {
    Credentials::new(UserId::new(USER), Pin::new(PIN))
}

fn fresh_state() -> SessionState {
    SessionState::new(&fints_client::Blz::new(BLZ), &UserId::new(USER))
}

/// Open a dialog; the login is exempt from SCA.
async fn open(bank: &HttpBank, state: &SessionState) -> (jobs::Dialog, jobs::Opened) {
    let client = bank.client(&ProductId::new("TEST-123"));
    let (dialog, step) = jobs::open(&client, &creds(), state).await.expect("open");
    match step {
        Step::Done(done) => (dialog, done.value),
        other => panic!("the login is SCA-exempt, got {other:?}"),
    }
}

fn done<T: std::fmt::Debug>(step: Step<T>) -> T {
    match step {
        Step::Done(d) => d.value,
        other => panic!("expected Done, got {other:?}"),
    }
}

fn year() -> Range {
    Range::new(date(2026, 1, 1), date(2026, 12, 31))
}

/// Two current accounts: the main one with four bookings (balance
/// 2087.15), the second with one (balance 312.80). SCA exemption on login,
/// BPD version 31, UPD version 7.
fn model() -> BankModel {
    let mut m = BankModel::default();
    m.customer.user_id = USER.into();
    m.customer.pin = PIN.into();
    m.customer.customer_id = USER.into();
    m.bpd_version = 31;
    m.upd_version = 7;
    let main = testbank::Account::giro(BLZ, MAIN_NUMBER, "Testperson Flow").with_bookings(
        Decimal::new(150_000, 2),
        vec![
            Booking::new(date(2026, 4, 1), "-61.20", "SVWZ+Testabo April")
                .counterparty("Test Streaming"),
            Booking::new(date(2026, 4, 3), "850.00", "SVWZ+Testlohn April")
                .counterparty("Test Werkstatt"),
            Booking::new(date(2026, 4, 7), "-199.99", "SVWZ+Testeinkauf 1")
                .counterparty("Test Markt"),
            Booking::new(date(2026, 4, 9), "-1.66", "SVWZ+Testgebuehr"),
        ],
    );
    let second = testbank::Account::giro(BLZ, SECOND_NUMBER, "Testperson Flow").with_bookings(
        Decimal::new(30_000, 2),
        vec![Booking::new(date(2026, 4, 2), "12.80", "SVWZ+Testzins")],
    );
    m.accounts = vec![main, second];
    m
}

fn main_iban() -> String {
    testbank::iban_de(BLZ, MAIN_NUMBER)
}

async fn serve() -> HttpBank {
    let bank = TestBank::new(model()).serve().await;
    assert_eq!(bank.bank_config().bic.as_str(), BIC);
    bank
}

/// A synthetic MT535 statement with two positions (written from the MT535
/// layout in `src/segments/mt535.rs`; the ISINs are not real).
const MT535: &str = ":16R:GENL\r\n\
:97A::SAFE//12345678/0007000999\r\n\
:98A::STAT//20260410\r\n\
:16S:GENL\r\n\
:16R:FIN\r\n\
:35B:ISIN XX0000TEST01\r\n\
TESTFONDS WELT\r\n\
:90B::MRKT//ACTU/EUR21,5\r\n\
:98A::PRIC//20260410\r\n\
:93B::AGGR//UNIT/40,\r\n\
:19A::HOLD//EUR860,\r\n\
:16S:FIN\r\n\
:16R:FIN\r\n\
:35B:ISIN XX0000TEST02\r\n\
TESTANLEIHE 2030\r\n\
:90A::MRKT//PRCT/98,5\r\n\
:93B::AGGR//FAMT/1000,\r\n\
:16S:FIN\r\n\
-";

#[tokio::test]
async fn open_balance_transactions_end() {
    let bank = serve().await;
    let (mut dialog, opened) = open(&bank, &fresh_state()).await;
    assert_eq!(opened.accounts.len(), 2, "the UPD lists both accounts");
    let account = Account::new(&main_iban(), BIC).unwrap();

    let balance = done(jobs::balance(&mut dialog, &account).await.unwrap())
        .expect("balance should be present");
    assert_eq!(balance.amount.to_string(), "2087.15");
    assert_eq!(balance.currency.as_str(), "EUR");

    let statement = done(
        jobs::transactions(&mut dialog, &account, year(), Format::Auto)
            .await
            .unwrap(),
    );
    assert_eq!(
        statement.booked.len(),
        4,
        "expected 4 transactions, got {:?}",
        statement.booked
    );
    let state = dialog.end().await.unwrap();
    assert_eq!(state.system_id.as_str(), "TESTSYS0001");
    bank.assert_clean();
}

#[tokio::test]
async fn balance_only_sends_neither_transactions_nor_holdings() {
    let bank = serve().await;
    let (mut dialog, _) = open(&bank, &fresh_state()).await;
    let account = Account::new(&main_iban(), BIC).unwrap();
    let balance = done(jobs::balance(&mut dialog, &account).await.unwrap());
    assert_eq!(
        balance.map(|b| b.amount.to_string()).as_deref(),
        Some("2087.15")
    );
    dialog.end().await.unwrap();
    let requests = bank.requests();
    assert!(requests.iter().any(|r| r.has("HKSAL")));
    assert!(
        !requests
            .iter()
            .any(|r| r.has("HKKAZ") || r.has("HKCAZ") || r.has("HKWPD")),
        "a balance sends no statement or depot request"
    );
    bank.assert_clean();
}

#[tokio::test]
async fn holdings_job_reads_the_depot() {
    let mut m = model();
    let depot = testbank::Account::depot(BLZ, "0007000999", "Testperson Flow", MT535);
    let depot_iban = depot.iban.clone().expect("depot IBAN");
    m.accounts.push(depot);
    let bank = TestBank::new(m).serve().await;
    let (mut dialog, opened) = open(&bank, &fresh_state()).await;

    // The depot has no IBAN-only address: use the UPD entry, with the
    // bank's own account number for national formats.
    let upd = opened
        .accounts
        .iter()
        .find(|a| a.iban.as_ref().is_some_and(|i| i.as_str() == depot_iban))
        .expect("the depot is in the UPD");
    let account = upd.account().unwrap();
    let holdings = done(jobs::holdings(&mut dialog, &account, None).await.unwrap());
    assert_eq!(holdings.len(), 2, "{holdings:?}");
    let fund = &holdings[0];
    assert_eq!(fund.isin.as_ref().map(|i| i.as_str()), Some("XX0000TEST01"));
    assert_eq!(fund.name, "TESTFONDS WELT");
    assert_eq!(fund.quantity.to_string(), "40");
    assert_eq!(
        fund.market_value.map(|v| v.to_string()).as_deref(),
        Some("860")
    );
    let bond = &holdings[1];
    assert_eq!(bond.quantity.to_string(), "1000");
    assert_eq!(
        bond.market_value.map(|v| v.to_string()).as_deref(),
        Some("985.0")
    );
    assert!(bank.requests().iter().any(|r| r.has("HKWPD")));
    bank.assert_clean();
}

// ═══════════════════════════════════════════════════════════════════════════
// Session state persistence
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn session_state_skips_sync_and_keeps_system_id() {
    let bank = serve().await;

    // First run: full sync.
    let (dialog, _) = open(&bank, &fresh_state()).await;
    let first = dialog.end().await.expect("end");
    assert!(first.system_id.is_assigned());
    assert_eq!(
        (first.bpd_version, first.upd_version),
        (31, 7),
        "cached versions from HIBPA/HIUPA"
    );
    assert!(first.can_skip_sync());
    assert_eq!(first.accounts(&Bic::new(BIC)).len(), 2);

    // Persist and restore like the dashboard would.
    let restored = SessionState::from_json(&first.to_json().unwrap()).unwrap();
    let before = bank.requests().len();

    // Second run: no HKSYN, and the bank sends neither BPD nor UPD again.
    let (dialog, opened) = open(&bank, &restored).await;
    assert!(
        !opened.tan_methods.is_empty(),
        "TAN methods come from the cached BPD"
    );
    let second = dialog.end().await.expect("end");
    assert_eq!(
        second.system_id, first.system_id,
        "system ID must stay stable across runs"
    );
    assert_eq!((second.bpd_version, second.upd_version), (31, 7));
    assert_eq!(
        second.accounts(&Bic::new(BIC)).len(),
        2,
        "UPD kept although the bank didn't resend it"
    );
    let requests = bank.requests();
    assert!(
        !requests[before..].iter().any(|r| r.has("HKSYN")),
        "second run skips the sync dialog"
    );
    assert_eq!(bank.issued_system_ids().len(), 1);
    bank.assert_clean();
}

#[tokio::test]
async fn session_state_for_other_user_is_ignored() {
    let bank = serve().await;
    let foreign = SessionState::new(&fints_client::Blz::new(BLZ), &UserId::new("someone-else"))
        .with_system_id(&fints_client::SystemId::new("FOREIGN"));
    let (dialog, _) = open(&bank, &foreign).await;
    let state = dialog.end().await.unwrap();
    assert_ne!(state.system_id.as_str(), "FOREIGN");
    assert_eq!(state.system_id.as_str(), "TESTSYS0001");
    assert_eq!(state.user_id, UserId::new(USER));
    bank.assert_clean();
}

#[tokio::test]
async fn several_accounts_are_fetched_in_one_dialog() {
    let bank = serve().await;
    let (mut dialog, opened) = open(&bank, &fresh_state()).await;
    assert_eq!(opened.accounts.len(), 2);
    let mut balances = Vec::new();
    for acc in &opened.accounts {
        let account = acc.account().unwrap();
        let balance = done(jobs::balance(&mut dialog, &account).await.unwrap());
        balances.push(balance.expect("balance").amount.to_string());
        done(
            jobs::transactions(&mut dialog, &account, year(), Format::Auto)
                .await
                .unwrap(),
        );
    }
    assert_eq!(balances, ["2087.15", "312.8"]);
    let state = dialog.end().await.unwrap();
    assert!(state.system_id.is_assigned());
    let dialogs: std::collections::BTreeSet<String> = bank
        .requests()
        .iter()
        .filter(|r| r.has("HKSAL"))
        .map(|r| r.dialog_id.clone())
        .collect();
    assert_eq!(dialogs.len(), 1, "both accounts in one dialog");
    bank.assert_clean();
}

/// Compile-time check: the job futures can run in a background task
/// (`tokio::spawn` needs `Send`), which is how a dashboard should drive
/// pushTAN.
#[allow(dead_code)]
fn job_futures_are_send() {
    fn is_send<T: Send>(_: T) {}
    async fn run(client: jobs::BankClient, creds: Credentials, state: SessionState) {
        let Ok((mut dialog, _)) = jobs::open(&client, &creds, &state).await else {
            return;
        };
        let account = Account::new("DE92123456780007000111", "TESTDEFFXXX").unwrap();
        if let Ok(Step::NeedTan(_, resume)) = jobs::balance(&mut dialog, &account).await {
            let _ = dialog.poll(resume).await;
        }
        let _ = jobs::transactions(&mut dialog, &account, year(), Format::Auto).await;
        let _ = dialog.end().await;
    }
    let _ = |c: jobs::BankClient, cr: Credentials, s: SessionState| is_send(run(c, cr, s));
}
