//! Parser for the MT940/MT942 field `:86:` (Mehrzweckfeld).
//!
//! Written from DFÜ-Abkommen Anlage 3, Teil 5, §5.1.4 "Strukturierte Belegung
//! des Feldes 86" and the rules in `docs/formats-design.md` §3:
//!
//! ```text
//! GVC ?00 posting text ?10 primanota ?20…?29 purpose ?30 BIC/BLZ
//! ?31 IBAN/account ?32 ?33 name ?34 text key extension ?60…?63 purpose
//! ```
//!
//! The purpose subfields carry the SEPA keywords (`EREF+`, `SVWZ+`, …).
//! [`parse`] is pure, never fails and never panics; anything it does not
//! understand stays in the output.
//!
//! The output keeps every parsed value and the bank's characters exactly
//! (no SWIFT mapping, no case folding). It does not decide what a
//! `Transaction` exposes: the MT940 mapping does that
//! (docs/formats-design.md).

/// A SEPA keyword of a `:86:` purpose (§5.1.4, plus `PURP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Keyword {
    /// `EREF+` end-to-end reference.
    Eref,
    /// `KREF+` customer reference.
    Kref,
    /// `MREF+` mandate reference.
    Mref,
    /// `CRED+` creditor ID.
    Cred,
    /// `DEBT+` originator ID.
    Debt,
    /// `COAM+` compensation amount.
    Coam,
    /// `OAMT+` original amount.
    Oamt,
    /// `SVWZ+` remittance text.
    Svwz,
    /// `ABWA+` ultimate party on the initiator's side.
    Abwa,
    /// `ABWE+` ultimate party on the recipient's side.
    Abwe,
    /// `PURP+` purpose code (not in §5.1.4).
    Purp,
}

impl Keyword {
    /// All keywords. The first ten are in §5.1.4 order.
    pub(crate) const ALL: [Keyword; 11] = [
        Keyword::Eref,
        Keyword::Kref,
        Keyword::Mref,
        Keyword::Cred,
        Keyword::Debt,
        Keyword::Coam,
        Keyword::Oamt,
        Keyword::Svwz,
        Keyword::Abwa,
        Keyword::Abwe,
        Keyword::Purp,
    ];

    /// The four letters before the `+`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Keyword::Eref => "EREF",
            Keyword::Kref => "KREF",
            Keyword::Mref => "MREF",
            Keyword::Cred => "CRED",
            Keyword::Debt => "DEBT",
            Keyword::Coam => "COAM",
            Keyword::Oamt => "OAMT",
            Keyword::Svwz => "SVWZ",
            Keyword::Abwa => "ABWA",
            Keyword::Abwe => "ABWE",
            Keyword::Purp => "PURP",
        }
    }

    /// Whether the value is a reference, where `NOTPROVIDED`/`NONREF` mean
    /// absent. Other values are absent only when empty.
    pub(crate) fn is_reference(self) -> bool {
        matches!(
            self,
            Keyword::Eref | Keyword::Kref | Keyword::Mref | Keyword::Cred | Keyword::Debt
        )
    }

    /// The keyword at the start of `s` (`XXXX+`), if it is a known one.
    fn prefix_of(s: &str) -> Option<Keyword> {
        let b = s.as_bytes();
        if b.len() < 5 || b[4] != b'+' || !b[..4].iter().all(u8::is_ascii_uppercase) {
            return None;
        }
        Keyword::ALL
            .into_iter()
            .find(|k| k.as_str().as_bytes() == &b[..4])
    }

    /// Position in §5.1.4 order for rule 6, or `None` for `PURP`, which
    /// §5.1.4 does not list and which therefore only counts at a subfield
    /// start.
    fn spec_rank(self) -> Option<usize> {
        Keyword::ALL[..10].iter().position(|k| *k == self)
    }
}

/// An R-transaction marker at the start of the `SVWZ+` value (§5.1.4
/// footnote 32). `ABWA`/`ABWE` then refer to the original payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReturnMarker {
    Reject,
    Return,
    Refund,
    Ruecklastschrift,
}

