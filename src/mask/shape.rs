//! Shape masking of statements, amounts and free text.
//!
//! A shape-masked value loses its content and keeps what a parser needs
//! to see:
//!
//! - free text: eight `#` for each run of it, up to the next piece of
//!   markup or line break, whatever its length ([`shape_text`]): how long
//!   a purpose is links a booking to its counterpart in another account;
//! - a name: eight `#` whatever its length ([`name_shape`]);
//! - an amount: `#,##` whatever its size and its decimals
//!   ([`amount_shape`]): a round sum is recognisable by its decimals;
//! - a count: `#`.
//!
//! Three more things are changed among what stays: every date is moved by
//! the days of the trace ([`dates`]), the debit mark of a balance is
//! written as credit (an overdrawn account does not show; the mark of a
//! booking stays), and an identity value is written as its stand-in, which
//! for a counterparty's IBAN, a creditor ID and a reference has one fixed
//! form (`pseudonym`).
//!
//! The shaped data has its own length; the caller writes it into `@len@`.
//!
//! Statements are masked by field, not as a whole: markup and codes stay,
//! values go. The fields per format:
//!
//! - **MT940** (HIKAZ). Kept: the tags; `:28C:` (statement number);
//!   `:13D:`; in balances (`:60F:`, `:60M:`, `:62F:`, `:62M:`, `:64:`,
//!   `:65:`) the debit/credit mark, date and currency; in MT942 `:34F:`
//!   the currency and mark, in `:90D:`/`:90C:` the count and currency; in
//!   `:61:` the value
//!   date, entry date, debit/credit mark (`C`, `D`, `RC`, `RD`), funds code,
//!   transaction type (`NTRF`, …) and the customer reference `NONREF`; in a
//!   structured `:86:` the GV code, the `?nn` subfield codes, the
//!   Textschlüsselergänzung (`?34`) and the SEPA keywords (`EREF+`, `KREF+`,
//!   `MREF+`, `CRED+`, `DEBT+`, `COAM+`, `OAMT+`, `SVWZ+`, `ABWA+`, `ABWE+`,
//!   `IBAN+`, `BIC+`, `PURP+`) at the start of a subfield or after a space.
//!   The counterparty name (`?32`, `?33`) is a name. Amounts (balances,
//!   `:61:`, `:34F:`, `:90D:`/`:90C:`): shape, rebuilt as `1`; the `:90D:`/`:90C:` sums are then recomputed from the rebuilt
//!   `:61:` amounts, so a replayed MT942 adds up. Everything else is
//!   text: `:20:`, `:21:`, `:25:` (own account), references and
//!   supplementary details of `:61:`, every `:86:` subfield value (booking
//!   text, primanota, purpose, counterparty name, BLZ/BIC, account/IBAN),
//!   an unstructured `:86:`, unknown tags.
//! - **camt.052** (HICAZ). Kept: the markup (tags with element and
//!   attribute names and attribute values such as `Ccy="EUR"`, the XML
//!   declaration, comments, whitespace between tags) and the
//!   text of `CdtDbtInd`, `Cd`, `Sts`, `Dt`, `DtTm`, `CreDtTm`, `FrDtTm`,
//!   `ToDtTm`, `RvslInd`, `BtchBookg`, `LastPgInd` and `Ccy`. Amounts and
//!   counts (elements named `…Amt`, `Sum`, `…Nb`, `NbOfNtries`, `NbOfTxs`):
//!   shape, rebuilt as `1`. `Nm` and `AdrLine` are names. Every other text
//!   (IBANs, BICs, references, remittance information, booking text) is
//!   text, entity references (`&amp;`, `&#228;`) included.
//! - **MT535/MT536** (HIWPD, HIWDU). Kept: the tags, qualifiers (`:HOLD//`),
//!   the fields `:16R:`, `:16S:`, `:23G:`, `:22F:`, `:22H:`, `:25D:`,
//!   `:17B:`, `:28E:`, `:13A:`, `:98A:`, `:98B:`, `:98C:`, `:98E:`, `:69A:`,
//!   `:69B:`, `:94B:`, `:11A:`, `:12A:`, `:12B:`, `:12C:`; the `ISIN `
//!   keyword of `:35B:`. In the other fields each part between `/`, `+` and
//!   line breaks is read on its own: a four-letter code (`ACTU`, `UNIT`,
//!   `FAMT`) stays, a sign `N` and currency stay, a number with a decimal
//!   comma (prices, quantities, holding values, acquisition price) is an
//!   amount, a date (`YYYYMMDD`) in `:70E:` stays. In `:35B:` the ISIN and
//!   the WKN (`/DE/…`) are text and each further line is a name. Text: `:97A:`/`:97B:` (safekeeping account), `:95P:`/`:95Q:`/
//!   `:95R:`/`:95S:` (parties), `:20C:` (references), `:70C:`/`:70D:`/
//!   `:70G:` (narrative), and every other part.
//!
//! [`unshape`] reads the masked data with the same field rules (they look
//! only at markup, which the mask keeps) and turns each `#` back into a
//! valid character: `1` in amounts, `X` in text.
//!
//! Clean room: written from the SWIFT MT940/MT535/MT536 field layouts, the
//! DK "Datenformate" (structured `:86:`, SEPA keywords) and the ISO 20022
//! camt.052 message definition.

use std::collections::HashMap;

use super::dates::{self, Form};
use super::pseudonym::{self, IdClass, Identities};

/// The statement format of a segment's binary data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Format {
    Mt940,
    Camt,
    /// pain.001 (payment order) and pain.002 (payment status report):
    /// [`Format::pain`].
    Pain,
    /// MT535 and MT536 (same field syntax).
    Mt535,
}

