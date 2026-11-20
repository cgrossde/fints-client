//! Depot holdings from HIWPD: a SWIFT MT535 "Statement of Holdings",
//! filled as HBCI 2.2 IX.2.4 describes.
//!
//! The statement opens with a general block, then has one `FIN` block per
//! security. The fields read here (values invented):
//!
//! ```text
//! :16R:GENL
//! :97A::SAFE//12345678/0000004711          depot: [bank code/]number
//! :98A::STAT//20260131                     date of the statement
//! :16S:GENL
//! :16R:FIN                                 one security
//! :35B:ISIN DE0000000001                   ISIN,
//! /DE/A0TEST                               national code (WKN),
//! TESTAKTIE AG                             description, up to four lines
//! :90B::MRKT//ACTU/EUR12,5                 price per unit (ACTU) or
//!                                          in percent (90A, PRCT)
//! :94B::PRIC//LMAR/XFRA                    where the price was found
//! :98A::PRIC//20260130                     price date (98C with time)
//! :93B::AGGR//UNIT/40,                     quantity: pieces (UNIT) or
//!                                          face amount (FAMT)
//! :19A::HOLD//EUR500,                      value of the holding
//! :70E::HOLD//1STK++++20230315+            German narrative, line 1: unit
//! 211,25+EUR                               and purchase date (5th
//!                                          subfield); line 2: purchase
//!                                          price and its currency
//! :16S:FIN
//! -
//! ```
//!
//! Fields not listed are skipped; nothing in a statement makes parsing
//! fail. A `FIN` block becomes a holding when it identifies the security
//! by ISIN or WKN, or describes it and gives a quantity.

use jiff::civil::Date;
use rust_decimal::Decimal;

use super::swift::{
    Field, Price, SecurityId, amount, currency_amount, date, depot_number, fields, message_text,
    qualified, quantity,
};
use crate::types::*;

/// What one `FIN` block has said so far.
#[derive(Default)]
struct Holding {
    security: SecurityId,
    price: Option<Price>,
    price_date: Option<Date>,
    venue: Option<String>,
    quantity: Option<Decimal>,
    face_amount: bool,
    value: Option<(String, Decimal)>,
    purchase_date: Option<Date>,
    purchase_price: Option<Decimal>,
    purchase_currency: Option<String>,
}

impl Holding {
    fn read(&mut self, field: &Field<'_>) {
        if field.tag == "35B" {
            self.security = SecurityId::parse(&field.value);
            return;
        }
        let Some((qualifier, data)) = qualified(&field.value) else {
            return;
        };
        match (field.tag, qualifier) {
            // Yields and discounts are no prices and leave an earlier one.
            ("90A" | "90B", _) => self.price = Price::parse(data).or(self.price.take()),
            ("98A" | "98C", "PRIC") => self.price_date = date(data),
            ("94B", "PRIC") => {
                // `LMAR/XFRA`: the venue follows the source type. Some
                // banks write a text after the first slash instead.
                let venue = data.split_once('/').map_or(data, |(_, v)| v).trim();
                if !venue.is_empty() {
                    self.venue = Some(venue.to_owned());
                }
            }
            ("93B", "AGGR") => {
                let read = quantity(data);
                self.quantity = read.map(|(number, _)| number);
                self.face_amount = read.is_some_and(|(_, face)| face);
            }
            ("19A", "HOLD") => self.value = currency_amount(data),
            ("70E", "HOLD") => self.read_narrative(data),
            _ => {}
        }
    }

    /// The German structured `:70E::HOLD` text. Each line begins with its
    /// number; subfields are separated by `+`.
    fn read_narrative(&mut self, text: &str) {
        let mut lines = text.lines().map(str::trim);
        if let Some(first) = lines.next().and_then(|l| l.strip_prefix('1')) {
            self.purchase_date = first.split('+').nth(4).and_then(date);
        }
        if let Some(second) = lines.next().and_then(|l| l.strip_prefix('2')) {
            let mut subfields = second.split('+');
            self.purchase_price = subfields.next().and_then(amount);
            self.purchase_currency = subfields.next().filter(|c| c.len() == 3).map(str::to_owned);
        }
    }

