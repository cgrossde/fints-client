//! Masking of FinTS messages for logs and tooling.
//!
//! Every message that crosses the transport boundary (request and response)
//! is logged through [`mask_for_log`]. The function is segment-aware: it
//! tokenizes with `codec::wire` (so `?` escapes and `@len@` binary
//! data are respected), looks up per segment code and version which data
//! elements hold credentials or identifiers, and writes the message again
//! with those elements replaced.
//!
//! # The rule
//!
//! - A masked text element becomes one `*` per character: the length class
//!   stays visible, the value does not. (No prefix or suffix of an IBAN is
//!   kept.) PIN and TAN are always eight stars: their length says something.
//! - Names and amounts are shown as a fixed shape (`shape`): a name is
//!   eight `#` whatever its length, an amount is `#,##` whatever its size
//!   and its decimals. Holder and user names (HIUPA 4, HIUPD 6 6 and
//!   7), product names (HIUPD 6 8, HISAL 2), the extensions (HIUPA 5,
//!   HIUPD 6 12) and every amount in HISAL and HIUPD 6.
//! - A binary element (MT940, camt, MT535/536, keys, certificates) becomes
//!   `@<original length>@<masked>` in the log and `@n@` + `n` stars in
//!   [`redact_credentials`]. Binary data is never shown. Only the trace
//!   recorder (`mask_for_recorder`) shows statements as their shape, field
//!   by field, with the length of the shaped data in `@len@`: a trace is a reviewed artefact, the log goes to any sink, so the
//!   log stays the more masked of the two.
//! - HNVSD carries the signed segments as binary data; its contents are
//!   masked recursively and written in place, so the log shows the whole
//!   dialog.
//! - Masked with stars in the log, with a stand-in in a trace
//!   (`pseudonym`: equal values give equal stand-ins; a counterparty's
//!   IBAN, a creditor ID and a reference have one fixed form each). In a
//!   trace every date is moved by the same number of days (`dates`). A per-trace alias for the account identifiers: user
//!   ID / customer ID (HKIDN, HNSHK, HNVSK, HIUPA, HIUPD); account number
//!   and IBAN in every account group of HKSAL/HISAL, HKKAZ/HIKAZ,
//!   HKCAZ/HICAZ, HKWPD, HKWDU, HKSPA/HISPA, HKTAN and HIUPD. A keyed
//!   pseudonym for the system ID (HKIDN, HNSHK, HNVSK, HISYN). Always starred: PIN and
//!   TAN (HNSHA), subaccounts, the TAN challenge (HITAN). BIC and bank code
//!   are kept.
//! - Payment orders (HKCCS, HKIPZ, HKIPS, HKVPP, HKVPA and their answers
//!   HIIPZ, HIIPS, HIVPP): the debtor account (KTI), the instant order ID,
//!   the payee IBAN in HIVPP and the binary VOP-ID and polling ID are
//!   starred in the log; in a trace the accounts get aliases, the IDs
//!   pseudonyms; the VoP touchdown
//!   point and the payee's additional information, registered name, other
//!   ID and RVNA reason are starred; the explanation text of HIVPP is shown
//!   as its shape. The pain.001/pain.002 messages are binary and so starred
//!   in the log; the trace recorder shows them as their shape (`shape`:
//!   markup, status and service level codes and currency stay; names, BICs,
//!   dates, remittance text and amounts do not) with aliases for IBANs,
//!   account numbers and creditor IDs and pseudonyms for references. Descriptors, flags, outcome codes and
//!   waits are kept.
//! - Segments known to carry only protocol data (envelope, feedback,
//!   parameter segments `HI…S`, HKEND, HKSYN, and HKVVB except its product registration, which is starred) are kept; feedback texts
//!   have IBAN-like words masked (an alias in a trace). Every other segment is unknown and masked
//!   completely except its header (better too much than too little); the
//!   recorder can keep the ones a person has looked at (`hide_unknown`).
//! - Input that does not tokenize is never shown: the log gets only a length.
//!
//! The function never panics. It is a logging aid, not a security boundary:
//! a bank that sends an identifier in a place the table does not list, inside
//! a known segment, is not caught.

