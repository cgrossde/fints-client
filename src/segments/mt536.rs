//! Depot transactions from HIWDU: a SWIFT MT536 "Statement of
//! Transactions", filled as HBCI 2.2 IX.2.5 describes.
//!
//! The statement nests blocks opened by `:16R:` and closed by `:16S:`:
//! a general block, then per security a `FIN` block whose `TRAN` blocks
//! are its transactions. The fields read here (values invented):
//!
//! ```text
//! :16R:GENL
//! :97A::SAFE//12345678/0000004711          depot: [bank code/]number
//! :16S:GENL
//! :16R:SUBSAFE
//! :16R:FIN                                 one security
//! :35B:ISIN DE0000000001 /DE/A0TEST        identification as in MT535
//! TESTAKTIE AG
//! :16R:TRAN                                one transaction
//! :16R:TRANSDET
//! :36B::PSTA//UNIT/8,                      quantity posted (UNIT/FAMT)
//! :19A::PSTA//NEUR100,                     amount posted, N = negative
//! :22F::TRAN//SETT                         kind of transaction
//! :22H::REDE//RECE                         received (RECE) or delivered (DELI)
//! :22H::PAYM//APMT                         against payment or free
//! :98A::ESET//20260115                     effective settlement date
//! :98C::TRAD//20260113101500               trade date, 98A or 98C
//! :98A::SETT//20260115                     settlement date
//! :90B::DEAL//ACTU/EUR12,5                 price of the deal
//! :25D::MOVE//REVE                         the posting is a reversal
//! :70E::TRDE//KAUF                         free text, may span lines
//! :16S:TRANSDET
//! :16S:TRAN
//! :16S:FIN
//! :16S:SUBSAFE
//! ```
//!
//! Fields not listed are skipped; nothing in a statement makes parsing
//! fail. A `TRAN` block outside a `FIN` block is dropped, since it names
//! no security.

use jiff::civil::Date;
use rust_decimal::Decimal;

use super::swift::{
    Price, SecurityId, currency_amount, date, depot_number, fields, message_text, qualified,
    quantity,
};
use crate::types::*;

/// What one `TRAN` block has said so far.
#[derive(Default)]
struct Posting {
    quantity: Option<(Decimal, bool)>,
    amount: Option<(String, Decimal)>,
    receive_or_deliver: Option<String>,
    payment: Option<String>,
    kind: Option<String>,
    effective: Option<Date>,
    traded: Option<Date>,
    settled: Option<Date>,
    price: Option<Price>,
    reversal: bool,
    text: Vec<String>,
}

impl Posting {
    fn read(&mut self, tag: &str, content: &str) {
        let Some((qualifier, data)) = qualified(content) else {
            return;
        };
        let code = || Some(data.trim().to_owned());
        match (tag, qualifier) {
            ("36B", "PSTA") => self.quantity = quantity(data),
            ("19A", "PSTA") => self.amount = currency_amount(data),
            ("22H", "REDE") => self.receive_or_deliver = code(),
            ("22H", "PAYM") => self.payment = code(),
            ("22F", "TRAN") => self.kind = code(),
            ("98A" | "98C", "ESET") => self.effective = date(data),
            ("98A" | "98C", "TRAD") => self.traded = date(data),
            ("98A" | "98C", "SETT") => self.settled = date(data),
            ("90A" | "90B", "DEAL") => self.price = Price::parse(data).or(self.price.take()),
            ("25D", "MOVE") => self.reversal = data.trim() == "REVE",
            ("70E", _) => {
                let lines = data.lines().map(str::trim).filter(|l| !l.is_empty());
                self.text.extend(lines.map(str::to_owned));
            }
            _ => {}
        }
    }

    fn finish(self, security: &SecurityId, depot_id: Option<&str>) -> DepotTransaction {
        let direction = match self.receive_or_deliver.as_deref() {
            Some("RECE") => DepotTransactionDirection::Receive,
            Some("DELI") => DepotTransactionDirection::Deliver,
            _ => DepotTransactionDirection::Unknown,
        };
        let face_amount = self.quantity.is_some_and(|(_, face)| face);
        let (currency, amount) = self
            .amount
            .map_or((None, None), |(c, a)| (Some(c), Some(a)));
        let raw = serde_json::json!({
            "source": "MT536",
            "isin": security.isin,
            "wkn": security.wkn,
            "quantity_type": if face_amount { "FAMT" } else { "UNIT" },
            "direction": self.receive_or_deliver,
            "payment": self.payment,
            "transaction_type": self.kind,
            "price_type": self.price.as_ref().map_or("ACTU", Price::code),
            "reversal": self.reversal,
        });
        DepotTransaction {
            isin: security.isin.clone().map(Isin::new),
            wkn: security.wkn.clone().map(Wkn::new),
            name: security.name(),
            quantity: self.quantity.map(|(number, _)| number).unwrap_or_default(),
            amount,
            currency: currency.map(Currency::new),
            direction,
            transaction_type: self.kind,
            effective_date: self.effective,
            trade_date: self.traded,
            settlement_date: self.settled,
            price: self.price.as_ref().map(Price::value),
            price_currency: self
                .price
                .as_ref()
                .and_then(Price::currency)
                .map(Currency::new),
            is_reversal: self.reversal,
            details: (!self.text.is_empty()).then(|| self.text.join("\n")),
            depot_id: depot_id.map(str::to_owned),
            raw,
        }
    }
}