/// One keyword's value; repeats of the keyword are joined in text order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SepaValue {
    pub keyword: Keyword,
    /// The value as the bank sent it, trimmed. May be a placeholder.
    pub value: String,
}

impl SepaValue {
    /// Whether the value means "no value": [`super::absent`] for
    /// references, the empty string for text.
    pub(crate) fn is_absent(&self) -> bool {
        if self.keyword.is_reference() {
            super::absent(&self.value)
        } else {
            self.value.is_empty()
        }
    }
}

/// A subfield the parser keeps but does not interpret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Subfield {
    /// The two digits after `?`.
    pub code: String,
    pub value: String,
}

/// The parsed `:86:` field. Text values are trimmed; `None` means the
/// subfield is missing or empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tag86 {
    /// Geschäftsvorfallcode, three digits. `None` when the field does not
    /// start with one.
    pub gvc: Option<String>,
    /// Whether the field had `?nn` subfields (§3 rule 2). When false, the
    /// whole text after the GVC (or the whole field) is the purpose.
    pub structured: bool,
    /// `?00` posting text.
    pub posting_text: Option<String>,
    /// `?10` primanota.
    pub primanota: Option<String>,
    /// `?30` BIC or BLZ, cut at the first space.
    pub bic: Option<String>,
    /// `?31` IBAN or account number.
    pub account: Option<String>,
    /// `?32` and `?33` joined.
    pub name: Option<String>,
    /// `?34` text key extension.
    pub text_key_extension: Option<String>,
    /// `?20…?29` then `?60…?63` joined with no separator, keywords
    /// included, trimmed. For unstructured text, that text.
    pub purpose: String,
    /// Purpose text before the first keyword, or the whole purpose when it
    /// has no keyword (a non-SEPA booking).
    pub plain_purpose: Option<String>,
    /// Keyword values, one per keyword, in order of first appearance.
    pub sepa: Vec<SepaValue>,
    /// R-transaction marker from the `SVWZ+` value.
    pub return_marker: Option<ReturnMarker>,
    /// Subfields with an unknown code, in field order.
    pub other: Vec<Subfield>,
    /// The purpose split at keywords, untrimmed, in text order: joining
    /// each keyword token (`XXXX+`) and text gives the purpose back.
    segments: Vec<(Option<Keyword>, String)>,
}

impl Tag86 {
    /// The keyword's value as sent, placeholders included.
    pub(crate) fn raw(&self, keyword: Keyword) -> Option<&str> {
        self.sepa
            .iter()
            .find(|v| v.keyword == keyword)
            .map(|v| v.value.as_str())
    }

    /// The keyword's value, or `None` when it is missing or absent
    /// ([`SepaValue::is_absent`]).
    pub(crate) fn value(&self, keyword: Keyword) -> Option<&str> {
        self.sepa
            .iter()
            .find(|v| v.keyword == keyword && !v.is_absent())
            .map(|v| v.value.as_str())
    }

    /// Whether the booking is a SEPA direct debit (`MREF+` or `CRED+`).
    pub(crate) fn is_direct_debit(&self) -> bool {
        self.value(Keyword::Mref).is_some() || self.value(Keyword::Cred).is_some()
    }

    /// The ultimate debtor: `ABWA+` of a transfer (the initiator's side),
    /// `ABWE+` of a direct debit, where the creditor initiates. An
    /// R-transaction's keywords refer to the original payment (footnote
    /// 33), so the direction of the booking does not matter.
    pub(crate) fn ultimate_debtor(&self) -> Option<&str> {
        if self.is_direct_debit() {
            self.value(Keyword::Abwe)
        } else {
            self.value(Keyword::Abwa)
        }
    }

    /// The ultimate creditor: the other keyword than [`Self::ultimate_debtor`].
    pub(crate) fn ultimate_creditor(&self) -> Option<&str> {
        if self.is_direct_debit() {
            self.value(Keyword::Abwa)
        } else {
            self.value(Keyword::Abwe)
        }
    }

