//! Reading the SWIFT securities messages MT535 (holdings, in HIWPD) and
//! MT536 (transactions, in HIWDU), which arrive in a binary element.
//! HBCI 2.2 IX.2.4 and IX.2.5 describe how German banks fill them.
//!
//! A message is a sequence of fields `:TAG:content`. Content may continue
//! on the following lines; a line that does not start with `:` continues
//! the field before it. Many contents are qualified, `:QUAL//data`.
//! Amounts use a decimal comma and may end in it (`100,`); an `N` in
//! front makes them negative.

use std::str::FromStr;

use jiff::civil::Date;
use rust_decimal::Decimal;

/// One field: its tag (`35B`) and its content, continuation lines appended
/// after a `\n`.
pub(super) struct Field<'a> {
    pub(super) tag: &'a str,
    pub(super) value: String,
}

/// The fields of a message. The end mark `-` and empty lines are skipped;
/// text before the first field is dropped.
pub(super) fn fields(text: &str) -> Vec<Field<'_>> {
    let mut out: Vec<Field<'_>> = Vec::new();
    for line in text.lines().map(|l| l.trim_end_matches('\r')) {
        let opened = line
            .strip_prefix(':')
            .and_then(|after| after.split_once(':'));
        match (opened, out.last_mut()) {
            (Some((tag, first)), _) => out.push(Field {
                tag,
                value: first.to_owned(),
            }),
            _ if line.is_empty() || line == "-" => {}
            (None, Some(field)) => {
                field.value.push('\n');
                field.value.push_str(line);
            }
            (None, None) => {}
        }
    }
    out
}

/// Qualifier and data of a qualified content `:QUAL//data`.
pub(super) fn qualified(value: &str) -> Option<(&str, &str)> {
    let inner = value.strip_prefix(':')?;
    inner.split_once("//")
}

/// A SWIFT number: `1234,56`, `100,`, `,5`, with an optional leading `N`.
pub(super) fn amount(text: &str) -> Option<Decimal> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('N') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = digits.split_once(',').unwrap_or((digits, ""));
    let literal = match (whole, fraction) {
        ("", "") => return None,
        (whole, "") => whole.to_owned(),
        ("", fraction) => format!("0.{fraction}"),
        (whole, fraction) => format!("{whole}.{fraction}"),
    };
    let value = Decimal::from_str(&literal).ok()?;
    Some(if negative { -value } else { value })
}

/// Whether `text` starts with an ISO currency code (three capitals) and
/// has something after it.
fn starts_with_currency(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() > 3 && bytes[..3].iter().all(u8::is_ascii_uppercase)
}

/// A currency and an amount: `EUR4250,`, or negative `NEUR12,5`. A leading
/// `N` is a sign only when a currency follows it, so `NOK100,` stays
/// Norwegian kroner.
pub(super) fn currency_amount(text: &str) -> Option<(String, Decimal)> {
    let text = text.trim();
    let (negative, rest) = match text.strip_prefix('N') {
        Some(rest) if starts_with_currency(rest) => (true, rest),
        _ => (false, text),
    };
    if !starts_with_currency(rest) {
        return None;
    }
    let (currency, number) = rest.split_at(3);
    let value = amount(number)?;
    Some((currency.to_owned(), if negative { -value } else { value }))
}

/// The date at the start of a `YYYYMMDD` or `YYYYMMDDHHMMSS` value.
pub(super) fn date(text: &str) -> Option<Date> {
    let day = text.get(..8)?;
    Date::strptime("%Y%m%d", day).ok()
}

/// The text of a message as the bank sent it: Windows-1252 (a superset
/// of the ISO 8859-1 that FinTS prescribes), with lines that some banks
/// separate by `@@` inside the binary element.
pub(super) fn message_text(data: &[u8]) -> String {
    let decoded = encoding_rs::WINDOWS_1252.decode(data).0;
    decoded.replace("@@", "\n")
}