// Used by the trace recorder only; the log stars identity values.
pub(crate) mod dates;
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) mod pseudonym;
pub(crate) mod shape;

use crate::codec::wire;
use crate::parser::{DEG, DataElement};
use pseudonym::{IdClass, Identities};

/// Longest nesting of HNVSD inside HNVSD that is looked into.
const MAX_DEPTH: usize = 2;

/// How binary data and unparseable input are shown.
#[derive(Clone, Copy)]
enum Mode<'a> {
    /// `@n@<masked>`; unparseable input becomes a length marker.
    Log,
    /// `@n@` followed by `n` stars: the result is valid wire syntax again
    /// (for tooling that parses the masked message).
    Parseable,
    /// Like `Parseable`, but statements (HIKAZ, HICAZ, HIWPD, HIWDU) and
    /// pain messages are shape-masked by field instead of starred
    /// ([`shape`]), and identity values get aliases or keyed pseudonyms
    /// instead of stars (`mask::pseudonym`).
    #[cfg_attr(not(feature = "trace"), allow(dead_code))]
    Trace(&'a Identities),
    /// Like `Trace`, but a segment the table does not know stays as it is
    /// ([`is_unknown`]). The recorder holds messages in this form until the
    /// caller has decided which unknown segments a trace may show
    /// ([`hide_unknown`]).
    #[cfg_attr(not(feature = "trace"), allow(dead_code))]
    Recorder(&'a Identities),
}

/// Masks a message (or the clear text of an HNVSD) for logging. The result
/// is not wire syntax (binary data are shown as `@n@<masked>`).
pub fn mask_for_log(message: &[u8]) -> String {
    mask(message, Mode::Log, 0)
        .into_iter()
        .map(char::from)
        .collect()
}

/// Masks a message like [`mask_for_log`], but keeps it parseable: binary
/// data are replaced by as many `*` as they were long, and input that does
/// not tokenize keeps only its separators. Used by `debug` before it parses
/// a message for display.
pub fn redact_credentials(message: &[u8]) -> Vec<u8> {
    mask(message, Mode::Parseable, 0)
}

/// Masks a message like [`redact_credentials`], but shows the statements
/// in HIKAZ (MT940), HICAZ (camt), HIWPD and HIWDU (MT535/536) and the
/// pain messages as their shape: markup and codes stay, every letter,
/// digit and space of values becomes `#` ([`shape`]). Account identifiers
/// (IBAN, account number, user and customer ID, creditor ID) get per-trace
/// aliases, the other identity values (system ID, booking and payment
/// references, order, VOP and polling IDs) keyed pseudonyms
/// (`mask::pseudonym`), instead of stars or shapes. The trace recorder uses
/// [`mask_for_recorder`] and [`hide_unknown`], which together give this.
#[cfg(test)]
pub(crate) fn mask_for_trace(message: &[u8], ids: &Identities) -> Vec<u8> {
    mask(message, Mode::Trace(ids), 0)
}

/// [`mask_for_trace`] with the unknown segments left as they are. Pass the
/// result through [`hide_unknown`] before it leaves the recorder.
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) fn mask_for_recorder(message: &[u8], ids: &Identities) -> Vec<u8> {
    mask(message, Mode::Recorder(ids), 0)
}

/// A segment the table of masked fields does not know: a trace shows only
/// its header and the length of each element, unless the person recording
/// looked at it and chose to keep it. HIUPD in a version without a field
/// list is never offered: it holds the account and its holder.
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) fn is_unknown(code: &str, version: u16) -> bool {
    !code.is_empty() && code != "HIUPD" && Policy::of(code, version) == Policy::All
}