/// The transactions of one or more MT536 messages.
pub(crate) fn parse_mt536(data: &[u8]) -> Vec<DepotTransaction> {
    let text = message_text(data);
    let mut transactions = Vec::new();
    let mut depot_id: Option<String> = None;
    // The security of the open FIN block, and the open TRAN block in it.
    let mut security: Option<SecurityId> = None;
    let mut posting: Option<Posting> = None;

    for field in fields(&text) {
        let content = field.value.trim();
        match field.tag {
            "16R" | "16S" => {
                let opens = field.tag == "16R";
                match (content, opens) {
                    ("FIN", true) => security = Some(SecurityId::default()),
                    ("FIN", false) => security = None,
                    ("TRAN", true) => {
                        posting = security.as_ref().map(|_| Posting::default());
                        continue;
                    }
                    ("TRAN", false) => {
                        if let (Some(done), Some(security)) = (posting.take(), &security) {
                            transactions.push(done.finish(security, depot_id.as_deref()));
                        }
                    }
                    _ => continue,
                }
                posting = None;
            }
            _ => match posting.as_mut() {
                Some(open) => open.read(field.tag, &field.value),
                None if field.tag == "35B" => {
                    if let Some(security) = security.as_mut() {
                        *security = SecurityId::parse(&field.value);
                    }
                }
                None if field.tag == "97A" => {
                    if let Some(("SAFE", account)) = qualified(content) {
                        depot_id = Some(depot_number(account));
                    }
                }
                None => {}
            },
        }
    }
    transactions
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn dec(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    /// A synthetic statement: a purchase with every field and a reversed
    /// delivery of one share, then a bond bought at a percent price.
    const STATEMENT: &str = ":16R:GENL\r\n\
:28E:2/LAST\r\n\
:20C::SEME//AUSZUG-2026-01\r\n\
:23G:NEWM\r\n\
:69A::STAT//20260101/20260131\r\n\
:97A::SAFE//12345678/0000004711\r\n\
:17B::ACTI//Y\r\n\
:16S:GENL\r\n\
:16R:SUBSAFE\r\n\
:16R:FIN\r\n\
:35B:ISIN DE0000000001 /DE/A0TEST\r\n\
TESTAKTIE AG\r\n\
INHABER-AKTIEN\r\n\
:16R:TRAN\r\n\
:16R:LINK\r\n\
:20C::RELA//ORDER-0815\r\n\
:16S:LINK\r\n\
:16R:TRANSDET\r\n\
:36B::PSTA//UNIT/8,\r\n\
:19A::PSTA//NEUR100,\r\n\
:22F::TRAN//SETT\r\n\
:22H::REDE//RECE\r\n\
:22H::PAYM//APMT\r\n\
:98A::ESET//20260115\r\n\
:98C::TRAD//20260113101500\r\n\
:98A::SETT//20260116\r\n\
:90B::DEAL//ACTU/EUR12,5\r\n\
:70E::TRDE//KAUF\r\n\
TESTPLATZ\r\n\
:16S:TRANSDET\r\n\
:16S:TRAN\r\n\
:16R:TRAN\r\n\
:16R:TRANSDET\r\n\
:36B::PSTA//UNIT/3,\r\n\
:19A::PSTA//EUR37,5\r\n\
:22H::REDE//DELI\r\n\
:98A::ESET//20260120\r\n\
:25D::MOVE//REVE\r\n\
:16S:TRANSDET\r\n\
:16S:TRAN\r\n\
:16S:FIN\r\n\
:16R:FIN\r\n\
:35B:ISIN DE0000000003\r\n\
TEST ANLEIHE 2030\r\n\
:16R:TRAN\r\n\
:16R:TRANSDET\r\n\
:36B::PSTA//FAMT/500,\r\n\
:22H::REDE//RECE\r\n\
:98C::ESET//20260128090000\r\n\
:90A::DEAL//PRCT/101,\r\n\
:16S:TRANSDET\r\n\
:16S:TRAN\r\n\
:16S:FIN\r\n\
:16S:SUBSAFE\r\n\
-";

    fn transactions() -> Vec<DepotTransaction> {
        parse_mt536(STATEMENT.as_bytes())
    }

    #[test]
    fn purchase_with_every_field() {
        let all = transactions();
        assert_eq!(all.len(), 3);
        let buy = &all[0];
        assert_eq!(buy.isin.as_ref().map(Isin::as_str), Some("DE0000000001"));
        assert_eq!(buy.wkn.as_ref().map(Wkn::as_str), Some("A0TEST"));
        assert_eq!(buy.name, "TESTAKTIE AG INHABER-AKTIEN");
        assert_eq!(buy.quantity, dec("8"));
        assert_eq!(buy.amount, Some(dec("-100")));
        assert_eq!(buy.currency.as_ref().map(Currency::as_str), Some("EUR"));
        assert_eq!(buy.direction, DepotTransactionDirection::Receive);
        assert_eq!(buy.transaction_type.as_deref(), Some("SETT"));
        assert_eq!(buy.effective_date, Some(Date::constant(2026, 1, 15)));
        assert_eq!(buy.trade_date, Some(Date::constant(2026, 1, 13)));
        assert_eq!(buy.settlement_date, Some(Date::constant(2026, 1, 16)));
        assert_eq!(buy.price, Some(dec("12.5")));
        assert_eq!(
            buy.price_currency.as_ref().map(Currency::as_str),
            Some("EUR")
        );
        assert_eq!(buy.details.as_deref(), Some("KAUF\nTESTPLATZ"));
        assert_eq!(buy.depot_id.as_deref(), Some("0000004711"));
        assert!(!buy.is_reversal);
        assert_eq!(buy.raw["payment"], "APMT");
        assert_eq!(buy.raw["price_type"], "ACTU");
    }

    #[test]
    fn reversed_delivery_keeps_the_security() {
        let reversed = &transactions()[1];
        assert_eq!(
            reversed.isin.as_ref().map(Isin::as_str),
            Some("DE0000000001")
        );
        assert_eq!(reversed.direction, DepotTransactionDirection::Deliver);
        assert_eq!(reversed.quantity, dec("3"));
        assert_eq!(reversed.amount, Some(dec("37.5")));
        assert!(reversed.is_reversal);
        assert_eq!(reversed.details, None);
    }

    #[test]
    fn bond_at_a_percent_price() {
        let bond = &transactions()[2];
        assert!(bond.wkn.is_none());
        assert_eq!(bond.name, "TEST ANLEIHE 2030");
        assert_eq!(bond.quantity, dec("500"));
        assert_eq!(bond.raw["quantity_type"], "FAMT");
        assert_eq!(bond.raw["price_type"], "PRCT");
        assert_eq!(bond.price, Some(dec("101")));
        assert!(bond.price_currency.is_none());
        assert_eq!(bond.effective_date, Some(Date::constant(2026, 1, 28)));
        assert_eq!(bond.amount, None);
    }

    #[test]
    fn statements_without_transactions_and_stray_blocks() {
        assert!(parse_mt536(b"").is_empty());
        assert!(parse_mt536(b":16R:GENL\r\n:97A::SAFE//4711\r\n:16S:GENL\r\n-").is_empty());
        let outside_fin = ":16R:TRAN\r\n:36B::PSTA//UNIT/1,\r\n:16S:TRAN\r\n";
        assert!(parse_mt536(outside_fin.as_bytes()).is_empty());
        let after_fin = STATEMENT.replace(
            ":16S:SUBSAFE",
            ":16R:TRAN\r\n:36B::PSTA//UNIT/1,\r\n:16S:TRAN\r\n:16S:SUBSAFE",
        );
        assert_eq!(parse_mt536(after_fin.as_bytes()).len(), 3);
    }

    use proptest::strategy::Strategy as _;

    const PIECES: &[&str] = &[
        ":16R:FIN\r\n",
        ":16S:FIN\r\n",
        ":16R:TRAN\r\n",
        ":16S:TRAN\r\n",
        ":35B:ISIN ",
        ":36B::PSTA//FAMT/",
        ":19A::PSTA//N",
        ":98A::TRAD//",
        ":90A::DEAL//PRCT/",
        ":70E::TRDE//",
        "@@",
    ];

    proptest::proptest! {
        #[test]
        fn any_bytes_are_read_without_panic(
            bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..600)
        ) {
            parse_mt536(&bytes);
        }

        #[test]
        fn shuffled_fields_are_read_without_panic(
            picks in proptest::collection::vec(
                proptest::prop_oneof![
                    proptest::sample::select(PIECES).prop_map(str::to_owned),
                    "[0-9A-Z/:+,N ]{0,10}",
                    "[äöüß€\\x00-\\x7f]{0,5}",
                ],
                0..30,
            )
        ) {
            parse_mt536(picks.concat().as_bytes());
        }
    }
}