/// A price from the data of `:90A:` or `:90B:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Price {
    /// `PRCT/98,5`: percent of the face amount, as for bonds.
    Percent(Decimal),
    /// `ACTU/EUR12,5`: an amount per unit.
    PerUnit(String, Decimal),
}

impl Price {
    /// Reads the price types used for depots; yields (`YIEL`), discounts
    /// (`DISC`) and the rest give `None`.
    pub(super) fn parse(data: &str) -> Option<Self> {
        let (kind, rest) = data.split_once('/')?;
        match kind {
            "PRCT" => amount(rest).map(Self::Percent),
            "ACTU" => currency_amount(rest).map(|(c, a)| Self::PerUnit(c, a)),
            _ => None,
        }
    }

    pub(super) fn value(&self) -> Decimal {
        match self {
            Self::Percent(value) | Self::PerUnit(_, value) => *value,
        }
    }

    pub(super) fn currency(&self) -> Option<&str> {
        match self {
            Self::Percent(_) => None,
            Self::PerUnit(currency, _) => Some(currency),
        }
    }

    /// The `price_type` reported in the `raw` JSON.
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::Percent(_) => "PRCT",
            Self::PerUnit(..) => "ACTU",
        }
    }
}

/// A quantity from `:93B:` or `:36B:` data such as `UNIT/40,`: the number,
/// and whether it is a face amount (`FAMT`) rather than pieces.
pub(super) fn quantity(data: &str) -> Option<(Decimal, bool)> {
    let (unit, number) = data.split_once('/')?;
    Some((amount(number)?, unit == "FAMT"))
}

/// The depot number from `:97A::SAFE//` data, which is `bank code/number`
/// or the number alone.
pub(super) fn depot_number(account: &str) -> String {
    let number = account.rsplit('/').next().unwrap_or(account);
    number.trim().to_owned()
}

/// What a `:35B:` field says about a security: the ISIN after `ISIN `, a
/// national code after `/CC/` (in Germany the WKN), and the description
/// lines.
#[derive(Default)]
pub(super) struct SecurityId {
    pub(super) isin: Option<String>,
    pub(super) wkn: Option<String>,
    pub(super) name_lines: Vec<String>,
}

/// The first word of `text` and the trimmed rest.
fn first_word(text: &str) -> (&str, &str) {
    match text.split_once(' ') {
        Some((word, rest)) => (word, rest.trim()),
        None => (text, ""),
    }
}

impl SecurityId {
    /// Reads the field. The ISIN can only be on the first line. The
    /// national code may follow it on that line or start any later line;
    /// the first one counts. Text after either code and every other line
    /// is description.
    pub(super) fn parse(value: &str) -> Self {
        let mut id = Self::default();
        for (number, line) in value.lines().enumerate() {
            let mut text = line.trim();
            if number == 0
                && let Some(after) = text.strip_prefix("ISIN ")
            {
                let (isin, rest) = first_word(after.trim_start());
                id.isin = (!isin.is_empty()).then(|| isin.to_owned()).or(id.isin);
                text = rest;
            }
            if let Some((_country, coded)) = text.strip_prefix('/').and_then(|t| t.split_once('/'))
            {
                let (code, rest) = first_word(coded.trim());
                if id.wkn.is_none() && !code.is_empty() {
                    id.wkn = Some(code.to_owned());
                }
                text = rest;
            }
            if !text.is_empty() {
                id.name_lines.push(text.to_owned());
            }
        }
        id
    }

