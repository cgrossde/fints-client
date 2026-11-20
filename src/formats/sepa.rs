//! IBAN, BIC and the SEPA character sets (DFÜ-Abkommen Anlage 3 V26.11,
//! section 2.1 "Zeichensatz", section 2.3.1.1 string types).
//!
//! The checks reject; they never repair. [`transliterate`] is the one
//! helper that changes text, for a caller that wants to offer the user a
//! cleaned version of a rejected text.

use std::fmt;

use crate::types::{Bic, Iban};

/// Why a text, reference, IBAN or BIC is not acceptable for a SEPA order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Invalid {
    /// Empty, or only spaces (Anlage 3 2.1: text fields must not be blank).
    Empty,
    /// Longer than `max` characters.
    TooLong { max: usize },
    /// A character outside the allowed set.
    Character { ch: char },
    /// A reference that starts or ends with `/` or contains `//`
    /// (Anlage 3 2.1, footnote 8).
    Slash,
    /// Wrong length (IBAN 15..=34, BIC 8 or 11).
    Length,
    /// The IBAN does not start with two letters and two check digits.
    Country,
    /// The IBAN check digits are wrong (ISO 13616 mod 97).
    Checksum,
    /// The BIC is not `[A-Z0-9]{4}[A-Z]{2}[A-Z0-9]{2}([A-Z0-9]{3})?`.
    Format,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty"),
            Self::TooLong { max } => write!(f, "longer than {max} characters"),
            Self::Character { ch } => write!(f, "character {ch:?} is not allowed"),
            Self::Slash => f.write_str("must not start or end with '/' or contain '//'"),
            Self::Length => f.write_str("wrong length"),
            Self::Country => f.write_str("must start with a country code and two check digits"),
            Self::Checksum => f.write_str("check digits do not match"),
            Self::Format => f.write_str("not a BIC (8 or 11 characters, AAAABBCC[DDD])"),
        }
    }
}

impl std::error::Error for Invalid {}

/// Upper-case, without spaces.
fn compact(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase()
}

impl Iban {
    /// Check an IBAN: spaces removed, upper-cased, two letters, two check
    /// digits, 15..=34 alphanumerics in all, mod 97 = 1 (ISO 13616; Anlage 3
    /// 2.3.1.1 `IBAN2007Identifier`).
    ///
    /// ```
    /// use fints_client::Iban;
    /// assert_eq!(Iban::parse("de89 3704 0044 0532 0130 00").unwrap().as_str(),
    ///            "DE89370400440532013000");
    /// assert!(Iban::parse("DE88370400440532013000").is_err());
    /// ```
    pub fn parse(s: &str) -> Result<Iban, Invalid> {
        let iban = compact(s);
        if iban.is_empty() {
            return Err(Invalid::Empty);
        }
        if let Some(ch) = iban.chars().find(|c| !c.is_ascii_alphanumeric()) {
            return Err(Invalid::Character { ch });
        }
        if !(15..=34).contains(&iban.len()) {
            return Err(Invalid::Length);
        }
        let b = iban.as_bytes();
        if !(b[0].is_ascii_uppercase()
            && b[1].is_ascii_uppercase()
            && b[2].is_ascii_digit()
            && b[3].is_ascii_digit())
        {
            return Err(Invalid::Country);
        }
        // Move the first four characters to the end, letters A..Z -> 10..35,
        // and reduce digit by digit.
        let mut rem: u32 = 0;
        for &c in b[4..].iter().chain(&b[..4]) {
            let v = if c.is_ascii_digit() {
                u32::from(c - b'0')
            } else {
                u32::from(c - b'A') + 10
            };
            rem = if v >= 10 {
                (rem * 100 + v) % 97
            } else {
                (rem * 10 + v) % 97
            };
        }
        if rem != 1 {
            return Err(Invalid::Checksum);
        }
        Ok(Iban::new(iban))
    }
}

impl Bic {
    /// Check a BIC: spaces removed, upper-cased, 8 or 11 characters of the
    /// form `[A-Z0-9]{4}[A-Z]{2}[A-Z0-9]{2}([A-Z0-9]{3})?` (Anlage 3
    /// 2.3.1.1 `BICFIDec2014Identifier`).
    ///
    /// ```
    /// use fints_client::Bic;
    /// assert_eq!(Bic::parse("cobadeffxxx").unwrap().as_str(), "COBADEFFXXX");
    /// assert!(Bic::parse("COBADEF").is_err());
    /// ```
    pub fn parse(s: &str) -> Result<Bic, Invalid> {
        let bic = compact(s);
        if bic.is_empty() {
            return Err(Invalid::Empty);
        }
        if bic.len() != 8 && bic.len() != 11 {
            return Err(Invalid::Length);
        }
        let ok = bic.bytes().enumerate().all(|(i, c)| match i {
            4 | 5 => c.is_ascii_uppercase(),
            _ => c.is_ascii_uppercase() || c.is_ascii_digit(),
        });
        if !ok {
            return Err(Invalid::Format);
        }
        Ok(Bic::new(bic))
    }
}