    /// Quantity times a price given the way this holding's price is: per
    /// unit, or in percent of the face amount.
    fn worth(&self, price: Decimal) -> Decimal {
        let total = self.quantity.unwrap_or_default() * price;
        match self.price {
            Some(Price::Percent(_)) => total / Decimal::ONE_HUNDRED,
            _ => total,
        }
    }

    fn finish(
        self,
        depot_id: Option<&str>,
        statement_date: Option<Date>,
    ) -> Option<SecurityHolding> {
        let identified = self.security.isin.is_some() || self.security.wkn.is_some();
        let described = !self.security.name_lines.is_empty() && self.quantity.is_some();
        if !identified && !described {
            return None;
        }

        let name = self.security.name();
        let quantity = self.quantity.unwrap_or_default();
        let price = self.price.as_ref().map(Price::value);
        let price_currency = self
            .price
            .as_ref()
            .and_then(Price::currency)
            .map(str::to_owned);
        // The bank's value of the holding; computed from the price only
        // when the bank gave none.
        let (market_value, market_value_currency) = match &self.value {
            Some((currency, value)) => (Some(*value), Some(currency.clone())),
            None => (price.map(|p| self.worth(p)), price_currency.clone()),
        };
        let acquisition_value = self.purchase_price.map(|p| self.worth(p));
        let profit_loss = market_value.zip(acquisition_value).map(|(m, a)| m - a);
        let quantity_type = if self.face_amount { "FAMT" } else { "UNIT" };
        let price_type = self.price.as_ref().map_or("ACTU", Price::code);
        let text = |d: Option<Decimal>| d.map(|d| d.to_string());
        let day = |d: Option<Date>| d.map(|d| d.to_string());

        let raw = serde_json::json!({
            "source": "MT535",
            "isin": self.security.isin,
            "wkn": self.security.wkn,
            "name": name,
            "quantity": quantity.to_string(),
            "quantity_type": quantity_type,
            "price": text(price),
            "price_type": price_type,
            "price_currency": price_currency,
            "price_date": day(self.price_date),
            "exchange": self.venue,
            "market_value": text(market_value),
            "market_value_currency": market_value_currency,
            "acquisition_date": day(self.purchase_date),
            "acquisition_price": text(self.purchase_price),
            "acquisition_currency": self.purchase_currency,
        });

        Some(SecurityHolding {
            isin: self.security.isin.map(Isin::new),
            wkn: self.security.wkn.map(Wkn::new),
            name,
            quantity,
            price,
            // A percent price has no currency; the purchase currency is
            // the best guess then.
            price_currency: price_currency
                .or_else(|| self.purchase_currency.clone())
                .map(Currency::new),
            price_date: self.price_date,
            market_value,
            market_value_currency: market_value_currency.map(Currency::new),
            acquisition_value,
            acquisition_date: self.purchase_date,
            acquisition_price: self.purchase_price,
            acquisition_price_currency: self.purchase_currency.map(Currency::new),
            profit_loss,
            exchange: self.venue,
            depot_id: depot_id.map(str::to_owned),
            statement_date,
            raw,
        })
    }
}

