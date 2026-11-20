//! FinTS data element types with their format and length rules.
//!
//! Formats are from Formals 3.0 (2017-10-06), chapter B.4: B.4.1
//! "Basisformate" (an, num, dig, float, bin) and B.4.2 "Abgeleitete Formate"
//! (jn, code, dat, tim, id, cur, wrt). Lengths count bytes before escaping
//! (B.3.3); in ISO 8859-1 a byte is a character. Length limits that a field
//! adds (`..35`, `12`) come from `#[de(max = 35)]` / `#[de(len = 12)]` on the
//! field and are checked on top of the limits listed here.
//!
//! Constructors (`An::new`, ...) validate, so a value built in code always
//! serialises. Values read from the wire are kept as received, so a parsed
//! segment serialises back to the same bytes.

use std::fmt;

use jiff::civil::{Date, Time};
use rust_decimal::Decimal;

use crate::codec::element::{Cursor, Element, Rules};
use crate::codec::error::{CodecError, ErrorKind};
use crate::parser::DataElement;

/// A data element format: how one wire element maps to a Rust value.
///
/// For a new data element type (e.g. a typed code list), implement this and
/// add the type to the `scalar_element!` list in this file, which implements
/// [`Element`] with [`decode_scalar`] / [`encode_scalar`].
pub trait Scalar: Sized {
    /// Format name as in the specification tables (`an`, `num`, ...).
    const FORMAT: &'static str;
    /// Converts a non-empty wire element; length rules are already checked.
    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind>;
    /// Converts to a wire element; length rules are checked afterwards.
    fn to_wire(&self) -> Result<DataElement, ErrorKind>;
}

/// Byte length of a wire element before escaping (B.3.3).
fn wire_len(de: &DataElement) -> usize {
    match de {
        DataElement::Empty => 0,
        DataElement::Text(s) => s.chars().count(),
        DataElement::Binary(b) => b.len(),
    }
}

fn is_blank(de: &DataElement) -> bool {
    wire_len(de) == 0 && !matches!(de, DataElement::Binary(_))
}

/// Reads one scalar at the cursor: mandatory, length rules, then format.
pub fn decode_scalar<T: Scalar>(cur: &mut Cursor<'_>, rules: &Rules) -> Result<T, CodecError> {
    let at = cur.position().saturating_add(1);
    let de = cur.next();
    let result = if is_blank(de) {
        Err(ErrorKind::Missing)
    } else {
        rules.check(wire_len(de)).and_then(|()| T::from_wire(de))
    };
    result.map_err(|k| CodecError::new(k).at_element(at))
}

/// Writes one scalar: format, then mandatory and length rules.
pub fn encode_scalar<T: Scalar>(
    value: &T,
    out: &mut Vec<DataElement>,
    rules: &Rules,
) -> Result<(), CodecError> {
    let at = out.len().saturating_add(1);
    let de = value
        .to_wire()
        .and_then(|de| {
            if is_blank(&de) {
                Err(ErrorKind::Missing)
            } else {
                rules.check(wire_len(&de)).map(|()| de)
            }
        })
        .map_err(|k| CodecError::new(k).at_element(at))?;
    out.push(de);
    Ok(())
}

/// Implements [`Element`] for a [`Scalar`] type: one wire position.
macro_rules! scalar_element {
    ($($t:ty),* $(,)?) => {$(
        impl Element for $t {
            const WIDTH: Option<usize> = Some(1);
            const SCALAR: bool = true;
            const KIND: crate::codec::schema::ElementKind =
                crate::codec::schema::ElementKind::Scalar { format: <$t as Scalar>::FORMAT };

            fn decode(cur: &mut Cursor<'_>, rules: &Rules) -> Result<Self, CodecError> {
                decode_scalar(cur, rules)
            }

            fn encode(&self, out: &mut Vec<DataElement>, rules: &Rules) -> Result<(), CodecError> {
                encode_scalar(self, out, rules)
            }
        }
    )*};
}

fn text(de: &DataElement) -> Result<&str, ErrorKind> {
    match de {
        DataElement::Text(s) => Ok(s),
        DataElement::Binary(_) => Err(ErrorKind::ExpectedText),
        DataElement::Empty => Err(ErrorKind::Missing),
    }
}