/// The EPC basic Latin set (Anlage 3 2.1, first table): `a-z A-Z 0-9` and
/// `' : ? , - ( + . ) /` and space.
fn is_basic(c: char) -> bool {
    c.is_ascii_alphanumeric() || " '():?,-+./".contains(c)
}

/// A character of the SEPA text set: the basic set plus the characters the
/// German banks are obliged to accept (Anlage 3 2.1, second table:
/// `Ä Ö Ü ä ö ü ß & * $ %`).
pub(crate) fn is_sepa_char(c: char) -> bool {
    is_basic(c) || "ÄÖÜäöüß&*$%".contains(c)
}

/// The first character of `s` outside the SEPA text set, if any.
pub(crate) fn first_invalid_char(s: &str) -> Option<char> {
    s.chars().find(|&c| !is_sepa_char(c))
}

/// Whether `s` consists only of SEPA text characters (names, purpose).
/// The empty string passes; length limits are the caller's.
pub fn is_sepa_text(s: &str) -> bool {
    first_invalid_char(s).is_none()
}

/// Whether `s` is a valid SEPA reference (`MsgId`, `PmtInfId`,
/// `EndToEndId`): 1..=35 characters of the basic set (Anlage 3 2.3.1.1
/// `Max35Text`, restricted pattern), not starting or ending with `/` and
/// without `//` (Anlage 3 2.1, footnote 8).
pub fn is_sepa_reference(s: &str) -> bool {
    check_reference(s).is_ok()
}

/// [`is_sepa_reference`] with the reason.
pub(crate) fn check_reference(s: &str) -> Result<(), Invalid> {
    if s.is_empty() {
        return Err(Invalid::Empty);
    }
    if s.chars().count() > 35 {
        return Err(Invalid::TooLong { max: 35 });
    }
    if let Some(ch) = s.chars().find(|&c| !is_basic(c)) {
        return Err(Invalid::Character { ch });
    }
    if s.starts_with('/') || s.ends_with('/') || s.contains("//") {
        return Err(Invalid::Slash);
    }
    Ok(())
}

/// Replace every character outside the SEPA text set so the result passes
/// [`is_sepa_text`]: accented Latin letters lose their accent (`é` → `e`,
/// `Ç` → `C`), ligatures are spelled out (`æ` → `ae`, `Œ` → `OE`), any
/// whitespace becomes a space, typographic quotes and dashes become `'`
/// and `-`, everything else becomes `.` (the EPC best practice Anlage 3
/// 2.1 names for `* $ %`). Characters of the set, umlauts and `ß`
/// included, are kept. Length is not touched.
pub fn transliterate(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_sepa_char(c) {
            out.push(c);
        } else if c.is_whitespace() {
            out.push(' ');
        } else {
            out.push_str(replacement(c));
        }
    }
    out
}