impl Format {
    /// The format of the binary data of this answer segment, if it holds a
    /// statement.
    pub(crate) fn of(code: &str) -> Option<Format> {
        match code {
            "HIKAZ" => Some(Format::Mt940),
            "HICAZ" => Some(Format::Camt),
            "HIWPD" | "HIWDU" => Some(Format::Mt535),
            _ => None,
        }
    }
}

impl Format {
    /// The format of binary data element `de` (counted from the segment
    /// header, which is 0) of a payment segment, if it holds a pain message:
    /// the order of HKCCS and HKIPZ (3), the report of HKIPS's answer HIIPS
    /// (3) and of HIVPP (5). Other binary data of these segments (VOP-ID,
    /// polling ID) is not shaped.
    pub(crate) fn pain(code: &str, de: usize) -> Option<Format> {
        match (code, de) {
            ("HKCCS" | "HKIPZ" | "HIIPS", 3) | ("HIVPP", 5) => Some(Format::Pain),
            _ => None,
        }
    }
}

/// What a byte of a statement is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Markup or a code: kept.
    Keep,
    /// Free text or an identifier: each run of it (up to markup or a line
    /// break) is written as eight `#` whatever its length, rebuilt as `X`.
    /// A run of spaces only is one space.
    Text,
    /// A name (payee, payer, security) of at most this many characters by
    /// its format: written with a fixed length ([`name_shape`]), rebuilt as
    /// `X`.
    Name(usize),
    /// An amount: always `#,##` (`#.##` in XML), rebuilt as `1`
    /// ([`amount_shape`]).
    Amount,
    /// A count: `#`, rebuilt as `1`.
    Count,
}

/// The shape of a free text: each line of it is eight `#` whatever its
/// length (an empty line stays empty); the line breaks stay.
pub(crate) fn shape_text(text: &str) -> String {
    let mut out = String::new();
    let mut in_line = false;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            out.push(c);
            in_line = false;
        } else if !in_line {
            out.push_str(&"#".repeat(TEXT_LEN));
            in_line = true;
        }
    }
    out
}

/// Length of the shape of a name within its limit.
const NAME_LEN: usize = 8;

/// The shape of a name of `chars` characters whose format allows `limit`:
/// eight `#`, so the length says nothing about the name; `limit + 1` when
/// the bank sent more than the format allows, so that stays visible.
pub(crate) fn name_shape(chars: usize, limit: usize) -> String {
    let len = match chars {
        0 => 0,
        n if n > limit => limit + 1,
        _ => NAME_LEN,
    };
    "#".repeat(len)
}

/// The shape of an amount: `#,##` whatever its size and however many
/// decimals were sent (`#.##` when the separator is a point, as in XML), so
/// neither the magnitude nor a round sum shows. A number without a
/// separator is `#`. A sign stays.
pub(crate) fn amount_shape(text: &str) -> String {
    let sign: String = text
        .chars()
        .take_while(|c| matches!(c, '-' | '+'))
        .collect();
    match text.chars().find(|c| matches!(c, ',' | '.')) {
        Some(sep) => format!("{sign}#{sep}##"),
        None if text.len() == sign.len() => sign,
        None => format!("{sign}#"),
    }
}

/// A shaped text made valid again: `1` for each `#` of an amount shape
/// (`###,##`), `X` for each `#` of anything else.
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) fn unshape_text(text: &str) -> String {
    let amount = text.split_once(',').is_some_and(|(a, b)| {
        !a.is_empty() && a.chars().all(|c| c == '#') && b.chars().all(|c| c == '#')
    });
    let fill = if amount { '1' } else { 'X' };
    text.chars()
        .map(|c| if c == '#' { fill } else { c })
        .collect()
}

/// A FinTS amount (`Wrt`): digits, a comma, digits; or the shape of one
/// (`#,##`), so a message that is masked again is read the same way.
pub(crate) fn is_amount(s: &str) -> bool {
    let digit = |c: u8| c.is_ascii_digit() || c == b'#';
    s.split_once(',')
        .is_some_and(|(a, b)| !a.is_empty() && a.bytes().all(digit) && b.bytes().all(digit))
}

/// An identity value inside a statement or pain message: its class and
/// the positions of its bytes (in order; a value may span `:86:` subfields
/// and line breaks).
#[derive(Debug, PartialEq, Eq)]
struct Span {
    class: IdClass,
    at: Vec<usize>,
}

/// A date among the kept bytes of a statement.
#[derive(Debug, PartialEq, Eq)]
enum DateAt {
    /// A date of this notation at this position.
    Plain(usize, Form),
    /// An MT940 entry date `MMDD` at the first position, beside the value
    /// date `YYMMDD` at the second.
    Entry(usize, usize),
    /// A time of day (and a UTC offset) from the first position up to the
    /// second: written as noon.
    Time(usize, usize),
}