    /// The description on one line.
    pub(super) fn name(&self) -> String {
        self.name_lines.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    #[test]
    fn fields_join_continuation_lines() {
        let text = "junk\r\n:16R:FIN\r\n:35B:ISIN XS0000000003\r\n/DE/A1B2C3\r\nTEST ANLEIHE\r\n\r\n:16S:FIN\r\n-";
        let read: Vec<_> = fields(text).into_iter().map(|f| (f.tag, f.value)).collect();
        assert_eq!(
            read,
            [
                ("16R", "FIN".to_owned()),
                (
                    "35B",
                    "ISIN XS0000000003\n/DE/A1B2C3\nTEST ANLEIHE".to_owned()
                ),
                ("16S", "FIN".to_owned()),
            ]
        );
    }

    #[test]
    fn qualified_contents() {
        assert_eq!(qualified(":HOLD//EUR1,"), Some(("HOLD", "EUR1,")));
        assert_eq!(qualified("HOLD//EUR1,"), None);
        assert_eq!(qualified(":HOLD/EUR1,"), None);
    }

    #[test]
    fn numbers() {
        assert_eq!(amount("100,"), Some(dec("100")));
        assert_eq!(amount("100,").unwrap().to_string(), "100");
        assert_eq!(amount(" 0,0125 "), Some(dec("0.0125")));
        assert_eq!(amount(",5"), Some(dec("0.5")));
        assert_eq!(amount("N7,25"), Some(dec("-7.25")));
        for bad in ["", "N", ",", "1,2,3", "abc"] {
            assert_eq!(amount(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn currency_amounts() {
        assert_eq!(
            currency_amount("EUR12,5"),
            Some(("EUR".into(), dec("12.5")))
        );
        assert_eq!(currency_amount("NUSD3,"), Some(("USD".into(), dec("-3"))));
        assert_eq!(currency_amount("NOK100,"), Some(("NOK".into(), dec("100"))));
        assert_eq!(currency_amount("EUR"), None);
        assert_eq!(currency_amount("eur1,"), None);
        assert_eq!(currency_amount("€€€"), None);
    }

    #[test]
    fn dates() {
        assert_eq!(date("20260131"), Some(Date::constant(2026, 1, 31)));
        assert_eq!(date("20260131093000"), Some(Date::constant(2026, 1, 31)));
        assert_eq!(date("2026013"), None);
        assert_eq!(date("20261331"), None);
    }

    #[test]
    fn prices_quantities_and_depots() {
        assert_eq!(Price::parse("PRCT/98,5"), Some(Price::Percent(dec("98.5"))));
        let per_unit = Price::parse("ACTU/EUR12,5").unwrap();
        assert_eq!(
            (per_unit.value(), per_unit.currency()),
            (dec("12.5"), Some("EUR"))
        );
        assert_eq!(per_unit.code(), "ACTU");
        assert_eq!(Price::parse("YIEL/3,1"), None);
        assert_eq!(Price::parse("ACTU/12,5"), None);

        assert_eq!(quantity("UNIT/40,"), Some((dec("40"), false)));
        assert_eq!(quantity("FAMT/2000,"), Some((dec("2000"), true)));
        assert_eq!(quantity("UNIT"), None);

        assert_eq!(depot_number("12345678/0000004711 "), "0000004711");
        assert_eq!(depot_number("4711"), "4711");

        assert_eq!(
            message_text(b":16R:FIN@@:16S:FIN\xe4"),
            ":16R:FIN\n:16S:FINä"
        );
    }

    #[test]
    fn security_identification() {
        let id = SecurityId::parse("ISIN XS0000000003 /DE/A1B2C3 TEST\nANLEIHE 2030");
        assert_eq!(id.isin.as_deref(), Some("XS0000000003"));
        assert_eq!(id.wkn.as_deref(), Some("A1B2C3"));
        assert_eq!(id.name(), "TEST ANLEIHE 2030");

        let national_only = SecurityId::parse("/DE/A1B2C3\n/DE/ZZZZZZ\nTESTFONDS");
        assert_eq!(national_only.isin, None);
        assert_eq!(national_only.wkn.as_deref(), Some("A1B2C3"));
        assert_eq!(national_only.name(), "TESTFONDS");

        let empty_code = SecurityId::parse("/XX/\nOHNE KENNUNG");
        assert_eq!(empty_code.wkn, None);
        assert_eq!(empty_code.name_lines, ["OHNE KENNUNG"]);
    }
}