    /// The reason code after an R-transaction marker in `SVWZ+`, e.g. `AC04`
    /// in `RETURN AC04`: the next word when it is four letters or digits.
    pub(crate) fn return_reason(&self) -> Option<&str> {
        self.return_marker?;
        let word = self.raw(Keyword::Svwz)?.split_whitespace().nth(1)?;
        (word.len() == 4
            && word
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
        .then_some(word)
    }

    /// An amount keyword (`OAMT+`, `COAM+`) as a decimal: `12,50` or
    /// `12.50`, nothing else. The field carries no currency.
    pub(crate) fn amount(&self, keyword: Keyword) -> Option<rust_decimal::Decimal> {
        let text = self.value(keyword)?.trim().replace(',', ".");
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return None;
        }
        text.parse().ok()
    }
}

/// Parse the full text of one `:86:` field (without the `:86:` tag).
pub(crate) fn parse(text: &str) -> Tag86 {
    // Rule 1: the 65-character wrap can split a `?nn` code or a keyword.
    let text: String = text.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
    let mut out = Tag86::default();

    // Rule 2: GVC, then subfields; a 9xx GVC is followed by free text.
    let digits = text.len() >= 3 && text.as_bytes()[..3].iter().all(u8::is_ascii_digit);
    let rest = if digits { &text[3..] } else { text.as_str() };
    let structured = digits && !text.starts_with('9') && marker_at(rest, 0);
    let purpose_parts: Vec<&str> = if structured {
        out.gvc = Some(text[..3].to_owned());
        out.structured = true;
        let subfields = split_subfields(rest);
        let mut purpose: Vec<(u8, &str)> = Vec::new();
        let mut name = String::new();
        for (code, value) in subfields {
            match code {
                0 => out.posting_text = non_empty(value),
                10 => out.primanota = non_empty(value),
                20..=29 | 60..=63 => purpose.push((code, value)),
                30 => out.bic = non_empty(value.trim().split(' ').next().unwrap_or("")),
                31 => out.account = non_empty(value),
                32 | 33 => name.push_str(value),
                34 => out.text_key_extension = non_empty(value),
                _ => out.other.push(Subfield {
                    code: format!("{code:02}"),
                    value: value.to_owned(),
                }),
            }
        }
        out.name = non_empty(&name);
        // Rule 4: code order; a stable sort keeps repeated codes in place.
        purpose.sort_by_key(|(code, _)| *code);
        purpose.into_iter().map(|(_, v)| v).collect()
    } else if digits && text.starts_with('9') {
        out.gvc = Some(text[..3].to_owned());
        vec![rest]
    } else {
        vec![text.as_str()]
    };

    let joined: String = purpose_parts.concat();
    out.purpose = joined.trim().to_owned();
    out.segments = split_keywords(&purpose_parts);

    for (keyword, text) in &out.segments {
        match keyword {
            None => out.plain_purpose = non_empty(text),
            Some(k) => match out.sepa.iter_mut().find(|v| v.keyword == *k) {
                Some(v) => v.value.push_str(text),
                None => out.sepa.push(SepaValue {
                    keyword: *k,
                    value: text.clone(),
                }),
            },
        }
    }
    for v in &mut out.sepa {
        v.value = v.value.trim().to_owned();
    }
    out.return_marker = out.raw(Keyword::Svwz).and_then(return_marker);
    out
}

/// Rule 5 and 6: split the purpose subfields at keywords.
///
/// A keyword counts at the start of a subfield (§5.1.4). When the
/// purpose starts with a keyword, a keyword later in §5.1.4 order than the
/// current one also counts inside a subfield (rule 6), so `SVWZ+` text is
/// only ended by `ABWA+`/`ABWE+`.
fn split_keywords(parts: &[&str]) -> Vec<(Option<Keyword>, String)> {
    let rule6 = parts
        .iter()
        .find(|p| !p.is_empty())
        .is_some_and(|p| Keyword::prefix_of(p).is_some());
    let mut segments = Vec::new();
    let mut current: Option<Keyword> = None;
    let mut buf = String::new();
    for part in parts {
        let mut rest: &str = part;
        if let Some(k) = Keyword::prefix_of(rest) {
            if current.is_some() || !buf.is_empty() {
                segments.push((current, std::mem::take(&mut buf)));
            }
            current = Some(k);
            rest = &rest[5..];
        }
        while let Some((pos, k)) = current
            .filter(|_| rule6)
            .and_then(|c| next_later_keyword(rest, c))
        {
            buf.push_str(&rest[..pos]);
            segments.push((current, std::mem::take(&mut buf)));
            current = Some(k);
            rest = &rest[pos + 5..];
        }
        buf.push_str(rest);
    }
    if current.is_some() || !buf.is_empty() {
        segments.push((current, buf));
    }
    segments
}