/// A message of [`mask_for_recorder`] with every unknown segment starred
/// after its header, except those `keep` names (by segment code). Every
/// other segment is written back unchanged.
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) fn hide_unknown(input: &[u8], keep: &dyn Fn(&str) -> bool) -> Vec<u8> {
    let Ok(segments) = wire::tokenize(input) else {
        return input.to_vec();
    };
    let mut out = Vec::with_capacity(input.len());
    for seg in &segments {
        let (code, version) = (seg.segment_type(), seg.segment_version());
        let hide = is_unknown(code, version) && !keep(code);
        for (i, deg) in seg.degs.iter().enumerate() {
            if i > 0 {
                out.push(b'+');
            }
            for (j, de) in deg.0.iter().enumerate() {
                if j > 0 {
                    out.push(b':');
                }
                match de {
                    DataElement::Empty => {}
                    DataElement::Binary(data) => {
                        let shown = if code == "HNVSD" && i == 1 && j == 0 {
                            hide_unknown(data, keep)
                        } else if hide && i > 0 {
                            vec![b'*'; data.len()]
                        } else {
                            data.clone()
                        };
                        out.extend_from_slice(format!("@{}@", shown.len()).as_bytes());
                        out.extend(shown);
                    }
                    DataElement::Text(text) if hide && i > 0 => {
                        out.extend(std::iter::repeat_n(b'*', text.chars().count()));
                    }
                    DataElement::Text(text) => write_text(text, &mut out),
                }
            }
        }
        out.push(b'\'');
    }
    out
}

/// The unknown segments of a message of [`mask_for_recorder`], each as its
/// wire bytes, those inside HNVSD included.
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) fn unknown_segments(input: &[u8]) -> Vec<Vec<u8>> {
    let Ok(segments) = wire::tokenize(input) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for seg in &segments {
        if seg.segment_type() == "HNVSD"
            && let Some(DataElement::Binary(inner)) = seg.degs.get(1).and_then(|d| d.0.first())
        {
            found.extend(unknown_segments(inner));
        } else if is_unknown(seg.segment_type(), seg.segment_version())
            && let Ok(bytes) = wire::serialize_segment(seg)
        {
            found.push(bytes);
        }
    }
    found
}

/// Target of the one `TRACE` event the library emits per exchange (success
/// or failure), for callers that record a dialog (e.g. as a replay fixture).
/// It is emitted around the [`Transport`](crate::transport::Transport) call,
/// so no transport can skip it.
///
/// Field `request`: the message as sent (with the PIN/TAN envelope, after the
/// outgoing quirks), passed through [`redact_credentials`] and given as
/// ISO 8859-1 text (one `char` per byte). On success, field `response`: the
/// answer as received (before the incoming quirks), redacted the same way.
/// On failure there is no `response`; field `error` holds a short summary
/// (`HTTP 500`, or `transport: <text>`). An HTTP body is never logged, because
/// it may echo request data. Enable it with a `tracing` subscriber for this
/// target; the library itself stores nothing.
pub const EXCHANGE_TARGET: &str = "fints_client::exchange";

/// [`redact_credentials`] as ISO 8859-1 text: one `char` per byte, so
/// `text.chars().map(|c| c as u8)` gives the bytes back.
pub fn redacted_text(message: &[u8]) -> String {
    redact_credentials(message)
        .into_iter()
        .map(char::from)
        .collect()
}

fn mask(input: &[u8], mode: Mode<'_>, depth: usize) -> Vec<u8> {
    let Ok(segments) = wire::tokenize(input) else {
        return match mode {
            Mode::Log => format!("<{} bytes, unparseable, masked>", input.len()).into_bytes(),
            Mode::Parseable | Mode::Trace(_) | Mode::Recorder(_) => input
                .iter()
                .map(|&b| {
                    if matches!(b, b'+' | b':' | b'\'') {
                        b
                    } else {
                        b'*'
                    }
                })
                .collect(),
        };
    };
    let mut out = Vec::with_capacity(input.len());
    for seg in &segments {
        write_segment(seg.degs.as_slice(), mode, depth, &mut out);
    }
    out
}