/// What [`classify`] found in a statement.
struct Plan {
    /// What each byte is.
    kinds: Vec<Kind>,
    /// The identity values (only ever over text: markup and codes stay).
    ids: Vec<Span>,
    /// The dates among the kept bytes.
    dates: Vec<DateAt>,
    /// Kept bytes that are written differently: the debit mark of a
    /// balance becomes the credit mark, so an overdrawn account does not
    /// show.
    marks: Vec<(usize, &'static [u8])>,
}

/// The statement, shape-masked by field. With `ids`, the identity values
/// ([`IdClass`]: account identifiers, booking references, end-to-end IDs,
/// pain message and payment information IDs, mandate references, creditor
/// IDs) get their alias or pseudonym instead of their shape, and every
/// date is moved by the days of `ids` ([`dates`]).
pub(crate) fn shape(format: Format, data: &[u8], ids: Option<&Identities>) -> Vec<u8> {
    let plan = classify(format, data);
    // The kept bytes, with dates moved and balance marks rewritten.
    let mut kept = data.to_vec();
    if let Some(days) = ids.map(Identities::days) {
        for date in &plan.dates {
            match *date {
                DateAt::Time(from, to) => {
                    if let Some(b) = kept.get_mut(from..to) {
                        dates::noon(b);
                    }
                }
                DateAt::Plain(at, form) => {
                    if let Some(b) = kept.get_mut(at..at + form.len()) {
                        dates::shift(b, form, days);
                    }
                }
                DateAt::Entry(at, value) => {
                    if let (Some(v), true) = (data.get(value..value + 6), at + 4 <= kept.len()) {
                        dates::shift_entry(&mut kept[at..at + 4], v, days);
                    }
                }
            }
        }
    }
    for &(at, mark) in &plan.marks {
        if let Some(b) = kept.get_mut(at..at + mark.len()) {
            b.copy_from_slice(mark);
        }
    }
    // The stand-in of each identity value, written at its first byte; its
    // other bytes are left out (a stand-in has its own length).
    let mut stand_ins: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut part_of_id = vec![false; data.len()];
    if let Some(ids) = ids {
        for span in &plan.ids {
            let value: Vec<u8> = span.at.iter().map(|&i| data[i]).collect();
            stand_ins.insert(span.at[0], ids.bytes(span.class, &value));
            for &i in &span.at {
                part_of_id[i] = true;
            }
        }
    }
    let decimal_point = matches!(format, Format::Camt | Format::Pain);
    let kinds = &plan.kinds;
    let line_break = |at: usize| matches!(data[at], b'\r' | b'\n');
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if part_of_id[i] {
            if let Some(stand_in) = stand_ins.get(&i) {
                out.extend_from_slice(stand_in);
            }
            i += 1;
            continue;
        }
        let kind = kinds[i];
        // The run of bytes of this kind, up to a line break or an identity
        // value.
        let run_end = |same: &dyn Fn(usize) -> bool| {
            (i..data.len())
                .find(|&at| !same(at) || part_of_id[at])
                .unwrap_or(data.len())
        };
        match kind {
            Kind::Keep => {
                out.push(kept[i]);
                i += 1;
            }
            Kind::Text | Kind::Name(_) if line_break(i) => {
                out.push(data[i]);
                i += 1;
            }
            Kind::Text => {
                let to = run_end(&|at| kinds[at] == Kind::Text && !line_break(at));
                if data[i..to].iter().all(|&b| b == b' ') {
                    out.push(b' ');
                } else {
                    out.extend(std::iter::repeat_n(b'#', TEXT_LEN));
                }
                i = to;
            }
            Kind::Name(limit) => {
                let to = run_end(&|at| kinds[at] == kind && !line_break(at));
                let chars = String::from_utf8_lossy(&data[i..to]).chars().count();
                out.extend(name_shape(chars, limit).bytes());
                i = to;
            }
            Kind::Amount => {
                let to = run_end(&|at| kinds[at] == Kind::Amount);
                let text: String = data[i..to].iter().map(|&b| char::from(b)).collect();
                let shape = amount_shape(&text);
                if decimal_point && !shape.contains(['.', ',']) && shape.ends_with('#') {
                    out.extend(format!("{shape}.##").bytes());
                } else {
                    out.extend(shape.bytes());
                }
                i = to;
            }
            Kind::Count => {
                let to = run_end(&|at| kinds[at] == Kind::Count);
                out.push(b'#');
                i = to;
            }
        }
    }
    out
}

/// Length of the shape of a run of free text.
const TEXT_LEN: usize = 8;

/// A shape-masked statement made valid again: each `#` becomes `1` in an
/// amount and `X` anywhere else.
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) fn unshape(format: Format, data: &[u8]) -> Vec<u8> {
    let kinds = classify(format, data).kinds;
    let out: Vec<u8> = data
        .iter()
        .zip(kinds)
        .map(|(&b, kind)| match (b, kind) {
            (b'#', Kind::Amount | Kind::Count) => b'1',
            (b'#', Kind::Text | Kind::Name(_)) => b'X',
            _ => b,
        })
        .collect();
    match format {
        // The rebuilt `:61:` amounts no longer add up to the recorded
        // MT942 sums.
        Format::Mt940 if data.contains(&b'#') => crate::formats::mt940::recompute_totals(&out),
        _ => out,
    }
}

/// What each byte is, the identity values, the dates and the balance marks.
fn classify(format: Format, data: &[u8]) -> Plan {
    let mut plan = Plan {
        kinds: vec![Kind::Text; data.len()],
        ids: Vec::new(),
        dates: Vec::new(),
        marks: Vec::new(),
    };
    match format {
        Format::Mt940 => mt940(data, &mut plan),
        Format::Camt => xml(data, &mut plan, camt_kind),
        Format::Pain => xml(data, &mut plan, pain_kind),
        Format::Mt535 => mt535(data, &mut plan),
    }
    let Plan { kinds, ids, .. } = &mut plan;
    if matches!(format, Format::Mt940 | Format::Mt535) {
        // A line of one `-` ends a SWIFT message: layout, not text.
        let mut start = 0;
        for at in 0..=data.len() {
            if at == data.len() || matches!(data[at], b'\r' | b'\n') {
                if &data[start..at] == b"-" {
                    kinds[start] = Kind::Keep;
                }
                start = at + 1;
            }
        }
    }
    for span in ids.iter_mut() {
        span.at.retain(|&i| matches!(kinds[i], Kind::Text));
    }
    ids.retain(|s| !s.at.is_empty());
    plan
}

/// A span of identity class `class` over `at`, without trailing spaces.
fn span(data: &[u8], class: IdClass, mut at: Vec<usize>) -> Option<Span> {
    while at.last().is_some_and(|&i| data[i].is_ascii_whitespace()) {
        at.pop();
    }
    (!at.is_empty()).then_some(Span { class, at })
}