/// `an` character rule (B.4.1): the FinTS base character set, ISO 8859-1
/// (B.1), without CR and LF.
fn check_an(s: &str, format: &'static str) -> Result<(), ErrorKind> {
    for ch in s.chars() {
        if u32::from(ch) > 0xFF {
            return Err(ErrorKind::NotLatin1 { found: ch });
        }
        if ch == '\r' || ch == '\n' {
            return Err(ErrorKind::InvalidCharacter { format, found: ch });
        }
    }
    Ok(())
}

/// `an`-based constructor: trims leading and trailing spaces, which B.4.1
/// requires before data goes into a message, then checks the characters.
fn new_an(s: &str, format: &'static str) -> Result<String, ErrorKind> {
    let s = s.trim_matches(' ');
    check_an(s, format)?;
    Ok(s.to_string())
}

fn check_max(s: &str, max: usize) -> Result<(), ErrorKind> {
    let actual = s.chars().count();
    if actual > max {
        Err(ErrorKind::TooLong { max, actual })
    } else {
        Ok(())
    }
}

macro_rules! string_type {
    ($(#[$doc:meta])* $name:ident, $format:literal, $max:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Validates and trims (Formals B.4.1) a value.
            pub fn new(value: impl AsRef<str>) -> Result<Self, ErrorKind> {
                let s = new_an(value.as_ref(), $format)?;
                if let Some(max) = $max {
                    check_max(&s, max)?;
                }
                Ok($name(s))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Scalar for $name {
            const FORMAT: &'static str = $format;

            fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
                let s = text(de)?;
                check_an(s, $format)?;
                if let Some(max) = $max {
                    check_max(s, max)?;
                }
                Ok($name(s.to_string()))
            }

            fn to_wire(&self) -> Result<DataElement, ErrorKind> {
                check_an(&self.0, $format)?;
                if let Some(max) = $max {
                    check_max(&self.0, max)?;
                }
                Ok(DataElement::Text(self.0.clone()))
            }
        }
    };
}

string_type!(
    /// `an`, alphanumeric (Formals B.4.1): ISO 8859-1 without CR and LF.
    /// The maximum length comes from the field (`#[de(max = n)]`).
    An,
    "an",
    None::<usize>
);

string_type!(
    /// `id`, identification (Formals B.4.2): `an`, at most 30 bytes. Used
    /// for user IDs, account numbers, dialog IDs.
    Id,
    "id",
    Some(30usize)
);

string_type!(
    /// `code` (Formals B.4.2): `an`; only the values listed for the field are
    /// allowed. The value list is not checked here; the length is from the
    /// field (`#[de(len = 1)]`, `#[de(max = 3)]`).
    Code,
    "code",
    None::<usize>
);

/// `num`, numeric (Formals B.4.1): digits `0`–`9`, no leading zeros.
///
/// "Not present" and `0` differ (B.3.1); use `Option<Num>` for optional
/// elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Num(u64);

impl Num {
    pub const fn new(value: u64) -> Self {
        Num(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl From<u64> for Num {
    fn from(v: u64) -> Self {
        Num(v)
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn check_digits(s: &str, format: &'static str) -> Result<(), ErrorKind> {
    match s.chars().find(|c| !c.is_ascii_digit()) {
        Some(found) => Err(ErrorKind::InvalidCharacter { format, found }),
        None => Ok(()),
    }
}

impl Scalar for Num {
    const FORMAT: &'static str = "num";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        let s = text(de)?;
        check_digits(s, "num")?;
        if s.len() > 1 && s.starts_with('0') {
            return Err(ErrorKind::LeadingZero { format: "num" });
        }
        s.parse().map(Num).map_err(|_| ErrorKind::NumberTooLarge)
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        Ok(DataElement::Text(self.0.to_string()))
    }
}

/// `dig`, digits (Formals B.4.1): digits `0`–`9`, leading zeros allowed.
/// Kept as text so the exact digits round-trip; e.g. the 12-digit message
/// size in HNHBK, or `ctr` country codes (`dig`, length 3, B.4.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Dig(String);

impl Dig {
    pub fn new(value: impl AsRef<str>) -> Result<Self, ErrorKind> {
        let s = value.as_ref();
        check_digits(s, "dig")?;
        Ok(Dig(s.to_string()))
    }

    /// `value` with leading zeros to `width` digits (more if it needs more).
    pub fn zero_padded(value: u64, width: usize) -> Self {
        Dig(format!("{value:0width$}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The numeric value, `None` if it does not fit in a `u64`.
    pub fn value(&self) -> Option<u64> {
        self.0.parse().ok()
    }
}

impl fmt::Display for Dig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Scalar for Dig {
    const FORMAT: &'static str = "dig";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        let s = text(de)?;
        check_digits(s, "dig")?;
        Ok(Dig(s.to_string()))
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        check_digits(&self.0, "dig")?;
        Ok(DataElement::Text(self.0.clone()))
    }
}

/// `jn`, yes/no (Formals B.4.2): `J` or `N`, upper case, length 1.
///
/// An omitted optional `jn` means `N` (B.4.2); `Option<Jn>` keeps the
/// difference, [`Jn::or_no`] applies the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Jn(pub bool);

impl Jn {
    pub const YES: Jn = Jn(true);
    pub const NO: Jn = Jn(false);

    pub const fn value(self) -> bool {
        self.0
    }

    /// The value of an optional `jn`, with the default `N` when omitted.
    pub fn or_no(value: Option<Jn>) -> bool {
        value.is_some_and(|j| j.0)
    }
}

impl From<bool> for Jn {
    fn from(v: bool) -> Self {
        Jn(v)
    }
}

impl Scalar for Jn {
    const FORMAT: &'static str = "jn";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        match text(de)? {
            "J" => Ok(Jn(true)),
            "N" => Ok(Jn(false)),
            _ => Err(ErrorKind::InvalidYesNo),
        }
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        Ok(DataElement::Text(
            if self.0 { "J" } else { "N" }.to_string(),
        ))
    }
}

/// `dat`, date (Formals B.4.2): `JJJJMMTT` (ISO 8601), an existing date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Dat(Date);

impl Dat {
    /// Fails for years outside 0000–9999, which eight digits cannot hold.
    pub fn new(date: Date) -> Result<Self, ErrorKind> {
        if (0..=9999).contains(&date.year()) {
            Ok(Dat(date))
        } else {
            Err(ErrorKind::InvalidDate)
        }
    }

    pub fn date(self) -> Date {
        self.0
    }
}

/// Parses exactly `n` ASCII digits.
fn fixed_digits(s: &str, n: usize) -> Option<u32> {
    if s.len() == n && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

impl Scalar for Dat {
    const FORMAT: &'static str = "dat";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        let s = text(de)?;
        let (y, m, d) = (s.get(0..4), s.get(4..6), s.get(6..8));
        match (s.len(), y, m, d) {
            (8, Some(y), Some(m), Some(d)) => {
                let date = fixed_digits(y, 4)
                    .zip(fixed_digits(m, 2))
                    .zip(fixed_digits(d, 2));
                date.and_then(|((y, m), d)| {
                    Date::new(
                        i16::try_from(y).ok()?,
                        i8::try_from(m).ok()?,
                        i8::try_from(d).ok()?,
                    )
                    .ok()
                })
                .map(Dat)
                .ok_or(ErrorKind::InvalidDate)
            }
            _ => Err(ErrorKind::InvalidDate),
        }
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        Dat::new(self.0)?;
        Ok(DataElement::Text(format!(
            "{:04}{:02}{:02}",
            self.0.year(),
            self.0.month(),
            self.0.day()
        )))
    }
}

/// `tim`, time (Formals B.4.2): `hhmmss` (ISO 8601), a valid local time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tim(Time);

impl Tim {
    /// Drops fractions of a second (a leap second `60` has no `Time` value).
    pub fn new(time: Time) -> Result<Self, ErrorKind> {
        Time::new(time.hour(), time.minute(), time.second(), 0)
            .map(Tim)
            .map_err(|_| ErrorKind::InvalidTime)
    }

    pub fn time(self) -> Time {
        self.0
    }
}

impl Scalar for Tim {
    const FORMAT: &'static str = "tim";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        let s = text(de)?;
        let parts = (s.get(0..2), s.get(2..4), s.get(4..6));
        match (s.len(), parts) {
            (6, (Some(h), Some(m), Some(sec))) => fixed_digits(h, 2)
                .zip(fixed_digits(m, 2))
                .zip(fixed_digits(sec, 2))
                .and_then(|((h, m), sec)| {
                    Time::new(
                        i8::try_from(h).ok()?,
                        i8::try_from(m).ok()?,
                        i8::try_from(sec).ok()?,
                        0,
                    )
                    .ok()
                })
                .map(Tim)
                .ok_or(ErrorKind::InvalidTime),
            _ => Err(ErrorKind::InvalidTime),
        }
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        let t = Tim::new(self.0)?;
        Ok(DataElement::Text(format!(
            "{:02}{:02}{:02}",
            t.0.hour(),
            t.0.minute(),
            t.0.second()
        )))
    }
}

/// `cur`, currency (Formals B.4.2): ISO 4217 alphabetic code, three
/// upper-case letters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cur(String);

fn check_cur(s: &str) -> Result<(), ErrorKind> {
    if let Some(found) = s.chars().find(|c| !c.is_ascii_uppercase()) {
        return Err(ErrorKind::InvalidCharacter {
            format: "cur",
            found,
        });
    }
    let actual = s.len();
    if actual != 3 {
        return Err(ErrorKind::WrongLength {
            expected: 3,
            actual,
        });
    }
    Ok(())
}

impl Cur {
    pub fn new(code: impl AsRef<str>) -> Result<Self, ErrorKind> {
        check_cur(code.as_ref())?;
        Ok(Cur(code.as_ref().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Cur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Scalar for Cur {
    const FORMAT: &'static str = "cur";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        let s = text(de)?;
        check_cur(s)?;
        Ok(Cur(s.to_string()))
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        check_cur(&self.0)?;
        Ok(DataElement::Text(self.0.clone()))
    }
}

/// `wrt`, value (Formals B.4.2): `float` (B.4.1), at most 15 bytes. Decimal
/// comma, obligatory; at least one integer digit; no leading zeros except
/// before the comma of a value below 1; never negative (signs are separate
/// elements, e.g. "Soll-Haben-Kennzeichen").
///
/// Written canonically: trailing zeros after the comma are cut (`100,` for
/// 100, `0,5` for 0.5). Read leniently in one point: trailing zeros after
/// the comma (`100,50`) are accepted, so such input does not round-trip
/// byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Wrt(Decimal);

const WRT_MAX: usize = 15;

impl Wrt {
    /// Fails for negative values and values longer than 15 bytes on the wire.
    pub fn new(value: Decimal) -> Result<Self, ErrorKind> {
        let w = Wrt(value);
        w.to_wire()?;
        Ok(w)
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

impl Scalar for Wrt {
    const FORMAT: &'static str = "wrt";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        let s = text(de)?;
        check_max(s, WRT_MAX)?;
        let Some((int, frac)) = s.split_once(',') else {
            return Err(ErrorKind::InvalidValue);
        };
        if int.is_empty() {
            return Err(ErrorKind::InvalidValue);
        }
        check_digits(int, "wrt")?;
        check_digits(frac, "wrt")?;
        if int.len() > 1 && int.starts_with('0') {
            return Err(ErrorKind::LeadingZero { format: "wrt" });
        }
        let number = if frac.is_empty() {
            int.to_string()
        } else {
            format!("{int}.{frac}")
        };
        number
            .parse::<Decimal>()
            .map(Wrt)
            .map_err(|_| ErrorKind::InvalidValue)
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        if self.0.is_sign_negative() && !self.0.is_zero() {
            return Err(ErrorKind::InvalidValue);
        }
        let s = self.0.abs().normalize().to_string();
        let s = match s.split_once('.') {
            Some((int, frac)) => format!("{int},{frac}"),
            None => format!("{s},"),
        };
        check_max(&s, WRT_MAX)?;
        Ok(DataElement::Text(s))
    }
}

/// `bin`, binary (Formals B.4.1, H.1.4): bytes, carried as `@len@data`.
/// The length rules count bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Bin(pub Vec<u8>);

impl Bin {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Scalar for Bin {
    const FORMAT: &'static str = "bin";

    fn from_wire(de: &DataElement) -> Result<Self, ErrorKind> {
        match de {
            DataElement::Binary(b) => Ok(Bin(b.clone())),
            _ => Err(ErrorKind::ExpectedBinary),
        }
    }

    fn to_wire(&self) -> Result<DataElement, ErrorKind> {
        Ok(DataElement::Binary(self.0.clone()))
    }
}

scalar_element!(An, Id, Code, Num, Dig, Jn, Dat, Tim, Cur, Wrt, Bin);

/// `btg`, amount (Messages B.1 "Betrag"): value and currency, the group
/// `4567,89:EUR` (Formals H.2.1).
#[derive(Debug, Clone, PartialEq, Eq, crate::codec::DataElementGroup)]
pub struct Btg {
    pub value: Wrt,
    pub currency: Cur,
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::ToSpan;
    use proptest::prelude::*;

    fn de(s: &str) -> DataElement {
        DataElement::Text(s.to_string())
    }

    fn decode<T: Element>(elements: &[DataElement], rules: Rules) -> Result<T, CodecError> {
        T::decode(&mut Cursor::new(elements), &rules)
    }

    fn kind<T: Element + fmt::Debug>(s: &str, rules: Rules) -> ErrorKind {
        decode::<T>(&[de(s)], rules).unwrap_err().kind
    }

    fn encode<T: Element>(value: &T, rules: Rules) -> Result<Vec<DataElement>, CodecError> {
        let mut out = Vec::new();
        value.encode(&mut out, &rules)?;
        Ok(out)
    }

    const MAX4: Rules = Rules {
        max: Some(4),
        len: None,
    };
    const LEN12: Rules = Rules {
        max: None,
        len: Some(12),
    };

    #[test]
    fn an_rules() {
        assert_eq!(
            decode::<An>(&[de("Hä ?")], Rules::NONE).unwrap().as_str(),
            "Hä ?"
        );
        assert_eq!(
            kind::<An>("a\nb", Rules::NONE),
            ErrorKind::InvalidCharacter {
                format: "an",
                found: '\n'
            }
        );
        assert_eq!(
            kind::<An>("abcde", MAX4),
            ErrorKind::TooLong { max: 4, actual: 5 }
        );
        // Lengths count characters (= ISO 8859-1 bytes), before escaping.
        assert!(decode::<An>(&[de("äöü+")], MAX4).is_ok());
        assert_eq!(
            decode::<An>(&[DataElement::Binary(b"x".to_vec())], Rules::NONE)
                .unwrap_err()
                .kind,
            ErrorKind::ExpectedText
        );
        assert_eq!(An::new("  Konto 1 ").unwrap().as_str(), "Konto 1");
        assert_eq!(
            An::new("€").unwrap_err(),
            ErrorKind::NotLatin1 { found: '€' }
        );
    }

    #[test]
    fn mandatory_and_optional() {
        let e = decode::<An>(&[DataElement::Empty], Rules::NONE).unwrap_err();
        assert_eq!((e.kind, e.element), (ErrorKind::Missing, Some(1)));
        assert_eq!(decode::<Option<An>>(&[], Rules::NONE).unwrap(), None);
        assert_eq!(
            encode(&None::<Num>, Rules::NONE).unwrap(),
            vec![DataElement::Empty]
        );
        assert_eq!(
            encode(&An("".into()), Rules::NONE).unwrap_err().kind,
            ErrorKind::Missing
        );
    }

    #[test]
    fn id_is_at_most_30() {
        assert!(Id::new("a".repeat(30)).is_ok());
        assert_eq!(
            Id::new("a".repeat(31)).unwrap_err(),
            ErrorKind::TooLong {
                max: 30,
                actual: 31
            }
        );
        assert_eq!(
            kind::<Id>(&"a".repeat(31), Rules::NONE),
            ErrorKind::TooLong {
                max: 30,
                actual: 31
            }
        );
    }

    #[test]
    fn num_rules() {
        assert_eq!(decode::<Num>(&[de("0")], Rules::NONE).unwrap(), Num(0));
        assert_eq!(decode::<Num>(&[de("300")], Rules::NONE).unwrap(), Num(300));
        assert_eq!(
            kind::<Num>("007", Rules::NONE),
            ErrorKind::LeadingZero { format: "num" }
        );
        assert_eq!(
            kind::<Num>("1a", Rules::NONE),
            ErrorKind::InvalidCharacter {
                format: "num",
                found: 'a'
            }
        );
        assert_eq!(
            kind::<Num>("-1", Rules::NONE),
            ErrorKind::InvalidCharacter {
                format: "num",
                found: '-'
            }
        );
        assert_eq!(
            kind::<Num>("99999999999999999999", Rules::NONE),
            ErrorKind::NumberTooLarge
        );
        assert_eq!(
            encode(&Num(12345), MAX4).unwrap_err().kind,
            ErrorKind::TooLong { max: 4, actual: 5 }
        );
    }

    #[test]
    fn dig_keeps_leading_zeros_and_fixed_length() {
        let d = decode::<Dig>(&[de("000000000123")], LEN12).unwrap();
        assert_eq!((d.as_str(), d.value()), ("000000000123", Some(123)));
        assert_eq!(Dig::zero_padded(123, 12).as_str(), "000000000123");
        assert_eq!(
            kind::<Dig>("123", LEN12),
            ErrorKind::WrongLength {
                expected: 12,
                actual: 3
            }
        );
    }

    #[test]
    fn jn_rules() {
        assert_eq!(decode::<Jn>(&[de("J")], Rules::NONE).unwrap(), Jn::YES);
        assert_eq!(decode::<Jn>(&[de("N")], Rules::NONE).unwrap(), Jn::NO);
        assert_eq!(kind::<Jn>("j", Rules::NONE), ErrorKind::InvalidYesNo);
        assert!(!Jn::or_no(None));
    }

    #[test]
    fn dat_and_tim_rules() {
        let d = decode::<Dat>(&[de("20240229")], Rules::NONE).unwrap();
        assert_eq!(d.date(), Date::new(2024, 2, 29).unwrap());
        for bad in ["20230229", "2024022", "202402290", "2024-2-1", "+0240229"] {
            assert_eq!(
                kind::<Dat>(bad, Rules::NONE),
                ErrorKind::InvalidDate,
                "{bad}"
            );
        }
        let t = decode::<Tim>(&[de("235959")], Rules::NONE).unwrap();
        assert_eq!(t.time(), Time::new(23, 59, 59, 0).unwrap());
        for bad in ["240000", "236000", "235960", "12345", "+12345"] {
            assert_eq!(
                kind::<Tim>(bad, Rules::NONE),
                ErrorKind::InvalidTime,
                "{bad}"
            );
        }
        assert_eq!(
            Dat::new(Date::new(-1, 1, 1).unwrap()).unwrap_err(),
            ErrorKind::InvalidDate
        );
    }

    #[test]
    fn cur_rules() {
        assert_eq!(
            decode::<Cur>(&[de("EUR")], Rules::NONE).unwrap().as_str(),
            "EUR"
        );
        assert!(matches!(
            kind::<Cur>("eur", Rules::NONE),
            ErrorKind::InvalidCharacter { .. }
        ));
        assert!(matches!(
            kind::<Cur>("EURO", Rules::NONE),
            ErrorKind::WrongLength { .. }
        ));
    }

    #[test]
    fn wrt_formals_b41_examples() {
        // Formals B.4.1: 100,00 → 100,   100,20 → 100,2   4.567,89 → 4567,89
        //                0 → 0,   0,50 → 0,5
        for (input, canonical) in [
            ("100,00", "100,"),
            ("100,20", "100,2"),
            ("4567,89", "4567,89"),
            ("0,", "0,"),
            ("0,50", "0,5"),
            ("100,", "100,"),
        ] {
            let w = decode::<Wrt>(&[de(input)], Rules::NONE).unwrap();
            assert_eq!(
                encode(&w, Rules::NONE).unwrap(),
                vec![de(canonical)],
                "{input}"
            );
        }
        assert_eq!(kind::<Wrt>("100", Rules::NONE), ErrorKind::InvalidValue);
        assert_eq!(kind::<Wrt>(",5", Rules::NONE), ErrorKind::InvalidValue);
        assert_eq!(
            kind::<Wrt>("4.567,89", Rules::NONE),
            ErrorKind::InvalidCharacter {
                format: "wrt",
                found: '.'
            }
        );
        assert_eq!(
            kind::<Wrt>("01,5", Rules::NONE),
            ErrorKind::LeadingZero { format: "wrt" }
        );
        assert_eq!(
            kind::<Wrt>("-1,", Rules::NONE),
            ErrorKind::InvalidCharacter {
                format: "wrt",
                found: '-'
            }
        );
        assert_eq!(
            kind::<Wrt>("1234567890123,45", Rules::NONE),
            ErrorKind::TooLong {
                max: 15,
                actual: 16
            }
        );
        assert_eq!(
            Wrt::new(Decimal::new(-5, 0)).unwrap_err(),
            ErrorKind::InvalidValue
        );
    }

    #[test]
    fn bin_rules() {
        let b = decode::<Bin>(&[DataElement::Binary(vec![0, 1, 2])], Rules::NONE).unwrap();
        assert_eq!(b.as_bytes(), &[0, 1, 2]);
        assert_eq!(kind::<Bin>("abc", Rules::NONE), ErrorKind::ExpectedBinary);
        assert_eq!(
            decode::<Bin>(&[DataElement::Binary(vec![0; 5])], MAX4)
                .unwrap_err()
                .kind,
            ErrorKind::TooLong { max: 4, actual: 5 }
        );
    }

    #[test]
    fn btg_formals_h21_example() {
        let elements = [de("4567,89"), de("EUR")];
        let b = decode::<Btg>(&elements, Rules::NONE).unwrap();
        assert_eq!(b.value.value(), Decimal::new(456789, 2));
        assert_eq!(b.currency.as_str(), "EUR");
        assert_eq!(encode(&b, Rules::NONE).unwrap(), elements.to_vec());
        let e = decode::<Btg>(&[de("1,"), de("eur")], Rules::NONE).unwrap_err();
        assert_eq!((e.field.as_str(), e.element), ("currency", Some(2)));
    }

    fn round_trip<T: Element + PartialEq + fmt::Debug>(value: &T, rules: Rules) {
        let wire = encode(value, rules).unwrap();
        let back: T = decode(&wire, rules).unwrap();
        assert_eq!(&back, value);
    }

    proptest! {
        #[test]
        fn decoding_never_panics(s in "\\PC{0,20}", b in proptest::collection::vec(any::<u8>(), 0..8)) {
            let inputs = [de(&s), DataElement::Binary(b), DataElement::Empty];
            for input in &inputs {
                let one = std::slice::from_ref(input);
                let _ = decode::<An>(one, MAX4);
                let _ = decode::<Id>(one, Rules::NONE);
                let _ = decode::<Code>(one, Rules::NONE);
                let _ = decode::<Num>(one, MAX4);
                let _ = decode::<Dig>(one, LEN12);
                let _ = decode::<Jn>(one, Rules::NONE);
                let _ = decode::<Dat>(one, Rules::NONE);
                let _ = decode::<Tim>(one, Rules::NONE);
                let _ = decode::<Cur>(one, Rules::NONE);
                let _ = decode::<Wrt>(one, Rules::NONE);
                let _ = decode::<Bin>(one, Rules::NONE);
                let _ = decode::<Btg>(one, Rules::NONE);
            }
        }

        #[test]
        fn an_round_trips(s in "[\\x00-\\x09\\x0b\\x0c\\x0e-\\xff]{1,40}") {
            if let Ok(v) = An::new(&s) && !v.as_str().is_empty() {
                round_trip(&v, Rules::NONE);
            }
        }

        #[test]
        fn num_round_trips(n in any::<u64>()) {
            round_trip(&Num(n), Rules::NONE);
        }

        #[test]
        fn dig_round_trips(n in any::<u32>(), width in 1usize..15) {
            round_trip(&Dig::zero_padded(u64::from(n), width), Rules::NONE);
        }

        #[test]
        fn dat_round_trips(days in 0i32..3_000_000) {
            let date = Date::new(1, 1, 1).unwrap() + i64::from(days).days();
            if let Ok(d) = Dat::new(date) {
                round_trip(&d, Rules::NONE);
            }
        }

        #[test]
        fn tim_round_trips(secs in 0u32..86_400) {
            let t = Time::midnight() + i64::from(secs).seconds();
            round_trip(&Tim::new(t).unwrap(), Rules::NONE);
        }

        #[test]
        fn wrt_round_trips(units in 0i64..10_000_000_000_000, scale in 0u32..4) {
            if let Ok(w) = Wrt::new(Decimal::new(units, scale)) {
                let wire = encode(&w, Rules::NONE).unwrap();
                let back: Wrt = decode(&wire, Rules::NONE).unwrap();
                prop_assert_eq!(back.value().normalize(), w.value().normalize());
            }
        }

        #[test]
        fn bin_round_trips(b in proptest::collection::vec(any::<u8>(), 0..64)) {
            round_trip(&Bin(b), Rules::NONE);
        }
    }
}