/// The holdings of one or more MT535 messages.
pub(crate) fn parse_mt535(data: &[u8]) -> Vec<SecurityHolding> {
    let text = message_text(data);
    let mut holdings = Vec::new();
    let mut depot_id: Option<String> = None;
    let mut statement_date: Option<Date> = None;
    let mut open: Option<Holding> = None;

    for field in fields(&text) {
        let content = field.value.as_str();
        match field.tag {
            "16R" if content == "FIN" => open = Some(Holding::default()),
            "16S" if content == "FIN" => {
                let done = open.take();
                holdings.extend(done.and_then(|h| h.finish(depot_id.as_deref(), statement_date)));
            }
            "97A" => {
                if let Some(("SAFE", account)) = qualified(content) {
                    depot_id = Some(depot_number(account));
                }
            }
            _ => match open.as_mut() {
                Some(holding) => holding.read(&field),
                None => {
                    if matches!(field.tag, "98A" | "98C")
                        && let Some(("STAT", day)) = qualified(content)
                    {
                        statement_date = date(day);
                    }
                }
            },
        }
    }
    holdings
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn dec(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    /// A synthetic statement: a share with every field, a fund without
    /// value field, a bond priced in percent, and a closing block.
    const STATEMENT: &str = ":16R:GENL\r\n\
:28E:2/LAST\r\n\
:20C::SEME//AUSZUG-2026-01\r\n\
:23G:NEWM\r\n\
:98A::PREP//20260131\r\n\
:98A::STAT//20260131\r\n\
:97A::SAFE//12345678/0000004711\r\n\
:17B::ACTI//Y\r\n\
:16S:GENL\r\n\
:16R:FIN\r\n\
:35B:ISIN DE0000000001\r\n\
/DE/A0TEST\r\n\
TESTAKTIE AG\r\n\
INHABER-AKTIEN\r\n\
:90B::MRKT//ACTU/EUR12,5\r\n\
:94B::PRIC//LMAR/XFRA\r\n\
:98A::PRIC//20260130\r\n\
:93B::AGGR//UNIT/40,\r\n\
:16R:SUBBAL\r\n\
:93C::TAVI//UNIT/AVAI/40,\r\n\
:16S:SUBBAL\r\n\
:19A::HOLD//EUR500,\r\n\
:70E::HOLD//1STK++++20230315+\r\n\
211,25+EUR\r\n\
:16S:FIN\r\n\
:16R:FIN\r\n\
:35B:ISIN LU0000000002\r\n\
/DE/A1FOND\r\n\
TESTFONDS WELT\r\n\
:90B::MRKT//ACTU/EUR20,04\r\n\
:98C::PRIC//20260130174500\r\n\
:93B::AGGR//UNIT/2,5\r\n\
:16S:FIN\r\n\
:16R:FIN\r\n\
:35B:ISIN DE0000000003\r\n\
TEST ANLEIHE 2030\r\n\
:90A::MRKT//PRCT/98,5\r\n\
:93B::AGGR//FAMT/2000,\r\n\
:19A::HOLD//EUR1970,\r\n\
:16S:FIN\r\n\
:16R:ADDINFO\r\n\
:19A::HOLP//EUR2520,1\r\n\
:16S:ADDINFO\r\n\
-";

    fn holdings() -> Vec<SecurityHolding> {
        parse_mt535(STATEMENT.as_bytes())
    }

    #[test]
    fn share_with_every_field() {
        let share = &holdings()[0];
        assert_eq!(share.isin.as_ref().map(Isin::as_str), Some("DE0000000001"));
        assert_eq!(share.wkn.as_ref().map(Wkn::as_str), Some("A0TEST"));
        assert_eq!(share.name, "TESTAKTIE AG INHABER-AKTIEN");
        assert_eq!(share.quantity, dec("40"));
        assert_eq!(share.price, Some(dec("12.5")));
        assert_eq!(
            share.price_currency.as_ref().map(Currency::as_str),
            Some("EUR")
        );
        assert_eq!(share.price_date, Some(Date::constant(2026, 1, 30)));
        assert_eq!(share.exchange.as_deref(), Some("XFRA"));
        assert_eq!(share.market_value, Some(dec("500")));
        assert_eq!(share.depot_id.as_deref(), Some("0000004711"));
        // Purchase: 40 at 11.25 (line 2 of :70E: without its number).
        assert_eq!(share.acquisition_date, Some(Date::constant(2023, 3, 15)));
        assert_eq!(share.acquisition_price, Some(dec("11.25")));
        assert_eq!(share.acquisition_value, Some(dec("450")));
        assert_eq!(share.profit_loss, Some(dec("50")));
        assert_eq!(
            share
                .acquisition_price_currency
                .as_ref()
                .map(Currency::as_str),
            Some("EUR")
        );
    }

    #[test]
    fn statement_date_is_stat_not_the_price_date() {
        let all = holdings();
        assert_eq!(all.len(), 3);
        for holding in &all {
            assert_eq!(holding.statement_date, Some(Date::constant(2026, 1, 31)));
        }
        let undated = STATEMENT.replace(":98A::STAT//20260131\r\n", "");
        assert!(
            parse_mt535(undated.as_bytes())
                .iter()
                .all(|h| h.statement_date.is_none())
        );
    }

    #[test]
    fn value_is_computed_when_the_bank_gives_none() {
        let fund = &holdings()[1];
        assert_eq!(fund.quantity, dec("2.5"));
        assert_eq!(fund.price_date, Some(Date::constant(2026, 1, 30)));
        assert_eq!(fund.market_value, Some(dec("50.1")));
        assert_eq!(
            fund.market_value_currency.as_ref().map(Currency::as_str),
            Some("EUR")
        );
        assert_eq!(fund.raw["market_value"], "50.100");
    }

    #[test]
    fn bond_price_is_percent_of_the_face_amount() {
        let bond = &holdings()[2];
        assert!(bond.wkn.is_none());
        assert_eq!(bond.name, "TEST ANLEIHE 2030");
        assert_eq!(bond.quantity, dec("2000"));
        assert_eq!(bond.price, Some(dec("98.5")));
        assert_eq!(bond.market_value, Some(dec("1970")));
        assert_eq!(bond.raw["quantity_type"], "FAMT");
        assert_eq!(bond.raw["price_type"], "PRCT");

        // Without :19A: the value comes from the percent price.
        let computed = STATEMENT.replace(":19A::HOLD//EUR1970,\r\n", "");
        assert_eq!(
            parse_mt535(computed.as_bytes())[2].market_value,
            Some(dec("1970"))
        );
    }

    #[test]
    fn identification_on_one_line() {
        let text = ":16R:FIN\r\n:35B:ISIN LU0000000002 /DE/A1FOND\r\nTESTFONDS WELT\r\n\
                    :93B::AGGR//UNIT/1,\r\n:16S:FIN\r\n";
        let fund = &parse_mt535(text.as_bytes())[0];
        assert_eq!(fund.isin.as_ref().map(Isin::as_str), Some("LU0000000002"));
        assert_eq!(fund.wkn.as_ref().map(Wkn::as_str), Some("A1FOND"));
        assert_eq!(fund.name, "TESTFONDS WELT");
    }

    #[test]
    fn narrative_with_all_subfields() {
        let text = ":16R:FIN\r\n:35B:ISIN DE0000000001\r\n:93B::AGGR//UNIT/4,\r\n\
                    :70E::HOLD//1STK+123+00045+DE+20240229+\r\n29,5+USD+\r\n3\r\n:16S:FIN\r\n";
        let share = &parse_mt535(text.as_bytes())[0];
        assert_eq!(share.acquisition_date, Some(Date::constant(2024, 2, 29)));
        assert_eq!(share.acquisition_price, Some(dec("9.5")));
        assert_eq!(share.acquisition_value, Some(dec("38")));
        assert_eq!(
            share.price_currency.as_ref().map(Currency::as_str),
            Some("USD")
        );
    }

    #[test]
    fn unidentified_block_needs_description_and_quantity() {
        let described =
            ":16R:FIN\r\n:35B:/XX/\r\nTEST BETEILIGUNG\r\n:93B::AGGR//UNIT/3,\r\n:16S:FIN\r\n";
        let found = parse_mt535(described.as_bytes());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "TEST BETEILIGUNG");
        assert!(found[0].isin.is_none() && found[0].wkn.is_none());

        for bare in [
            ":16R:FIN\r\n:93B::AGGR//UNIT/3,\r\n:16S:FIN\r\n",
            ":16R:FIN\r\n:35B:/XX/\r\nTEST BETEILIGUNG\r\n:16S:FIN\r\n",
        ] {
            assert!(parse_mt535(bare.as_bytes()).is_empty());
        }
    }

    #[test]
    fn empty_depot_and_lines_split_by_at_signs() {
        let empty = ":16R:GENL@@:97A::SAFE//12345678/1@@:17B::ACTI//N@@:16S:GENL@@-";
        assert!(parse_mt535(empty.as_bytes()).is_empty());
        let joined = STATEMENT.replace("\r\n", "@@");
        assert_eq!(parse_mt535(joined.as_bytes()).len(), 3);
    }

    use proptest::strategy::Strategy as _;

    const PIECES: &[&str] = &[
        ":16R:FIN\r\n",
        ":16S:FIN\r\n",
        ":16R:GENL\r\n",
        ":35B:ISIN ",
        "/DE/",
        ":90A::MRKT//PRCT/",
        ":90B::MRKT//ACTU/EUR",
        ":93B::AGGR//FAMT/",
        ":19A::HOLD//N",
        ":98C::STAT//",
        ":70E::HOLD//1",
        "\r\n2",
        "@@",
    ];

    proptest::proptest! {
        #[test]
        fn any_bytes_are_read_without_panic(
            bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..600)
        ) {
            parse_mt535(&bytes);
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
            parse_mt535(picks.concat().as_bytes());
        }
    }
}