/// An own or counterparty account written as text: an IBAN (its letters
/// and digits; a German one is 22 long, so a currency after it is not
/// part of it), else a national account number (its digits).
fn account_span(data: &[u8], r: std::ops::Range<usize>) -> Option<Span> {
    let b = &data[r.clone()];
    let start = b.iter().position(|&c| c != b' ')?;
    let run = b[start..]
        .iter()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .count();
    let iban_len = if b[start..].starts_with(b"DE") {
        run.min(22)
    } else {
        run
    };
    if pseudonym::is_iban(&b[start..start + iban_len]) {
        let at = (r.start + start..r.start + start + iban_len).collect();
        return span(data, IdClass::Iban, at);
    }
    let digits = b[start..].iter().take_while(|c| c.is_ascii_digit()).count();
    let at = (r.start + start..r.start + start + digits).collect();
    span(data, IdClass::Account, at)
}

/// A counterparty's account in an MT940 `?31`, at the positions `at` (the
/// value without its line breaks): an IBAN, else a national account number.
fn other_account(data: &[u8], at: &[usize]) -> Option<Span> {
    let value: Vec<u8> = at.iter().map(|&i| data[i]).collect();
    let start = value.iter().position(|&c| c != b' ')?;
    let run = value[start..]
        .iter()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .count();
    let iban_len = if value[start..].starts_with(b"DE") {
        run.min(22)
    } else {
        run
    };
    if pseudonym::is_iban(&value[start..start + iban_len]) {
        return span(
            data,
            IdClass::OtherIban,
            at[start..start + iban_len].to_vec(),
        );
    }
    let digits = value[start..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    span(data, IdClass::Account, at[start..start + digits].to_vec())
}

fn set(kinds: &mut [Kind], range: std::ops::Range<usize>, kind: Kind) {
    let end = range.end.min(kinds.len());
    let start = range.start.min(end);
    kinds[start..end].fill(kind);
}

// ── SWIFT fields ────────────────────────────────────────────────────────

/// One `:tag:` field: the tag (without colons), where the `:tag:` markup
/// is, and where its value is (up to the next field, line breaks and
/// continuation lines included).
struct Field<'a> {
    tag: &'a str,
    markup: std::ops::Range<usize>,
    value: std::ops::Range<usize>,
}