/// The earliest keyword in `s` that comes after `current` in §5.1.4 order.
fn next_later_keyword(s: &str, current: Keyword) -> Option<(usize, Keyword)> {
    let rank = current.spec_rank()?;
    Keyword::ALL[rank + 1..10]
        .iter()
        .filter_map(|k| {
            let token = format!("{}+", k.as_str());
            s.find(&token).map(|pos| (pos, *k))
        })
        .min_by_key(|(pos, _)| *pos)
}

/// Whether `s` has a `?nn` subfield marker at byte `i`.
fn marker_at(s: &str, i: usize) -> bool {
    let b = s.as_bytes();
    b.len() >= i + 3 && b[i] == b'?' && b[i + 1].is_ascii_digit() && b[i + 2].is_ascii_digit()
}

/// Split text that starts with a `?nn` marker into `(code, value)` pairs.
/// Values are not trimmed (rule 4).
fn split_subfields(s: &str) -> Vec<(u8, &str)> {
    let starts: Vec<usize> = (0..s.len()).filter(|&i| marker_at(s, i)).collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = starts.get(n + 1).copied().unwrap_or(s.len());
            let b = s.as_bytes();
            let code = (b[start + 1] - b'0') * 10 + (b[start + 2] - b'0');
            (code, &s[start + 3..end])
        })
        .collect()
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn return_marker(svwz: &str) -> Option<ReturnMarker> {
    [
        ("REJECT", ReturnMarker::Reject),
        ("RETURN", ReturnMarker::Return),
        ("REFUND", ReturnMarker::Refund),
        ("RUECKLASTSCHRIFT", ReturnMarker::Ruecklastschrift),
    ]
    .into_iter()
    .find(|(prefix, _)| svwz.starts_with(prefix))
    .map(|(_, marker)| marker)
}

#[cfg(test)]
mod tests {
    use super::{Keyword, ReturnMarker, Subfield, parse};

    // Synthetic values only: test IBAN, test creditor ID, invented names.
    const IBAN: &str = "DE00123456780000000000";
    const BIC: &str = "TESTDEFFXXX";
    const CRED: &str = "DE98ZZZ09999999999";

    fn values(t: &super::Tag86) -> Vec<(&'static str, &str)> {
        t.sepa
            .iter()
            .map(|v| (v.keyword.as_str(), v.value.as_str()))
            .collect()
    }

    #[test]
    fn credit_transfer_like_spec_example() {
        // §5.1.5 shape: incoming SEPA credit transfer.
        let field = format!(
            "166?00SEPA-UEBERWEISUNG?100001?20EREF+REF-0001?21SVWZ+Rechnung 4711 vom?22 01.01.?30{BIC}?31{IBAN}?32Firma B"
        );
        let t = parse(&field);
        assert_eq!(t.gvc.as_deref(), Some("166"));
        assert!(t.structured);
        assert_eq!(t.posting_text.as_deref(), Some("SEPA-UEBERWEISUNG"));
        assert_eq!(t.primanota.as_deref(), Some("0001"));
        assert_eq!(t.bic.as_deref(), Some(BIC));
        assert_eq!(t.account.as_deref(), Some(IBAN));
        assert_eq!(t.name.as_deref(), Some("Firma B"));
        assert_eq!(
            values(&t),
            [("EREF", "REF-0001"), ("SVWZ", "Rechnung 4711 vom 01.01.")]
        );
        assert_eq!(t.purpose, "EREF+REF-0001SVWZ+Rechnung 4711 vom 01.01.");
        assert_eq!(t.plain_purpose, None);
        assert!(t.other.is_empty());
    }