fn replacement(c: char) -> &'static str {
    match c {
        'À' | 'Á' | 'Â' | 'Ã' | 'Å' | 'Ā' | 'Ă' | 'Ą' => "A",
        'à' | 'á' | 'â' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'Æ' => "AE",
        'æ' => "ae",
        'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => "C",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'Ď' | 'Đ' | 'Ð' => "D",
        'ď' | 'đ' | 'ð' => "d",
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => "E",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => "G",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'Ĥ' | 'Ħ' => "H",
        'ĥ' | 'ħ' => "h",
        'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'İ' => "I",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'Ĵ' => "J",
        'ĵ' => "j",
        'Ķ' => "K",
        'ķ' => "k",
        'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => "L",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'Ñ' | 'Ń' | 'Ņ' | 'Ň' => "N",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' => "O",
        'ò' | 'ó' | 'ô' | 'õ' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'Œ' => "OE",
        'œ' => "oe",
        'Ŕ' | 'Ŗ' | 'Ř' => "R",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'Ś' | 'Ŝ' | 'Ş' | 'Š' | 'Ș' => "S",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
        'Ţ' | 'Ť' | 'Ŧ' | 'Ț' => "T",
        'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
        'Þ' => "TH",
        'þ' => "th",
        'Ù' | 'Ú' | 'Û' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => "U",
        'ù' | 'ú' | 'û' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'Ŵ' => "W",
        'ŵ' => "w",
        'Ý' | 'Ÿ' | 'Ŷ' => "Y",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'Ź' | 'Ż' | 'Ž' => "Z",
        'ź' | 'ż' | 'ž' => "z",
        '‘' | '’' | '‚' | '‛' | '`' | '´' | '"' | '“' | '”' | '„' | '«' | '»' => {
            "'"
        }
        '–' | '—' | '‐' | '‑' | '−' | '_' => "-",
        _ => ".",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iban_valid() {
        // Synthetic and published example IBANs (ISO 13616 examples).
        for (input, want) in [
            ("DE89370400440532013000", "DE89370400440532013000"),
            ("de89 3704 0044 0532 0130 00", "DE89370400440532013000"),
            ("  DE02 1203 0000 0000 2020 51 ", "DE02120300000000202051"),
            ("NO9386011117947", "NO9386011117947"),
            ("GB29NWBK60161331926819", "GB29NWBK60161331926819"),
            ("FR1420041010050500013M02606", "FR1420041010050500013M02606"),
            (
                "LC55HEMM000100010012001200023015",
                "LC55HEMM000100010012001200023015",
            ),
        ] {
            assert_eq!(Iban::parse(input).unwrap().as_str(), want, "{input}");
        }
    }

    #[test]
    fn iban_invalid() {
        for (input, why) in [
            ("", Invalid::Empty),
            ("   ", Invalid::Empty),
            ("DE88370400440532013000", Invalid::Checksum),
            ("DE89370400440532013001", Invalid::Checksum),
            ("DE8937040044", Invalid::Length),
            (
                "DE89-3704-0044-0532-0130-00",
                Invalid::Character { ch: '-' },
            ),
            ("8989370400440532013000", Invalid::Country),
            ("DEXX370400440532013000", Invalid::Country),
            ("DE893704004405320130000000000000000", Invalid::Length),
        ] {
            assert_eq!(Iban::parse(input), Err(why), "{input}");
        }
    }

    #[test]
    fn bic() {
        assert_eq!(Bic::parse("COBADEFF").unwrap().as_str(), "COBADEFF");
        assert_eq!(Bic::parse("cobadeffxxx").unwrap().as_str(), "COBADEFFXXX");
        assert_eq!(Bic::parse("BYLA DEM1 001").unwrap().as_str(), "BYLADEM1001");
        // Digits allowed in the first four places (BICFIDec2014Identifier).
        assert!(Bic::parse("1234DEFF").is_ok());
        assert_eq!(Bic::parse(""), Err(Invalid::Empty));
        assert_eq!(Bic::parse("COBADEF"), Err(Invalid::Length));
        assert_eq!(Bic::parse("COBADEFFXX"), Err(Invalid::Length));
        assert_eq!(Bic::parse("COBA1EFF"), Err(Invalid::Format));
        assert_eq!(Bic::parse("COBADEF-"), Err(Invalid::Format));
    }

    #[test]
    fn sepa_text() {
        assert!(is_sepa_text("Max Mustermann"));
        assert!(is_sepa_text("Rechnung 2026/11, Kunde: 42 (Rest) +1.00 ?'-"));
        // Anlage 3 2.1, second table.
        assert!(is_sepa_text("Müller & Söhne GmbH, Straße, ÄÖÜ äöü ß * $ %"));
        assert!(is_sepa_text(""));
        for bad in [
            "Café", "a_b", "x@y", "#1", "Zoë", "€", "a\tb", "a\nb", "<b>", "\"q\"",
        ] {
            assert!(!is_sepa_text(bad), "{bad}");
        }
        assert_eq!(first_invalid_char("Café"), Some('é'));
    }

    #[test]
    fn sepa_reference() {
        assert!(is_sepa_reference("NOTPROVIDED"));
        assert!(is_sepa_reference("RE 2026-11/42 (a) +b?c:d.e,f'g"));
        assert!(is_sepa_reference(&"A".repeat(35)));
        assert_eq!(check_reference(""), Err(Invalid::Empty));
        assert_eq!(
            check_reference(&"A".repeat(36)),
            Err(Invalid::TooLong { max: 35 })
        );
        assert_eq!(check_reference("/A"), Err(Invalid::Slash));
        assert_eq!(check_reference("A/"), Err(Invalid::Slash));
        assert_eq!(check_reference("A//B"), Err(Invalid::Slash));
        // The extended characters are allowed in text, not in references.
        assert_eq!(
            check_reference("Müller"),
            Err(Invalid::Character { ch: 'ü' })
        );
        assert_eq!(check_reference("A&B"), Err(Invalid::Character { ch: '&' }));
    }

    #[test]
    fn transliterate_cleans() {
        assert_eq!(transliterate("Café Zoë Ørsted"), "Cafe Zoe Orsted");
        assert_eq!(transliterate("Müller & Söhne"), "Müller & Söhne");
        assert_eq!(transliterate("Œuvre „quoted“ – x"), "OEuvre 'quoted' - x");
        assert_eq!(transliterate("a\tb\nc"), "a b c");
        assert_eq!(transliterate("x@y #1 €5"), "x.y .1 .5");
        for s in ["Ångström Łódź Ærø", "日本", "a_b@c"] {
            assert!(is_sepa_text(&transliterate(s)), "{s}");
        }
    }
}