/// The fields of a SWIFT message: a field starts at the beginning of a line
/// with `:` two digits, an optional letter and `:`.
fn fields(data: &[u8]) -> Vec<Field<'_>> {
    let mut starts = Vec::new();
    for at in 0..data.len() {
        if (at == 0 || data[at - 1] == b'\n') && data[at] == b':' {
            let rest = &data[at + 1..];
            let len = match rest {
                [a, b, b':', ..] if a.is_ascii_digit() && b.is_ascii_digit() => 2,
                [a, b, c, b':', ..]
                    if a.is_ascii_digit() && b.is_ascii_digit() && c.is_ascii_uppercase() =>
                {
                    3
                }
                _ => continue,
            };
            starts.push((at, len));
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (n, &(at, len)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map_or(data.len(), |&(next, _)| next);
        let tag = std::str::from_utf8(&data[at + 1..at + 1 + len]).unwrap_or("");
        out.push(Field {
            tag,
            markup: at..at + len + 2,
            value: at + len + 2..end,
        });
    }
    out
}

/// The end of the line that `from` is on (the position of its `\r` or
/// `\n`), at most `end`.
fn line_end(data: &[u8], from: usize, end: usize) -> usize {
    data[from..end]
        .iter()
        .position(|&b| b == b'\r' || b == b'\n')
        .map_or(end, |p| from + p)
}

fn is_number(b: u8) -> bool {
    b.is_ascii_digit() || b == b',' || b == b'#'
}

// ── MT940 ───────────────────────────────────────────────────────────────

/// SEPA keywords of the DK structured purpose (without the `+`).
const SEPA_KEYWORDS: [&[u8]; 13] = [
    b"EREF", b"KREF", b"MREF", b"CRED", b"DEBT", b"COAM", b"OAMT", b"SVWZ", b"ABWA", b"ABWE",
    b"IBAN", b"BIC", b"PURP",
];

fn mt940(data: &[u8], plan: &mut Plan) {
    let Plan {
        kinds,
        ids,
        dates,
        marks,
    } = plan;
    let kinds = kinds.as_mut_slice();
    for f in fields(data) {
        set(kinds, f.markup.clone(), Kind::Keep);
        let v = f.value.clone();
        match f.tag {
            "28C" | "28" => set(kinds, v, Kind::Keep),
            // `YYMMDDhhmm+zzzz`: when the bank made the list.
            "13D" | "13" => {
                dates.push(DateAt::Plain(v.start, Form::Ymd6));
                let end = line_end(data, v.start, v.end);
                dates.push(DateAt::Time((v.start + 6).min(end), end));
                set(kinds, v, Kind::Keep);
            }
            "60F" | "60M" | "62F" | "62M" | "64" | "65" => {
                balance(data, v, kinds, dates, marks);
            }
            "34F" => floor(data, v, kinds),
            "90D" | "90C" => total(data, v, kinds),
            "61" => statement_line(data, v, kinds, ids, dates),
            "86" => details(data, v, kinds, ids),
            "25" => {
                // `BLZ/number`: the slash is layout.
                let end = line_end(data, v.start, v.end);
                if let Some(slash) = data[v.start..end].iter().position(|&b| b == b'/') {
                    kinds[v.start + slash] = Kind::Keep;
                }
                ids.extend(own_account(data, v));
            }
            _ => {}
        }
    }
}

/// `:25:` `BLZ/account number` or an IBAN, either with an optional
/// currency: the account number or the IBAN.
fn own_account(data: &[u8], v: std::ops::Range<usize>) -> Option<Span> {
    let end = line_end(data, v.start, v.end);
    let line = &data[v.start..end];
    match line.iter().position(|&b| b == b'/') {
        Some(slash) if line[..slash].iter().all(u8::is_ascii_digit) => {
            account_span(data, v.start + slash + 1..end)
        }
        _ => account_span(data, v.start..end),
    }
}

/// `C260101EUR100,00`: date and currency kept, amount. The mark is
/// written as `C`: whether an account is overdrawn does not show.
fn balance(
    data: &[u8],
    v: std::ops::Range<usize>,
    kinds: &mut [Kind],
    dates: &mut Vec<DateAt>,
    marks: &mut Vec<(usize, &'static [u8])>,
) {
    let end = line_end(data, v.start, v.end);
    if end < v.start + 10 {
        return;
    }
    set(kinds, v.start..v.start + 10, Kind::Keep);
    if data[v.start] == b'D' {
        marks.push((v.start, b"C"));
    }
    dates.push(DateAt::Plain(v.start + 1, Form::Ymd6));
    let mut p = v.start + 10;
    while p < end && is_number(data[p]) {
        kinds[p] = Kind::Amount;
        p += 1;
    }
}

/// MT942 `:34F:` `EURD0,`: currency and optional mark kept, amount.
fn floor(data: &[u8], v: std::ops::Range<usize>, kinds: &mut [Kind]) {
    let end = line_end(data, v.start, v.end);
    let line = &data[v.start..end];
    if line.len() < 3 || !line[..3].iter().all(u8::is_ascii_alphabetic) {
        return;
    }
    let p = if matches!(line.get(3), Some(b'D' | b'C')) {
        4
    } else {
        3
    };
    set(kinds, v.start..v.start + p, Kind::Keep);
    numbers(data, v.start + p, end, kinds);
}

/// MT942 `:90D:`/`:90C:` `3EUR8,50`: the count of bookings and the
/// currency kept (the count is visible from the `:61:` lines anyway), the
/// sum an amount. [`unshape`] recomputes the sum from the rebuilt `:61:`
/// amounts, so the replayed MT942 adds up.
fn total(data: &[u8], v: std::ops::Range<usize>, kinds: &mut [Kind]) {
    let end = line_end(data, v.start, v.end);
    let line = &data[v.start..end];
    let count = line.iter().take_while(|b| b.is_ascii_digit()).count();
    if count == 0
        || line.len() < count + 3
        || !line[count..count + 3].iter().all(u8::is_ascii_alphabetic)
    {
        return;
    }
    set(kinds, v.start..v.start + count + 3, Kind::Keep);
    numbers(data, v.start + count + 3, end, kinds);
}

fn numbers(data: &[u8], from: usize, end: usize, kinds: &mut [Kind]) {
    let mut p = from;
    while p < end && is_number(data[p]) {
        kinds[p] = Kind::Amount;
        p += 1;
    }
}

/// `:61:` value date, [entry date], mark, [funds code], amount, type,
/// customer reference [`//` bank reference] [line break, supplementary
/// details]. Identity values: the customer reference (an end-to-end ID
/// when the payer gave one) and the bank reference (the booking
/// reference).
fn statement_line(
    data: &[u8],
    v: std::ops::Range<usize>,
    kinds: &mut [Kind],
    ids: &mut Vec<Span>,
    dates: &mut Vec<DateAt>,
) {
    let end = line_end(data, v.start, v.end);
    let line = &data[v.start..end];
    let mut p = 0;
    let digits = |from: usize, n: usize| {
        line.get(from..from + n)
            .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
    };
    if !digits(0, 6) {
        return;
    }
    dates.push(DateAt::Plain(v.start, Form::Ymd6));
    p += 6;
    if digits(p, 4) {
        dates.push(DateAt::Entry(v.start + p, v.start));
        p += 4;
    }
    if line[p..].starts_with(b"RC") || line[p..].starts_with(b"RD") {
        p += 2;
    } else if line[p..].starts_with(b"C") || line[p..].starts_with(b"D") {
        p += 1;
    } else {
        return;
    }
    if line.get(p).is_some_and(u8::is_ascii_alphabetic) {
        p += 1;
    }
    set(kinds, v.start..v.start + p, Kind::Keep);
    while p < line.len() && is_number(line[p]) {
        kinds[v.start + p] = Kind::Amount;
        p += 1;
    }
    let kind_end = (p + 4).min(line.len());
    set(kinds, v.start + p..v.start + kind_end, Kind::Keep);
    p = kind_end;
    let reference_end = line[p..]
        .windows(2)
        .position(|w| w == b"//")
        .map_or(line.len(), |at| p + at);
    if &line[p..reference_end] == b"NONREF" {
        set(kinds, v.start + p..v.start + reference_end, Kind::Keep);
    } else {
        let at = (v.start + p..v.start + reference_end).collect();
        ids.extend(span(data, IdClass::EndToEnd, at));
    }
    if reference_end < line.len() {
        set(
            kinds,
            v.start + reference_end..v.start + reference_end + 2,
            Kind::Keep,
        );
        let at = (v.start + reference_end + 2..end).collect();
        ids.extend(span(data, IdClass::BookingRef, at));
    }
}

/// `:86:` structured (`GVC` + separator + `nn` subfields) or plain text.
/// Line breaks inside the field are skipped, so a subfield code split by a
/// line break is still found.
///
/// Identity values: the counterparty account (`?31`, IBAN or account
/// number) and, in the purpose (`?20` to `?29`, `?60` to `?63`, read as one
/// text), the values of `EREF+` (end-to-end ID), `MREF+` (mandate
/// reference), `CRED+` (creditor ID) and `IBAN+`, each up to the next
/// keyword, across subfields.
fn details(data: &[u8], v: std::ops::Range<usize>, kinds: &mut [Kind], ids: &mut Vec<Span>) {
    let at: Vec<usize> = v
        .filter(|&i| data[i] != b'\r' && data[i] != b'\n')
        .collect();
    let b = |n: usize| data[at[n]];
    let structured = at.len() >= 4
        && (0..3).all(|n| b(n).is_ascii_digit())
        && !b(3).is_ascii_alphanumeric()
        && b(3) != b' ';
    if !structured {
        return;
    }
    let keep = |kinds: &mut [Kind], from: usize, to: usize| {
        for &i in &at[from..to] {
            kinds[i] = Kind::Keep;
        }
    };
    keep(kinds, 0, 3);
    let sep = b(3);
    let is_code = |n: usize| {
        n + 2 < at.len() && b(n) == sep && b(n + 1).is_ascii_digit() && b(n + 2).is_ascii_digit()
    };
    // (code, value start, value end), positions in `at`.
    let mut subfields = Vec::new();
    let mut n = 3;
    while n < at.len() {
        if !is_code(n) {
            n += 1;
            continue;
        }
        keep(kinds, n, n + 3);
        let code = [b(n + 1), b(n + 2)];
        n += 3;
        let start = n;
        while n < at.len() && b(n) != sep {
            n += 1;
        }
        subfields.push((code, start, n));
    }
    for &(code, start, end) in &subfields {
        match &code {
            b"34" => keep(kinds, start, end),
            // The counterparty account. `at` skips line breaks, so an
            // account the bank wrapped is still one value.
            b"31" if end > start => ids.extend(other_account(data, &at[start..end])),
            // The counterparty name, 27 characters per subfield.
            b"32" | b"33" => {
                for &i in &at[start..end] {
                    kinds[i] = Kind::Name(27);
                }
            }
            _ => {}
        }
    }
    // SEPA keywords at the start of a subfield or after a space.
    let keywords = |from: usize, to: usize| {
        let mut hits = Vec::new();
        for q in from..to {
            if q != from && !matches!(b(q - 1), b' ' | b'#') {
                continue;
            }
            for kw in SEPA_KEYWORDS {
                let len = kw.len();
                if q + len < to && (0..len).all(|i| b(q + i) == kw[i]) && b(q + len) == b'+' {
                    hits.push((q, kw));
                }
            }
        }
        hits
    };
    for &(code, start, end) in &subfields {
        if &code != b"34" {
            for (q, kw) in keywords(start, end) {
                keep(kinds, q, q + kw.len() + 1);
            }
        }
    }
    // The purpose as one text: positions in `at`, subfield codes left out.
    let is_purpose = |c: &[u8; 2]| matches!(c, [b'2', _] | [b'6', b'0'..=b'3']);
    let purpose: Vec<usize> = subfields
        .iter()
        .filter(|(c, ..)| is_purpose(c))
        .flat_map(|&(_, s, e)| s..e)
        .collect();
    let starts: Vec<usize> = subfields
        .iter()
        .filter(|(c, ..)| is_purpose(c))
        .scan(0, |len, &(_, s, e)| {
            let start = *len;
            *len += e - s;
            Some(start)
        })
        .collect();
    let pb = |k: usize| data[at[purpose[k]]];
    let mut hits = Vec::new();
    for q in 0..purpose.len() {
        if !starts.contains(&q) && !matches!(pb(q - 1), b' ' | b'#') {
            continue;
        }
        for kw in SEPA_KEYWORDS {
            let len = kw.len();
            if q + len < purpose.len()
                && (0..len).all(|i| pb(q + i) == kw[i])
                && pb(q + len) == b'+'
            {
                hits.push((q, kw));
            }
        }
    }
    for (h, &(q, kw)) in hits.iter().enumerate() {
        let class = match kw {
            b"EREF" => IdClass::EndToEnd,
            b"MREF" => IdClass::Mandate,
            b"CRED" => IdClass::CreditorId,
            b"IBAN" => IdClass::OtherIban,
            _ => continue,
        };
        let to = hits.get(h + 1).map_or(purpose.len(), |&(next, _)| next);
        let from = (q + kw.len() + 1).min(to);
        let positions = purpose[from..to].iter().map(|&k| at[k]).collect();
        ids.extend(span(data, class, positions));
    }
}

// ── camt ────────────────────────────────────────────────────────────────

/// Elements whose text is a code, a date or a flag.
const CAMT_KEPT: [&str; 12] = [
    "CdtDbtInd",
    "Cd",
    "Sts",
    "Dt",
    "DtTm",
    "CreDtTm",
    "FrDtTm",
    "ToDtTm",
    "RvslInd",
    "BtchBookg",
    "LastPgInd",
    "Ccy",
];

fn camt_kind(element: Option<&str>) -> Kind {
    match element {
        Some(e) if CAMT_KEPT.contains(&e) => Kind::Keep,
        // Party names (`Max140Text`) and address lines (`Max70Text`).
        Some("Nm") => Kind::Name(140),
        Some("AdrLine") => Kind::Name(70),
        Some(e) if e.ends_with("Amt") || e == "Sum" => Kind::Amount,
        Some(e) if e.ends_with("Nb") || matches!(e, "NbOfNtries" | "NbOfTxs") => Kind::Count,
        _ => Kind::Text,
    }
}

/// Elements of a pain.001 or pain.002 whose text is a code, not a value:
/// the status codes (`DtldSts` is the status a count in `NbOfTxsPerSts`
/// stands for), `Cd` (service level, local instrument, reason code) and
/// `Ccy`. Names, IBANs, BICs, references, dates, remittance information and
/// every other text are values; amounts (`InstdAmt`, `CtrlSum`) and counts
/// are amounts.
const PAIN_KEPT: [&str; 7] = [
    "Cd",
    "Sts",
    "TxSts",
    "GrpSts",
    "PmtInfSts",
    "DtldSts",
    "Ccy",
];

fn pain_kind(element: Option<&str>) -> Kind {
    match element {
        Some(e) if PAIN_KEPT.contains(&e) => Kind::Keep,
        Some("CtrlSum") => Kind::Amount,
        other => camt_kind(other.filter(|e| !CAMT_KEPT.contains(e))),
    }
}

fn find(data: &[u8], from: usize, what: &[u8]) -> Option<usize> {
    data.get(from..)?
        .windows(what.len())
        .position(|w| w == what)
        .map(|p| from + p)
}

/// The identity class of the text of the innermost element of `path` in a
/// camt or pain message.
fn xml_id(path: &[String]) -> Option<IdClass> {
    let name = |back: usize| path.len().checked_sub(back + 1).map(|i| path[i].as_str());
    match name(0)? {
        // The statement's own account (`Stmt/Acct`, `Rpt/Acct`) and the
        // debtor account of a pain.001 are the user's. Every other IBAN is
        // a counterparty's.
        "IBAN" if path.iter().any(|e| e == "Acct") => Some(IdClass::Iban),
        "IBAN" if path.iter().any(|e| e == "DbtrAcct") && !path.iter().any(|e| e == "Ntry") => {
            Some(IdClass::Iban)
        }
        "IBAN" => Some(IdClass::OtherIban),
        "AcctSvcrRef" => Some(IdClass::BookingRef),
        "EndToEndId" | "OrgnlEndToEndId" => Some(IdClass::EndToEnd),
        "MsgId" | "OrgnlMsgId" => Some(IdClass::MessageId),
        "PmtInfId" | "OrgnlPmtInfId" => Some(IdClass::PaymentInfoId),
        "MndtId" => Some(IdClass::Mandate),
        // CdtrSchmeId/Id/PrvtId/Othr/Id.
        "Id" if path.iter().any(|e| e == "CdtrSchmeId") => Some(IdClass::CreditorId),
        // Acct/Id/Othr/Id, DbtrAcct/…, CdtrAcct/…: an account without IBAN.
        "Id" if name(1) == Some("Othr")
            && name(2) == Some("Id")
            && name(3).is_some_and(|e| e.ends_with("Acct")) =>
        {
            Some(IdClass::Account)
        }
        _ => None,
    }
}

/// Elements of a camt whose text starts with a date (`YYYY-MM-DD`).
const CAMT_DATES: [&str; 5] = ["Dt", "DtTm", "CreDtTm", "FrDtTm", "ToDtTm"];

fn xml(data: &[u8], plan: &mut Plan, kind_of: fn(Option<&str>) -> Kind) {
    let Plan {
        kinds,
        ids,
        dates,
        marks,
    } = plan;
    let kinds = kinds.as_mut_slice();
    let mut open: Vec<String> = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let here = &data[i..];
        if here.starts_with(b"<![CDATA[") {
            let end = find(data, i + 9, b"]]>").unwrap_or(data.len());
            set(kinds, i..i + 9, Kind::Keep);
            set(kinds, i + 9..end, kind_of(open.last().map(String::as_str)));
            set(kinds, end..end + 3, Kind::Keep);
            i = end + 3;
        } else if here.starts_with(b"<!--") {
            let end = find(data, i + 4, b"-->").map_or(data.len(), |e| e + 3);
            set(kinds, i..end, Kind::Keep);
            i = end;
        } else if data[i] == b'<' {
            // A tag ends at the first `>` outside a quoted attribute value.
            let mut quote = None;
            let mut j = i + 1;
            while j < data.len() {
                match (quote, data[j]) {
                    (None, b'"' | b'\'') => quote = Some(data[j]),
                    (Some(q), c) if c == q => quote = None,
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            let end = (j + 1).min(data.len());
            set(kinds, i..end, Kind::Keep);
            let tag = &data[i + 1..j.min(data.len())];
            let name_of = |t: &[u8]| {
                let name = t
                    .split(|&b| b.is_ascii_whitespace() || b == b'/' || b == b'>')
                    .next()
                    .unwrap_or_default();
                let local = name.rsplit(|&b| b == b':').next().unwrap_or_default();
                String::from_utf8_lossy(local).into_owned()
            };
            match tag.first() {
                Some(b'/') => {
                    open.pop();
                }
                Some(b'?' | b'!') | None => {}
                _ if tag.last() == Some(&b'/') => {}
                _ => open.push(name_of(tag)),
            }
            i = end;
        } else {
            // Text up to the next tag.
            let end = data[i..]
                .iter()
                .position(|&b| b == b'<')
                .map_or(data.len(), |p| i + p);
            let kind = if data[i..end].iter().all(u8::is_ascii_whitespace) {
                Kind::Keep
            } else {
                kind_of(open.last().map(String::as_str))
            };
            set(kinds, i..end, kind);
            let element = open.last().map(String::as_str);
            let start = data[i..end]
                .iter()
                .position(|b| !b.is_ascii_whitespace())
                .map_or(end, |p| i + p);
            if kind == Kind::Keep && element.is_some_and(|e| CAMT_DATES.contains(&e)) {
                dates.push(DateAt::Plain(start, Form::Iso));
                dates.push(DateAt::Time((start + 10).min(end), end));
            }
            // The sign of a balance (`Bal/CdtDbtInd`), not of an entry.
            if kind == Kind::Keep
                && element == Some("CdtDbtInd")
                && open.len() >= 2
                && open[open.len() - 2] == "Bal"
                && data[start..end].starts_with(b"DBIT")
            {
                marks.push((start, b"CRDT"));
            }
            // Entity references are part of the text: a kept `&amp;` would
            // show where a name has an ampersand.
            if matches!(kind, Kind::Text)
                && let Some(class) = xml_id(&open)
            {
                ids.extend(span(data, class, (start..end).collect()));
            }
            i = end;
        }
    }
}

// ── MT535/MT536 ─────────────────────────────────────────────────────────

/// Fields that hold only codes, dates, numbering and flags.
const MT535_KEPT: [&str; 20] = [
    "16R", "16S", "23G", "22F", "22H", "25D", "17B", "28E", "13A", "98A", "98B", "98C", "98E",
    "69A", "69B", "94B", "11A", "12A", "12B", "12C",
];

/// Fields whose value (after the qualifier) is text throughout.
const MT535_TEXT: [&str; 11] = [
    "35B", "97A", "97B", "95P", "95Q", "95R", "95S", "20C", "70C", "70D", "70G",
];

fn mt535(data: &[u8], plan: &mut Plan) {
    let Plan {
        kinds, ids, dates, ..
    } = plan;
    let kinds = kinds.as_mut_slice();
    for f in fields(data) {
        set(kinds, f.markup.clone(), Kind::Keep);
        let v = f.value.clone();
        if MT535_KEPT.contains(&f.tag) {
            // Dates and periods (`:98A:`, `:98C:`, `:69A:`): each run of
            // digits that starts with `YYYYMMDD`.
            if f.tag.starts_with("98") || f.tag.starts_with("69") {
                let mut at = v.start;
                while at < v.end {
                    let run = data[at..v.end]
                        .iter()
                        .take_while(|b| b.is_ascii_digit())
                        .count();
                    if run >= 8 && dates::is_date(&data[at..at + 8], Form::Ymd8) {
                        dates.push(DateAt::Plain(at, Form::Ymd8));
                        dates.push(DateAt::Time(at + 8, at + run));
                    }
                    at += run.max(1);
                }
            }
            set(kinds, v, Kind::Keep);
            continue;
        }
        let mut p = v.start;
        // `:QUAL//` or `:QUAL/ISSUER/`: the qualifier stays.
        if data.get(p) == Some(&b':') {
            let end = line_end(data, p, v.end);
            if let Some(slash) = data[p..end].iter().position(|&b| b == b'/')
                && let Some(second) = data[p + slash + 1..end].iter().position(|&b| b == b'/')
            {
                let after = p + slash + 1 + second + 1;
                set(kinds, p..after, Kind::Keep);
                p = after;
            }
        }
        if f.tag == "97A" {
            // Safekeeping account: `BLZ/depot number` or the number alone.
            let end = line_end(data, p, v.end);
            let from = match data[p..end].iter().rposition(|&b| b == b'/') {
                Some(slash) => {
                    kinds[p + slash] = Kind::Keep;
                    p + slash + 1
                }
                None => p,
            };
            ids.extend(account_span(data, from..end));
        }
        if f.tag == "35B" {
            // `ISIN <code>` on the first line, then the WKN and the
            // security's name, 35 characters per line.
            let mut names = p;
            if data[p..v.end].starts_with(b"ISIN ") {
                set(kinds, p..p + 5, Kind::Keep);
                names = line_end(data, p, v.end);
            }
            set(kinds, names..v.end, Kind::Name(35));
            // A line `/DE/123456` is the national number (WKN): the scheme
            // stays, the number is text of a fixed length.
            let mut line = names;
            while line < v.end {
                let end = line_end(data, line, v.end);
                if let [b'/', a, b, b'/', ..] = &data[line..end]
                    && a.is_ascii_uppercase()
                    && b.is_ascii_uppercase()
                {
                    set(kinds, line..line + 4, Kind::Keep);
                    set(kinds, line + 4..end, Kind::Text);
                }
                line = end + 1;
            }
            continue;
        }
        if MT535_TEXT.contains(&f.tag) {
            continue;
        }
        let in_70e = f.tag == "70E";
        let mut start = p;
        for at in p..=v.end {
            if at == v.end || matches!(data[at], b'/' | b'+' | b'\r' | b'\n') {
                component(data, start..at, in_70e, kinds, dates);
                if at < v.end {
                    kinds[at] = Kind::Keep;
                }
                start = at + 1;
            }
        }
    }
}

/// One part of an MT535 value: a code, `[N]CCY[amount]`, `[N]amount`, a
/// date (when `dates`), or text.
fn component(
    data: &[u8],
    r: std::ops::Range<usize>,
    with_dates: bool,
    kinds: &mut [Kind],
    dates: &mut Vec<DateAt>,
) {
    let b = &data[r.clone()];
    if b.is_empty() {
        return;
    }
    if b.len() == 4 && b.iter().all(u8::is_ascii_uppercase) {
        set(kinds, r, Kind::Keep);
        return;
    }
    let sign = usize::from(b.len() > 1 && b[0] == b'N');
    let rest = &b[sign..];
    let number = |s: &[u8]| !s.is_empty() && s.iter().all(|&c| is_number(c));
    if rest.len() >= 3
        && rest[..3].iter().all(u8::is_ascii_uppercase)
        && (rest.len() == 3 || number(&rest[3..]))
    {
        set(kinds, r.start..r.start + sign + 3, Kind::Keep);
        set(kinds, r.start + sign + 3..r.end, Kind::Amount);
    } else if number(rest) && rest.contains(&b',') {
        set(kinds, r.start..r.start + sign, Kind::Keep);
        set(kinds, r.start + sign..r.end, Kind::Amount);
    } else if with_dates && dates::is_date(b, Form::Ymd8) {
        dates.push(DateAt::Plain(r.start, Form::Ymd8));
        set(kinds, r, Kind::Keep);
    }
}

#[cfg(test)]
mod tests;