fn write_segment(degs: &[DEG], given: Mode<'_>, depth: usize, out: &mut Vec<u8>) {
    let (mode, keep_unknown) = match given {
        Mode::Recorder(ids) => (Mode::Trace(ids), true),
        mode => (mode, false),
    };
    let ids = match mode {
        Mode::Trace(ids) | Mode::Recorder(ids) => Some(ids),
        Mode::Log | Mode::Parseable => None,
    };
    let code = match degs.first().and_then(|d| d.0.first()) {
        Some(DataElement::Text(s)) => s.as_str(),
        _ => "",
    };
    let version = degs
        .first()
        .and_then(|d| d.0.get(2))
        .and_then(DataElement::as_str)
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(0);
    let policy = Policy::of(code, version);
    let envelope = code == "HNVSD" && depth < MAX_DEPTH;
    let clear = keep_unknown && is_unknown(code, version);

    for (i, deg) in degs.iter().enumerate() {
        if i > 0 {
            out.push(b'+');
        }
        for (j, de) in deg.0.iter().enumerate() {
            if j > 0 {
                out.push(b':');
            }
            match de {
                DataElement::Empty => {}
                DataElement::Binary(data) if envelope && i == 1 && j == 0 => {
                    let inner = mask(data, given, depth + 1);
                    out.extend_from_slice(format!("@{}@", inner.len()).as_bytes());
                    out.extend_from_slice(&inner);
                }
                DataElement::Binary(data) if clear => {
                    out.extend_from_slice(format!("@{}@", data.len()).as_bytes());
                    out.extend_from_slice(data);
                }
                DataElement::Text(text) if clear => write_text(text, out),
                DataElement::Binary(data) => {
                    let format = shape::Format::of(code).or_else(|| shape::Format::pain(code, i));
                    // A shaped statement has its own length: names and
                    // amounts are written in fixed shapes.
                    let shown = match (mode, format, binary_id(code, version, i)) {
                        (Mode::Log, _, _) => {
                            out.extend_from_slice(format!("@{}@<masked>", data.len()).as_bytes());
                            continue;
                        }
                        (Mode::Trace(ids), Some(format), _) => {
                            shape::shape(format, data, Some(ids))
                        }
                        (Mode::Trace(ids), None, Some(class)) => ids.bytes(class, data),
                        _ => vec![b'*'; data.len()],
                    };
                    out.extend_from_slice(format!("@{}@", shown.len()).as_bytes());
                    out.extend(shown);
                }
                DataElement::Text(text) => {
                    let shape = if i > 0 {
                        shaped(code, version, i, deg.0.len(), text)
                    } else {
                        None
                    };
                    // A balance of HISAL (`C:1,00:EUR:20260101:120000`) in
                    // a trace: the debit mark is written as credit, so an
                    // overdrawn account does not show, and its date moves
                    // with the other dates of the trace.
                    let balance = match ids {
                        Some(ids) if code == "HISAL" && i >= 3 => {
                            if j == 0 && text == "D" && deg.0.len() >= 3 {
                                Some("C".to_string())
                            } else if dates::is_date(text.as_bytes(), dates::Form::Ymd8) {
                                Some(dates::shift_text(text, ids.days()))
                            } else if j > 0
                                && deg.0[j - 1].as_str().is_some_and(|d| {
                                    dates::is_date(d.as_bytes(), dates::Form::Ymd8)
                                })
                            {
                                Some(dates::noon_text(text))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    let shown = if let Some(balance) = balance {
                        balance
                    } else if let Some(shape) = shape {
                        match shape {
                            Shape::Name(limit) => shape::name_shape(text.chars().count(), limit),
                            Shape::Amount => shape::amount_shape(text),
                            Shape::Text => shape::shape_text(text),
                        }
                    } else if let Some(hide) = policy.masks(i, j).filter(|_| i > 0) {
                        match (hide, ids) {
                            (Hide::Id(class), Some(ids)) => ids.text(class, text),
                            (Hide::Date, Some(ids)) => dates::shift_text(text, ids.days()),
                            (Hide::Time, Some(_)) => dates::noon_text(text),
                            (Hide::Date | Hide::Time, None) => text.clone(),
                            // PIN and TAN: a fixed length, so the trace does
                            // not say how long the PIN is.
                            _ if code == "HNSHA" => "*".repeat(SECRET_LEN),
                            _ => "*".repeat(text.chars().count()),
                        }
                    } else if i > 0 && policy == Policy::Feedback {
                        scrub_ibans(text, ids)
                    } else {
                        text.clone()
                    };
                    write_text(&shown, out);
                }
            }
        }
    }
    out.push(b'\'');
}

/// Stars written for a PIN or a TAN, whatever its length.
const SECRET_LEN: usize = 8;

/// How a free text or an amount of a segment is shown, in every mode.
enum Shape {
    /// A name whose format allows this many characters
    /// ([`shape::name_shape`]): `Erika Beispiel` → `########`.
    Name(usize),
    /// [`shape::amount_shape`]: `1234,56` → `#,##`.
    Amount,
    /// [`shape::shape_text`]: eight `#` per line.
    Text,
}

/// Free text (names, product names, extensions) and amounts, which are
/// shown as a shape. Positions count data elements from 1.
fn shaped(code: &str, version: u16, de: usize, group_len: usize, text: &str) -> Option<Shape> {
    let amount = || shape::is_amount(text).then_some(Shape::Amount);
    match code {
        // 4 Benutzername (an ..35), 5 Erweiterung (an ..2048).
        "HIUPA" if de == 4 => Some(Shape::Name(35)),
        "HIUPA" if de == 5 => Some(Shape::Name(2048)),
        // 6, 7 Name des Kontoinhabers (an ..27), 8 Kontoproduktbezeichnung
        // (an ..30); amounts in 9 Kontolimit and the allowed orders; 12
        // Erweiterung (an ..2048) follows the repeated allowed orders (data
        // elements 10 on, each at least two group elements) as a single
        // element.
        "HIUPD" if version == 6 => match de {
            6 | 7 => Some(Shape::Name(27)),
            8 => Some(Shape::Name(30)),
            _ if de >= 10 && group_len == 1 => Some(Shape::Name(2048)),
            _ if de >= 9 => amount(),
            _ => None,
        },
        // 2 Kontoproduktbezeichnung (an ..30); balances, credit line,
        // available and used amounts.
        "HISAL" if de == 2 => Some(Shape::Name(30)),
        "HISAL" if de >= 3 => amount(),
        // 8 Aufklaerungstext Autorisierung trotz Abweichung: may quote the
        // names.
        "HIVPP" if version == 1 && de == 7 && group_len == 1 => Some(Shape::Text),
        _ => None,
    }
}

/// Writes text as ISO 8859-1 with the FinTS escapes.
fn write_text(text: &str, out: &mut Vec<u8>) {
    for c in text.chars() {
        let byte = u8::try_from(u32::from(c)).unwrap_or(b'?');
        if matches!(byte, b'?' | b'+' | b':' | b'\'' | b'@') {
            out.push(b'?');
        }
        out.push(byte);
    }
}

/// Replaces words that look like an IBAN (two letters, two digits, at least
/// 15 letters and digits in all) with `*`, in a trace with their alias.
fn scrub_ibans(text: &str, ids: Option<&Identities>) -> String {
    let flush = |word: &mut String, out: &mut String| {
        let chars: Vec<char> = word.chars().collect();
        let iban_like = chars.len() >= 15
            && chars[..2].iter().all(char::is_ascii_uppercase)
            && chars[2..4].iter().all(char::is_ascii_digit);
        match ids {
            Some(ids) if iban_like => out.push_str(&ids.text(IdClass::OtherIban, word)),
            None if iban_like => out.extend(std::iter::repeat_n('*', chars.len())),
            _ => out.push_str(word),
        }
        word.clear();
    };
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// How a masked data element is hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hide {
    /// One `*` per character, in every mode.
    Star,
    /// Stars in the log, the stand-in of this class in a trace (an alias
    /// or a keyed pseudonym).
    Id(IdClass),
    /// A date (`YYYYMMDD`): as sent in the log, moved by the days of the
    /// trace in a trace (`dates`).
    Date,
    /// A time of day (`hhmmss`): as sent in the log, noon in a trace.
    Time,
}

use Hide::{Id, Star};

/// Which data elements of a segment are masked. Positions count data
/// elements from 1 (the header is 0, never masked) and group elements from 0.
#[derive(Debug, PartialEq, Eq)]
enum Policy {
    /// Only binary data is masked.
    Safe,
    /// Like `Safe`, IBAN-like words in the text are masked.
    Feedback,
    /// The listed (data element, group elements, how) entries; no group
    /// elements listed means the whole data element.
    At(Vec<(usize, Vec<usize>, Hide)>),
    /// Every data element from this position on, with stars.
    From(usize),
    /// Unknown segment: everything after the header, with stars.
    All,
}

/// Group elements of a data element and how they are hidden.
type Parts = &'static [(&'static [usize], Hide)];

/// `Ktv3` (number, subaccount): the number identifies the account.
const KTV: Parts = &[(&[0], Id(IdClass::Account)), (&[1], Star)];
/// `Kti1` (IBAN, BIC, number, subaccount, …); the BIC is kept.
const KTI: Parts = &[
    (&[0], Id(IdClass::Iban)),
    (&[2], Id(IdClass::Account)),
    (&[3], Star),
];
/// `Ktz1` (use, IBAN, BIC, number, subaccount, …); the BIC is kept.
const KTZ: Parts = &[
    (&[1], Id(IdClass::Iban)),
    (&[3], Id(IdClass::Account)),
    (&[4], Star),
];
/// Sicherheitsidentifikation, Details (HNSHK 6, HNVSK 4): CID and
/// Kundensystem-ID.
const SECURITY_ID: Parts = &[(&[1], Star), (&[2], Id(IdClass::SystemId))];
/// Schlüsselname (HNSHK 11, HNVSK 7): the Benutzerkennung.
const KEY_NAME: Parts = &[(&[2], Id(IdClass::UserId))];
const USER: Parts = &[(&[], Id(IdClass::UserId))];
const SYSTEM: Parts = &[(&[], Id(IdClass::SystemId))];
const ORDER: Parts = &[(&[], Id(IdClass::OrderId))];
const IBAN: Parts = &[(&[], Id(IdClass::Iban))];
const STARS: Parts = &[(&[], Star)];
const DATE: Parts = &[(&[], Hide::Date)];
/// Sicherheitsdatum und -uhrzeit (HNSHK 8, HNVSK 5): type, date, time.
const SECURITY_DATE: Parts = &[(&[1], Hide::Date), (&[2], Hide::Time)];
/// Gültigkeit der VOP-ID (HIVPP 2): date, time.
const VALIDITY: Parts = &[(&[0], Hide::Date), (&[1], Hide::Time)];
/// HIVPP 6 Ergebnis Einzeltransaktion: the IBAN; IBAN info, registered
/// name, other ID and reason RVNA starred; the outcome (4) stays.
const PAYEE: Parts = &[(&[0], Id(IdClass::OtherIban)), (&[1, 2, 3, 5], Star)];

impl Policy {
    fn of(code: &str, version: u16) -> Policy {
        fn at(list: &[(usize, Parts)]) -> Policy {
            Policy::At(
                list.iter()
                    .flat_map(|&(d, parts)| parts.iter().map(move |&(e, h)| (d, e.to_vec(), h)))
                    .collect(),
            )
        }
        match code {
            "HNHBK" | "HNHBS" | "HNVSD" | "HKEND" | "HKSYN" => Policy::Safe,
            // 4 Produktbezeichnung: the product registration number.
            "HKVVB" => at(&[(4, STARS)]),
            "HIRMG" | "HIRMS" => Policy::Feedback,
            "HNSHA" => Policy::From(3),
            // 2 Kunden-ID, 3 Kundensystem-ID.
            "HKIDN" => at(&[(2, USER), (3, SYSTEM)]),
            "HNSHK" => at(&[(6, SECURITY_ID), (8, SECURITY_DATE), (11, KEY_NAME)]),
            "HNVSK" => at(&[(4, SECURITY_ID), (5, SECURITY_DATE), (7, KEY_NAME)]),
            "HISYN" => at(&[(1, SYSTEM)]),
            "HIUPA" => at(&[(1, USER)]),
            "HIUPD" if version == 6 => at(&[(1, KTV), (2, IBAN), (3, USER)]),
            "HIUPD" => Policy::All,
            "HISAL" if version >= 7 => at(&[(1, KTI)]),
            "HISAL" => at(&[(1, KTV)]),
            // 3 Von Datum, 4 Bis Datum.
            "HKKAZ" if version >= 7 => at(&[(1, KTI), (3, DATE), (4, DATE)]),
            "HKKAZ" => at(&[(1, KTV), (3, DATE), (4, DATE)]),
            "HKSAL" | "HIKAZ" if version >= 7 => at(&[(1, KTI)]),
            // 4 Von Datum, 5 Bis Datum.
            "HKWDU" => at(&[(1, KTV), (4, DATE), (5, DATE)]),
            "HKSAL" | "HIKAZ" | "HKWPD" | "HIWPD" | "HIWDU" => at(&[(1, KTV)]),
            "HKCAZ" => at(&[(1, KTI), (4, DATE), (5, DATE)]),
            "HICAZ" => at(&[(1, KTI)]),
            "HKTAN" => at(&[(3, KTI)]),
            // Payment orders (docs/payments-design.md): the debtor account,
            // the order IDs and the payee check IDs are masked. The
            // descriptor, flags, outcome codes and waits are kept. The pain
            // messages (binary) are starred, or shaped in a trace.
            "HKCCS" | "HKIPZ" if version <= 2 => at(&[(1, KTI)]),
            "HKIPS" if version == 1 => at(&[(1, KTI), (3, ORDER)]),
            "HIIPS" if version == 1 => at(&[(1, KTI), (4, ORDER)]),
            "HIIPZ" if version <= 2 => at(&[(1, ORDER)]),
            // 4 Aufsetzpunkt.
            "HKVPP" if version == 1 => at(&[(4, STARS)]),
            "HKVPA" if version == 1 => Policy::Safe,
            // 2 Gültigkeit der VOP-ID: date, time.
            "HIVPP" if version == 1 => at(&[(2, VALIDITY), (6, PAYEE)]),
            "HITAN" => Policy::From(4),
            "HKSPA" => at(&(1..=99).map(|d| (d, KTV)).collect::<Vec<_>>()),
            "HISPA" => at(&(1..=99).map(|d| (d, KTZ)).collect::<Vec<_>>()),
            c if is_parameter_segment(c) => Policy::Safe,
            _ => Policy::All,
        }
    }

    /// How data element `de`, group element `element` is hidden, if it is.
    fn masks(&self, de: usize, element: usize) -> Option<Hide> {
        match self {
            Policy::Safe | Policy::Feedback => None,
            Policy::At(list) => list
                .iter()
                .find(|(d, e, _)| *d == de && (e.is_empty() || e.contains(&element)))
                .map(|&(_, _, h)| h),
            Policy::From(n) => (de >= *n).then_some(Star),
            Policy::All => Some(Star),
        }
    }
}

/// The identity class of binary data element `de` of a segment that holds
/// no statement or pain message: the VOP-ID and polling ID of the payee
/// check (HIVPP 1 and 3, HKVPA 1, HKVPP 2). Other binary data (keys,
/// certificates, order hashes) is starred.
fn binary_id(code: &str, version: u16, de: usize) -> Option<IdClass> {
    match (code, version, de) {
        ("HIVPP", 1, 1) | ("HKVPA", 1, 1) => Some(IdClass::VopId),
        ("HIVPP", 1, 3) | ("HKVPP", 1, 2) => Some(IdClass::PollingId),
        _ => None,
    }
}

/// Bank parameter segments carry no personal data: the general ones
/// (Formals D.2 to D.5: HIBPA, HIKOM with the bank's addresses, HISHV,
/// HIKPV) and the job parameters (`HIPINS`, `HITANS`, `HIKAZS`, ...).
/// The code of a job parameter segment is the order's code with `I` for
/// `K` and an `S` at the end (Formals D.6), whoever defined the order:
/// `HI…S`, `DI…S` for the banks' common orders (comdirect: DIKKUS) and
/// any other first letter for a bank's own (Consorsbank: XIADAS).
fn is_parameter_segment(code: &str) -> bool {
    let b = code.as_bytes();
    matches!(code, "HIBPA" | "HIKOM" | "HISHV" | "HIKPV")
        || (code.starts_with("HI") && code.ends_with('S') && code.len() == 5)
        || (b.len() == 6 && b[1] == b'I' && b[5] == b'S' && b.iter().all(u8::is_ascii_uppercase))
}

#[cfg(test)]
mod tests;