    #[test]
    fn direct_debit_like_spec_example() {
        // §5.1.5 shape: SEPA direct debit charged to the account.
        let field = format!(
            "105?00SEPA-BASIS-LASTSCHRIFT?20EREF+E-0002?21MREF+M-0003?22CRED+{CRED}?23SVWZ+Beitrag Januar?30{BIC}?31{IBAN}?32Verein C"
        );
        let t = parse(&field);
        assert_eq!(
            values(&t),
            [
                ("EREF", "E-0002"),
                ("MREF", "M-0003"),
                ("CRED", CRED),
                ("SVWZ", "Beitrag Januar")
            ]
        );
        assert!(t.is_direct_debit());
        assert_eq!(t.return_marker, None);
    }

    #[test]
    fn value_continues_into_next_subfield() {
        let t = parse("166?20EREF+ABCDEFGHIJKLMNOPQRSTUV?21WXYZ0123?22SVWZ+Text");
        assert_eq!(t.raw(Keyword::Eref), Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123"));
        assert_eq!(t.raw(Keyword::Svwz), Some("Text"));
    }

    #[test]
    fn subfield_values_are_not_trimmed_before_the_join() {
        let t = parse("166?20SVWZ+Miete fuer ?21Wohnung 3 ");
        assert_eq!(t.raw(Keyword::Svwz), Some("Miete fuer Wohnung 3"));
    }

    #[test]
    fn purpose_in_6x_subfields_including_63() {
        let t = parse(
            "166?20SVWZ+Teil eins ?29Teil zwei ?32Name A?60Teil drei ?61Teil vier ?62Teil fuenf ?63Teil sechs",
        );
        assert_eq!(
            t.raw(Keyword::Svwz),
            Some("Teil eins Teil zwei Teil drei Teil vier Teil fuenf Teil sechs")
        );
        assert_eq!(t.name.as_deref(), Some("Name A"));
    }

    #[test]
    fn purpose_subfields_join_in_code_order() {
        let t = parse("166?60drei?21zwei?20SVWZ+eins ");
        assert_eq!(t.raw(Keyword::Svwz), Some("eins zweidrei"));
    }

    #[test]
    fn line_wrap_inside_keyword_and_code() {
        let t = parse("166?20EREF+E-0004?21?22K\r\nREF+K-0005?\r\n23SVWZ+Text\r\n weiter");
        assert_eq!(
            values(&t),
            [
                ("EREF", "E-0004"),
                ("KREF", "K-0005"),
                ("SVWZ", "Text weiter")
            ]
        );
    }

    #[test]
    fn placeholders_are_absent_but_raw_is_kept() {
        let t = parse("166?20EREF+NOTPROVIDED?21KREF+NONREF?22MREF+?23SVWZ+Text");
        assert_eq!(t.raw(Keyword::Eref), Some("NOTPROVIDED"));
        assert_eq!(t.value(Keyword::Eref), None);
        assert_eq!(t.raw(Keyword::Kref), Some("NONREF"));
        assert_eq!(t.value(Keyword::Kref), None);
        assert_eq!(t.raw(Keyword::Mref), Some(""));
        assert_eq!(t.value(Keyword::Mref), None);
        assert!(!t.is_direct_debit());
        assert_eq!(t.value(Keyword::Svwz), Some("Text"));
    }

    #[test]
    fn nonref_is_text_in_svwz() {
        let t = parse("166?20SVWZ+NONREF");
        assert_eq!(t.value(Keyword::Svwz), Some("NONREF"));
    }

    #[test]
    fn empty_eref_is_absent() {
        let t = parse("166?20EREF+?21SVWZ+Text");
        assert_eq!(t.raw(Keyword::Eref), Some(""));
        assert_eq!(t.value(Keyword::Eref), None);
    }

    #[test]
    fn no_keywords_is_plain_purpose() {
        let t = parse("005?00LASTSCHRIFT?20Miete Januar ?21Wohnung 3?32Name A");
        assert_eq!(t.gvc.as_deref(), Some("005"));
        assert!(t.sepa.is_empty());
        assert_eq!(t.purpose, "Miete Januar Wohnung 3");
        assert_eq!(t.plain_purpose.as_deref(), Some("Miete Januar Wohnung 3"));
    }

    #[test]
    fn text_before_first_keyword_is_plain() {
        let t = parse("166?20Vorab ?21SVWZ+Text");
        assert_eq!(t.plain_purpose.as_deref(), Some("Vorab"));
        assert_eq!(t.raw(Keyword::Svwz), Some("Text"));
    }

    #[test]
    fn gvc_999_is_unstructured() {
        let t = parse("999Freier Text?20ohne Struktur");
        assert_eq!(t.gvc.as_deref(), Some("999"));
        assert!(!t.structured);
        assert_eq!(t.purpose, "Freier Text?20ohne Struktur");
        assert_eq!(
            t.plain_purpose.as_deref(),
            Some("Freier Text?20ohne Struktur")
        );
        assert!(t.sepa.is_empty());
    }

    #[test]
    fn field_without_subfields_is_unstructured() {
        let t = parse("Gutschrift ohne GVC");
        assert_eq!(t.gvc, None);
        assert!(!t.structured);
        assert_eq!(t.purpose, "Gutschrift ohne GVC");
    }

    #[test]
    fn unstructured_text_with_keywords_splits_in_spec_order() {
        // Rule 6: later keywords split anywhere; SVWZ only ends at ABWA/ABWE.
        let t = parse("999EREF+E-0006 KREF+K-0007 SVWZ+Zahlung EREF+ X ABWA+Firma B");
        assert_eq!(
            values(&t),
            [
                ("EREF", "E-0006"),
                ("KREF", "K-0007"),
                ("SVWZ", "Zahlung EREF+ X"),
                ("ABWA", "Firma B")
            ]
        );
    }

    #[test]
    fn keywords_mid_subfield_split_only_in_spec_order() {
        let t = parse("166?20EREF+E-0008 SVWZ+Text MREF+M ?21ABWE+Firma B");
        assert_eq!(
            values(&t),
            [
                ("EREF", "E-0008"),
                ("SVWZ", "Text MREF+M"),
                ("ABWE", "Firma B")
            ]
        );
    }

    #[test]
    fn keyword_mid_subfield_without_leading_keyword_does_not_split() {
        // Decision 2a: text that does not start with a keyword stays plain.
        let t = parse("166?20Kauf SVWZ+Text");
        assert!(t.sepa.is_empty());
        assert_eq!(t.plain_purpose.as_deref(), Some("Kauf SVWZ+Text"));
    }

    #[test]
    fn unknown_keyword_stays_in_value() {
        let t = parse("166?20SVWZ+Text ?21ABCD+mehr?22XYZ+ende");
        assert_eq!(t.raw(Keyword::Svwz), Some("Text ABCD+mehrXYZ+ende"));
    }

    #[test]
    fn repeated_keyword_is_joined() {
        let t = parse("166?20SVWZ+Teil eins ?21SVWZ+Teil zwei");
        assert_eq!(t.raw(Keyword::Svwz), Some("Teil eins Teil zwei"));
    }

    #[test]
    fn name_with_plus_and_continuation() {
        let t = parse(&format!(
            "166?20SVWZ+Text?31{IBAN}?32Firma B + Partner Han?33del GmbH"
        ));
        assert_eq!(t.name.as_deref(), Some("Firma B + Partner Handel GmbH"));
        assert_eq!(t.raw(Keyword::Svwz), Some("Text"));
    }

    #[test]
    fn empty_and_unknown_subfields_never_fail() {
        let t = parse(&format!("166?00?11X1?20SVWZ+Text?30{BIC} 12345?34?99"));
        assert_eq!(t.posting_text, None);
        assert_eq!(t.text_key_extension, None);
        assert_eq!(t.bic.as_deref(), Some(BIC));
        assert_eq!(t.raw(Keyword::Svwz), Some("Text"));
        assert_eq!(
            t.other,
            [
                Subfield {
                    code: "11".into(),
                    value: "X1".into()
                },
                Subfield {
                    code: "99".into(),
                    value: String::new()
                },
            ]
        );
    }

    #[test]
    fn text_key_extension() {
        let t = parse("166?20SVWZ+Text?34992");
        assert_eq!(t.text_key_extension.as_deref(), Some("992"));
    }

    #[test]
    fn characters_are_kept() {
        let t = parse("166?00Überweisung?20SVWZ+Grüße aus Straße ä?32Bäckerei D");
        assert_eq!(t.posting_text.as_deref(), Some("Überweisung"));
        assert_eq!(t.raw(Keyword::Svwz), Some("Grüße aus Straße ä"));
        assert_eq!(t.name.as_deref(), Some("Bäckerei D"));
    }

    #[test]
    fn long_field_of_568_characters_is_kept_whole() {
        let piece = |n: usize| format!("{:<27}", format!("Teil {n:02} Text"));
        let mut field = format!("166?00{}?101234567890", piece(80));
        let mut expected = String::new();
        for (i, code) in (20..=29).chain(60..=63).enumerate() {
            let text = if i == 0 {
                format!("SVWZ+{}", &piece(i)[5..])
            } else {
                piece(i)
            };
            expected.push_str(&text);
            field.push_str(&format!("?{code}{text}"));
        }
        field.push_str(&format!(
            "?30{BIC}?31{IBAN}?32{}?33{}?34997",
            piece(90),
            piece(91)
        ));
        assert!(field.len() >= 568, "{}", field.len());
        let t = parse(&field);
        assert_eq!(t.purpose, expected.trim());
        assert_eq!(t.raw(Keyword::Svwz), Some(expected[5..].trim()));
        assert_eq!(t.text_key_extension.as_deref(), Some("997"));
    }

    #[test]
    fn r_transaction_uses_original_direction_for_abwa() {
        // A returned direct debit, credited back: the original was a debit
        // initiated by the creditor, so ABWA names the counterparty's side.
        let t = parse(&format!(
            "109?20EREF+E-0009?21MREF+M-0010?22CRED+{CRED}?23SVWZ+RUECKLASTSCHRIFT Beitrag?24ABWA+Firma B?25ABWE+Name A"
        ));
        assert_eq!(t.return_marker, Some(ReturnMarker::Ruecklastschrift));
        assert_eq!(t.ultimate_creditor(), Some("Firma B"));
        assert_eq!(t.ultimate_debtor(), Some("Name A"));
    }

    #[test]
    fn return_markers() {
        for (svwz, marker) in [
            ("REJECT AC04", ReturnMarker::Reject),
            ("RETURN MD06", ReturnMarker::Return),
            ("REFUND", ReturnMarker::Refund),
        ] {
            assert_eq!(
                parse(&format!("159?20SVWZ+{svwz}")).return_marker,
                Some(marker)
            );
        }
        assert_eq!(parse("166?20SVWZ+Return of goods").return_marker, None);
    }

    #[test]
    fn ultimate_parties_follow_the_payment_kind() {
        let transfer = parse("166?20SVWZ+Text?21ABWA+Firma B?22ABWE+Name A");
        assert_eq!(transfer.ultimate_debtor(), Some("Firma B"));
        assert_eq!(transfer.ultimate_creditor(), Some("Name A"));
        let debit = parse("105?20MREF+M-1?21SVWZ+Text?22ABWA+Firma B?23ABWE+Name A");
        assert_eq!(debit.ultimate_creditor(), Some("Firma B"));
        assert_eq!(debit.ultimate_debtor(), Some("Name A"));
    }

    #[test]
    fn return_reason_and_amounts() {
        let t = parse("159?20SVWZ+RETURN MD06?21OAMT+12,50?22COAM+1,5");
        assert_eq!(t.return_reason(), Some("MD06"));
        assert_eq!(t.amount(Keyword::Oamt), Some("12.50".parse().unwrap()));
        assert_eq!(t.amount(Keyword::Coam), Some("1.5".parse().unwrap()));
        assert_eq!(parse("159?20SVWZ+REFUND").return_reason(), None);
        assert_eq!(parse("166?20SVWZ+RETURN of goods").return_reason(), None);
        assert_eq!(
            parse("159?20SVWZ+RETURN AC04").return_reason(),
            Some("AC04")
        );
        assert_eq!(parse("159?20OAMT+EUR 5").amount(Keyword::Oamt), None);
    }

    #[test]
    fn consorsbank_shape() {
        // Skeleton of the masked trace: legacy GVC, EREF continuing into
        // ?21, KREF+NONREF, SVWZ over ?23/?24, CRLF wraps at 65 characters
        // that split a keyword and a subfield code. Synthetic fill.
        let field = format!(
            "016?00LASTSCHRIFT?20EREF+NOTPROVIDED?21?22K\r\nREF+NONREF?23SVWZ+Kauf 0001,?24 Datum 01.01.?30{BIC}?\r\n31{IBAN}?32Firma B Handel und Vertr?33ieb + Co"
        );
        let wrapped: Vec<_> = field.split("\r\n").collect();
        assert!(wrapped.iter().all(|l| l.len() <= 65), "{wrapped:?}");
        let t = parse(&field);
        assert_eq!(t.gvc.as_deref(), Some("016"));
        assert_eq!(t.value(Keyword::Eref), None);
        assert_eq!(t.raw(Keyword::Eref), Some("NOTPROVIDED"));
        assert_eq!(t.value(Keyword::Kref), None);
        assert_eq!(t.value(Keyword::Svwz), Some("Kauf 0001, Datum 01.01."));
        assert_eq!(t.plain_purpose, None);
        assert_eq!(t.bic.as_deref(), Some(BIC));
        assert_eq!(t.account.as_deref(), Some(IBAN));
        assert_eq!(t.name.as_deref(), Some("Firma B Handel und Vertrieb + Co"));

        // EREF value continuing into ?21, as in the trace.
        let t = parse("008?20EREF+0123456789012345678901?21234?22KREF+NONREF?23SVWZ+Text");
        assert_eq!(t.value(Keyword::Eref), Some("0123456789012345678901234"));
    }

    mod props {
        use super::super::{Tag86, parse};
        use proptest::prelude::*;

        fn rebuilt(t: &Tag86) -> String {
            t.segments
                .iter()
                .map(|(k, text)| match k {
                    Some(k) => format!("{}+{text}", k.as_str()),
                    None => text.clone(),
                })
                .collect()
        }

        fn no_ws(s: &str) -> String {
            s.chars().filter(|c| !c.is_whitespace()).collect()
        }

        fn subfield_text() -> impl Strategy<Value = String> {
            let piece = prop_oneof![
                "\\?[0-9]{2}",
                Just("EREF+".to_owned()),
                Just("SVWZ+".to_owned()),
                Just("ABWA+".to_owned()),
                Just("KREF+".to_owned()),
                Just("\r\n".to_owned()),
                "[ A-Za-z0-9+?äß]{0,12}",
            ];
            ("[0-9]{3}", prop::collection::vec(piece, 0..30))
                .prop_map(|(gvc, pieces)| format!("{gvc}{}", pieces.concat()))
        }

        proptest! {
            #[test]
            fn never_panics_on_arbitrary_text(s in any::<String>()) {
                let t = parse(&s);
                prop_assert_eq!(no_ws(&rebuilt(&t)), no_ws(&t.purpose));
            }

            #[test]
            fn purpose_is_kept_and_wraps_do_not_matter(s in subfield_text(), at in any::<prop::sample::Index>()) {
                let t = parse(&s);
                // Keyword tokens plus values give the whole purpose back.
                prop_assert_eq!(no_ws(&rebuilt(&t)), no_ws(&t.purpose));
                // A line break anywhere (on a char boundary) changes nothing.
                let mut cut = at.index(s.len() + 1);
                while !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                let wrapped = format!("{}\r\n{}", &s[..cut], &s[cut..]);
                prop_assert_eq!(parse(&wrapped), t);
            }
        }
    }
}
