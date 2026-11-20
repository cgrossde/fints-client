//! Identity questions over recorded traces (`fints_client::trace`): equal
//! values have equal stand-ins within a trace, so a trace can answer
//! whether two values are the same without holding them.
//!
//! Account identifiers are aliases of one trace: an account is named by
//! its alias in that trace, and accounts are never matched across traces.
//! References are keyed pseudonyms: they match across traces only when the
//! traces were recorded with the same salt (`Recorder::with_salt`); traces
//! of `Recorder::new` each have their own salt and never match.
//!
//! [`analyse`] reads the statements and payment messages of one or more
//! traces and reports:
//!
//! - booking references (camt `Ntry/AcctSvcrRef`, MT940 `:61:` bank
//!   reference) that repeat within one statement of an account (one
//!   trace);
//! - references of pending entries in one trace that are on a booked entry
//!   in another trace, matched by the reference alone;
//! - end-to-end IDs sent in a pain.001 (HKCCS, HKIPZ) and found again in a
//!   pain.002 (`OrgnlEndToEndId` in HIVPP, HIIPS) or on an entry (camt
//!   `EndToEndId`, MT940 `EREF+` or `:61:` customer reference).
//!
//! The [`Report`] holds counts, aliases and pseudonyms only. A value that
//! is still a shape (`#`) or stars (traces recorded before stand-ins) is
//! counted as such and never compared.
//!
//! Clean room: written from the camt.052, pain.001 and pain.002 message
//! definitions (ISO 20022), the SWIFT MT940 field layout and the DK
//! structured `:86:` (SEPA keywords).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use fints_client::codec::{DataElement, RawSegment, wire};
use fints_client::trace::Trace;

/// One entry of a statement, as far as identity goes.
#[derive(Debug, Clone)]
struct Entry {
    trace: usize,
    account: String,
    booked: bool,
    /// The booking reference, if it is a pseudonym.
    reference: Option<String>,
    /// End-to-end IDs (and MT940 customer references), pseudonyms only.
    end_to_end: Vec<String>,
}

/// A booking reference seen more than once in one statement of an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeat {
    /// Index of the trace (from 0, in the order given).
    pub trace: usize,
    /// The account, by its alias in that trace: the IBAN or number alias
    /// the request names, else the recorder's `#n`.
    pub account: String,
    /// The reference's pseudonym.
    pub reference: String,
    /// How often it occurs.
    pub count: usize,
}

/// A reference of a pending entry that is on a booked entry in another
/// trace (same salt), matched by the reference alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingBooked {
    /// The account of the pending entry, by its alias in that trace.
    pub account: String,
    /// The reference's pseudonym.
    pub reference: String,
    /// The trace with the pending entry.
    pub pending_in: usize,
    /// The traces with the booked entry.
    pub booked_in: Vec<usize>,
}

/// An end-to-end ID sent in a pain.001.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentEndToEnd {
    /// The ID's pseudonym.
    pub id: String,
    /// Found in a pain.002 (`OrgnlEndToEndId`).
    pub in_status_report: bool,
    /// Found on a statement entry (booked or pending).
    pub on_entry: bool,
}

/// What [`analyse`] found. Counts, aliases and pseudonyms only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Traces read.
    pub traces: usize,
    /// Booked entries.
    pub booked: usize,
    /// Pending entries.
    pub pending: usize,
    /// Entries whose booking reference is a pseudonym.
    pub with_reference: usize,
    /// Entries with a reference that is a shape or stars, not a pseudonym.
    pub masked_references: usize,
    /// Booking references that repeat within one statement of an account.
    pub repeated: Vec<Repeat>,
    /// Pending entries with a pseudonymised reference.
    pub pending_with_reference: usize,
    /// Pending references found on a booked entry of another trace.
    pub pending_booked: Vec<PendingBooked>,
    /// End-to-end IDs sent in a pain.001, in order of first appearance.
    pub sent: Vec<SentEndToEnd>,
}

/// A value that can be compared (an alias or a pseudonym): not empty, no
/// shape `#`, no stars.
fn comparable(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty() && !v.contains('#') && !v.contains('*')).then(|| v.to_string())
}

fn latin1(b: &[u8]) -> String {
    b.iter().copied().map(char::from).collect()
}

fn code(seg: &RawSegment) -> &str {
    seg.segment_type()
}

fn element(seg: &RawSegment, de: usize, ge: usize) -> Option<&DataElement> {
    seg.degs.get(de)?.0.get(ge)
}

fn text(seg: &RawSegment, de: usize, ge: usize) -> Option<&str> {
    match element(seg, de, ge)? {
        DataElement::Text(s) => Some(s),
        _ => None,
    }
}

fn binaries(seg: &RawSegment, de: usize) -> Vec<&[u8]> {
    seg.degs
        .get(de)
        .map(|d| {
            d.0.iter()
                .filter_map(|e| match e {
                    DataElement::Binary(b) => Some(b.as_slice()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every segment of a message, the ones inside HNVSD included.
fn segments(msg: &[u8]) -> Vec<RawSegment> {
    let Ok(outer) = wire::tokenize(msg) else {
        return Vec::new();
    };
    let mut all = Vec::new();
    for seg in outer {
        if code(&seg) == "HNVSD"
            && let Some(DataElement::Binary(inner)) = element(&seg, 1, 0)
            && let Ok(inner) = wire::tokenize(inner)
        {
            all.extend(inner);
        }
        all.push(seg);
    }
    all
}

/// The account a statement request is about: the IBAN alias of a KTI,
/// else the number alias of a KTI or KTV.
fn account_of(request: &[RawSegment]) -> Option<String> {
    let seg = request
        .iter()
        .find(|s| matches!(code(s), "HKKAZ" | "HKCAZ"))?;
    let first = text(seg, 1, 0).unwrap_or_default();
    let iban_like = first.len() >= 15
        && first[..2].bytes().all(|b| b.is_ascii_uppercase())
        && first[2..4].bytes().all(|b| b.is_ascii_digit());
    // KTI without IBAN (`:BIC:number:sub:280:BLZ`), else KTV
    // (`number:sub:280:BLZ`).
    let id = if !iban_like && seg.degs[1].0.len() >= 6 {
        text(seg, 1, 2).unwrap_or_default()
    } else {
        first
    };
    comparable(id)
}

// ── camt ────────────────────────────────────────────────────────────────

/// The text of every `<tag>…</tag>` in `xml` (no attributes on the tag).
fn texts<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let from = &rest[at + open.len()..];
        let Some(end) = from.find(&close) else { break };
        out.push(&from[..end]);
        rest = &from[end + close.len()..];
    }
    out
}

/// The text of `xml` without its tags.
fn strip_tags(xml: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in xml.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// The entries of a camt document; `booked` when the entry has no status.
fn camt_entries(xml: &str, booked: bool) -> Vec<(bool, Option<String>, Vec<String>, bool)> {
    texts(xml, "Ntry")
        .into_iter()
        .map(|ntry| {
            let head = ntry.split("<NtryDtls>").next().unwrap_or(ntry);
            let status = texts(head, "Sts").first().map(|s| strip_tags(s));
            let booked = match status.as_deref() {
                Some("BOOK") => true,
                Some("PDNG") | Some("INFO") => false,
                _ => booked,
            };
            let raw = texts(head, "AcctSvcrRef").first().map(|s| s.to_string());
            let masked = raw.as_deref().is_some_and(|r| comparable(r).is_none());
            let reference = raw.as_deref().and_then(comparable);
            let e2e = texts(ntry, "EndToEndId")
                .into_iter()
                .filter_map(comparable)
                .collect();
            (booked, reference, e2e, masked)
        })
        .collect()
}

// ── MT940 ───────────────────────────────────────────────────────────────

/// The fields of a SWIFT message: (tag, value).
fn fields(text: &str) -> Vec<(&str, &str)> {
    let b = text.as_bytes();
    let mut starts = Vec::new();
    for at in 0..b.len() {
        if (at == 0 || b[at - 1] == b'\n') && b[at] == b':' {
            let tag_len = b[at + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric())
                .count();
            if (2..=3).contains(&tag_len) && b.get(at + 1 + tag_len) == Some(&b':') {
                starts.push((at, tag_len));
            }
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &(at, len))| {
            let end = starts.get(n + 1).map_or(b.len(), |&(next, _)| next);
            (&text[at + 1..at + 1 + len], &text[at + len + 2..end])
        })
        .collect()
}

const SEPA_KEYWORDS: [&str; 13] = [
    "EREF", "KREF", "MREF", "CRED", "DEBT", "COAM", "OAMT", "SVWZ", "ABWA", "ABWE", "IBAN", "BIC",
    "PURP",
];

/// The `EREF+` value of a structured `:86:` (purpose subfields `?20` to
/// `?29`, `?60` to `?63` read as one text, up to the next keyword).
fn eref(details: &str) -> Option<String> {
    let d: String = details
        .chars()
        .filter(|&c| c != '\r' && c != '\n')
        .collect();
    let b = d.as_bytes();
    if b.len() < 4 || !b[..3].iter().all(u8::is_ascii_digit) || b[3].is_ascii_alphanumeric() {
        return None;
    }
    let sep = char::from(b[3]);
    let mut purpose = String::new();
    for part in d[3..].split(sep).skip(1) {
        let (code, value) = part.split_at(part.len().min(2));
        if code.starts_with('2') || matches!(code, "60" | "61" | "62" | "63") {
            purpose.push_str(value);
        }
    }
    let start = purpose.find("EREF+")? + 5;
    let rest = &purpose[start..];
    let end = SEPA_KEYWORDS
        .iter()
        .filter_map(|k| rest.find(&format!("{k}+")))
        .min()
        .unwrap_or(rest.len());
    comparable(&rest[..end])
}

/// `:61:` customer reference and bank reference.
fn statement_line(line: &str) -> (Option<String>, Option<String>) {
    let line = line.lines().next().unwrap_or_default();
    let b = line.as_bytes();
    let digits = |from: usize, n: usize| {
        b.get(from..from + n)
            .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
    };
    if !digits(0, 6) {
        return (None, None);
    }
    let mut p = 6;
    if digits(p, 4) {
        p += 4;
    }
    if line[p..].starts_with("RC") || line[p..].starts_with("RD") {
        p += 2;
    } else if line[p..].starts_with('C') || line[p..].starts_with('D') {
        p += 1;
    } else {
        return (None, None);
    }
    if b.get(p).is_some_and(u8::is_ascii_alphabetic) {
        p += 1;
    }
    while p < b.len() && (b[p].is_ascii_digit() || b[p] == b',' || b[p] == b'#') {
        p += 1;
    }
    p = (p + 4).min(b.len());
    let rest = &line[p..];
    let (customer, bank) = match rest.split_once("//") {
        Some((c, r)) => (c, Some(r)),
        None => (rest, None),
    };
    let customer = Some(customer)
        .filter(|c| *c != "NONREF")
        .and_then(comparable);
    (customer, bank.map(|r| r.trim().to_string()))
}

fn mt940_entries(data: &str) -> Vec<(Option<String>, Vec<String>, bool)> {
    let fields = fields(data);
    let mut out = Vec::new();
    for (n, &(tag, value)) in fields.iter().enumerate() {
        if tag != "61" {
            continue;
        }
        let (customer, bank) = statement_line(value);
        let masked = bank.as_deref().is_some_and(|r| comparable(r).is_none());
        let mut e2e: Vec<String> = customer.into_iter().collect();
        if let Some(&("86", details)) = fields.get(n + 1) {
            e2e.extend(eref(details));
        }
        out.push((bank.as_deref().and_then(comparable), e2e, masked));
    }
    out
}

// ── The report ──────────────────────────────────────────────────────────

/// Reads the traces (in this order) and answers the identity questions.
pub fn analyse(traces: &[Trace]) -> Report {
    let mut entries: Vec<Entry> = Vec::new();
    let mut masked_references = 0;
    let mut sent: Vec<String> = Vec::new();
    let mut reported: BTreeSet<String> = BTreeSet::new();
    for (t, trace) in traces.iter().enumerate() {
        for ex in trace.exchanges.iter().filter(|ex| ex.error.is_none()) {
            let request = segments(&ex.request);
            let response = segments(&ex.response);
            for seg in &request {
                if matches!(code(seg), "HKCCS" | "HKIPZ") {
                    for pain in binaries(seg, 3) {
                        for id in texts(&latin1(pain), "EndToEndId") {
                            if let Some(id) = comparable(id)
                                && !sent.contains(&id)
                            {
                                sent.push(id);
                            }
                        }
                    }
                }
            }
            let account = account_of(&request)
                .or_else(|| ex.account.clone())
                .unwrap_or_else(|| "?".into());
            let mut push = |booked: bool, reference: Option<String>, e2e: Vec<String>| {
                entries.push(Entry {
                    trace: t,
                    account: account.clone(),
                    booked,
                    reference,
                    end_to_end: e2e,
                });
            };
            for seg in &response {
                match code(seg) {
                    "HIKAZ" => {
                        for (de, booked) in [(1, true), (2, false)] {
                            for data in binaries(seg, de) {
                                for (r, e, masked) in mt940_entries(&latin1(data)) {
                                    masked_references += usize::from(masked);
                                    push(booked, r, e);
                                }
                            }
                        }
                    }
                    "HICAZ" => {
                        for (de, booked) in [(3, true), (4, false)] {
                            for data in binaries(seg, de) {
                                let xml = String::from_utf8_lossy(data);
                                for (b, r, e, masked) in camt_entries(&xml, booked) {
                                    masked_references += usize::from(masked);
                                    push(b, r, e);
                                }
                            }
                        }
                    }
                    "HIVPP" | "HIIPS" => {
                        let de = if code(seg) == "HIVPP" { 5 } else { 3 };
                        for pain in binaries(seg, de) {
                            let xml = latin1(pain);
                            reported.extend(
                                texts(&xml, "OrgnlEndToEndId")
                                    .into_iter()
                                    .filter_map(comparable),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut report = Report {
        traces: traces.len(),
        booked: entries.iter().filter(|e| e.booked).count(),
        pending: entries.iter().filter(|e| !e.booked).count(),
        with_reference: entries.iter().filter(|e| e.reference.is_some()).count(),
        masked_references,
        ..Report::default()
    };

    // Repeats within one statement (one trace) of an account.
    let mut counts: BTreeMap<(usize, &str, &str), usize> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.booked) {
        if let Some(r) = &e.reference {
            *counts.entry((e.trace, &e.account, r)).or_default() += 1;
        }
    }
    report.repeated = counts
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|((trace, account, reference), count)| Repeat {
            trace,
            account: account.to_string(),
            reference: reference.to_string(),
            count,
        })
        .collect();

    // Pending in one trace, booked in another: by reference only, the
    // accounts are aliases of their own trace.
    let mut booked_in: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for e in entries.iter().filter(|e| e.booked) {
        if let Some(r) = &e.reference {
            booked_in.entry(r).or_default().insert(e.trace);
        }
    }
    let mut seen = BTreeSet::new();
    for e in entries.iter().filter(|e| !e.booked) {
        let Some(r) = &e.reference else { continue };
        report.pending_with_reference += 1;
        let others: Vec<usize> = booked_in
            .get(r.as_str())
            .into_iter()
            .flatten()
            .copied()
            .filter(|&t| t != e.trace)
            .collect();
        if !others.is_empty() && seen.insert((e.account.clone(), r.clone(), e.trace)) {
            report.pending_booked.push(PendingBooked {
                account: e.account.clone(),
                reference: r.clone(),
                pending_in: e.trace,
                booked_in: others,
            });
        }
    }

    // End-to-end IDs.
    let on_entries: BTreeSet<&str> = entries
        .iter()
        .flat_map(|e| e.end_to_end.iter().map(String::as_str))
        .collect();
    report.sent = sent
        .into_iter()
        .map(|id| SentEndToEnd {
            in_status_report: reported.contains(&id),
            on_entry: on_entries.contains(id.as_str()),
            id,
        })
        .collect();
    report
}

impl fmt::Display for Report {
    /// The report for humans: counts, then the aliases and pseudonyms
    /// behind them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} traces; {} booked and {} pending entries; {} with a pseudonymised booking \
             reference, {} with a masked one (no pseudonym, not compared)",
            self.traces, self.booked, self.pending, self.with_reference, self.masked_references
        )?;
        writeln!(
            f,
            "booking references repeated within one statement of an account: {}",
            self.repeated.len()
        )?;
        for r in &self.repeated {
            writeln!(
                f,
                "  {} x{} (trace {}, account {})",
                r.reference, r.count, r.trace, r.account
            )?;
        }
        writeln!(
            f,
            "pending entries with a reference: {}; on a booked entry in another trace \
             (same salt): {}",
            self.pending_with_reference,
            self.pending_booked.len()
        )?;
        for p in &self.pending_booked {
            let traces: Vec<String> = p.booked_in.iter().map(|t| t.to_string()).collect();
            writeln!(
                f,
                "  {} pending in trace {} (account {}), booked in trace {}",
                p.reference,
                p.pending_in,
                p.account,
                traces.join(", ")
            )?;
        }
        let status = self.sent.iter().filter(|s| s.in_status_report).count();
        let entry = self.sent.iter().filter(|s| s.on_entry).count();
        writeln!(
            f,
            "end-to-end IDs sent in a pain.001: {}; in a pain.002: {status}; on an entry: {entry}",
            self.sent.len()
        )?;
        let yes = |b: bool| if b { "yes" } else { "no" };
        for s in &self.sent {
            writeln!(
                f,
                "  {}: pain.002 {}, entry {}",
                s.id,
                yes(s.in_status_report),
                yes(s.on_entry)
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fints_client::trace::Exchange;

    const IBAN: &str = "DE47120300009382716455";
    const OTHER_ALIAS: &str = "DE70120300004410293857";

    fn message(inner: &str) -> Vec<u8> {
        format!("HNHBK:1:3+000000000100+300+DIALOG0001+2'{inner}HNHBS:9:1+2'").into_bytes()
    }

    fn bin(data: &str) -> String {
        format!("@{}@{data}", data.len())
    }

    fn trace(exchanges: Vec<(String, String)>) -> Trace {
        Trace {
            exchanges: exchanges
                .into_iter()
                .map(|(request, response)| Exchange {
                    request: message(&request),
                    response: message(&response),
                    error: None,
                    account: Some("#1".into()),
                    notes: Vec::new(),
                })
                .collect(),
            ..Trace::default()
        }
    }

    fn ntry(status: &str, reference: &str, e2e: &str) -> String {
        format!(
            "<Ntry><Amt Ccy=\"EUR\">1.11</Amt><Sts><Cd>{status}</Cd></Sts>\
             <AcctSvcrRef>{reference}</AcctSvcrRef><NtryDtls><TxDtls><Refs>\
             <EndToEndId>{e2e}</EndToEndId></Refs></TxDtls></NtryDtls></Ntry>"
        )
    }

    fn camt(entries: &[String]) -> String {
        format!(
            "<Document><BkToCstmrAcctRpt><Rpt>{}</Rpt></BkToCstmrAcctRpt></Document>",
            entries.concat()
        )
    }

    /// Trace 0: camt, a booking reference twice, a pending entry with an
    /// end-to-end ID and one entry still shape-masked. Trace 1 (same
    /// salt): MT940, the pending entry booked, the account under another
    /// alias. Trace 2: a transfer of three end-to-end IDs,
    /// one reported in the pain.002.
    fn traces() -> Vec<Trace> {
        let camt_request = format!("HKCAZ:3:1+{IBAN}:TESTDEFFXXX:::+urn?:x+N'");
        let booked = camt(&[
            ntry("BOOK", "4821-07-19-55.20.31.904417", "NOTPROVIDED"),
            ntry("BOOK", "4821-07-19-55.20.31.904417", "NOTPROVIDED"),
            ntry("BOOK", "####-##-##-##.##.##.######", "###"),
        ]);
        let pending = camt(&[ntry("PDNG", "9035-61-02-18.44.07.215590", "QWE-8812")]);
        let t0 = trace(vec![(
            camt_request,
            format!(
                "HICAZ:3:1:3+{IBAN}:TESTDEFFXXX:::+urn?:x+{}+{}'",
                bin(&booked),
                bin(&pending)
            ),
        )]);
        let mt940 = ":20:XXXX\r\n:25:12030000/0006612094\r\n:28C:1/1\r\n:60F:C260101EUR1,\r\n\
            :61:2601020102DR1,NDDTNONREF//9035-61-02-18.44.07.215590\r\n\
            :86:105?00XXXX?20EREF+ZUI-?214410 SVWZ+XXXX\r\n:62F:C260102EUR1,\r\n-";
        let t1 = trace(vec![(
            // Another trace: the same account has another alias.
            format!("HKKAZ:3:7+{OTHER_ALIAS}:TESTDEFFXXX:::+N'"),
            format!("HIKAZ:3:7:3+{}'", bin(mt940)),
        )]);
        let pain001 = "<Document><CstmrCdtTrfInitn><PmtInf>\
            <CdtTrfTxInf><PmtId><EndToEndId>QWE-8812</EndToEndId></PmtId></CdtTrfTxInf>\
            <CdtTrfTxInf><PmtId><EndToEndId>ZUI-4410</EndToEndId></PmtId></CdtTrfTxInf>\
            <CdtTrfTxInf><PmtId><EndToEndId>LKJ-0007</EndToEndId></PmtId></CdtTrfTxInf>\
            </PmtInf></CstmrCdtTrfInitn></Document>";
        let pain002 = "<Document><CstmrPmtStsRpt><OrgnlPmtInfAndSts><TxInfAndSts>\
            <OrgnlEndToEndId>QWE-8812</OrgnlEndToEndId><TxSts>ACCP</TxSts>\
            </TxInfAndSts></OrgnlPmtInfAndSts></CstmrPmtStsRpt></Document>";
        let t2 = trace(vec![(
            format!("HKCCS:3:1+{IBAN}:TESTDEFFXXX:::+urn?:x+{}'", bin(pain001)),
            format!("HIVPP:3:1:3+++++{}'", bin(pain002)),
        )]);
        vec![t0, t1, t2]
    }

    #[test]
    fn finds_repeats_pending_bookings_and_end_to_end_ids() {
        let report = analyse(&traces());
        assert_eq!(report.traces, 3);
        assert_eq!((report.booked, report.pending), (4, 1));
        assert_eq!(report.with_reference, 4);
        assert_eq!(report.masked_references, 1);
        assert_eq!(
            report.repeated,
            [Repeat {
                trace: 0,
                account: IBAN.into(),
                reference: "4821-07-19-55.20.31.904417".into(),
                count: 2,
            }]
        );
        assert_eq!(report.pending_with_reference, 1);
        assert_eq!(
            report.pending_booked,
            [PendingBooked {
                account: IBAN.into(),
                reference: "9035-61-02-18.44.07.215590".into(),
                pending_in: 0,
                booked_in: vec![1],
            }]
        );
        let sent: Vec<(&str, bool, bool)> = report
            .sent
            .iter()
            .map(|s| (s.id.as_str(), s.in_status_report, s.on_entry))
            .collect();
        assert_eq!(
            sent,
            [
                ("QWE-8812", true, true),
                ("ZUI-4410", false, true),
                ("LKJ-0007", false, false),
            ]
        );
        let text = report.to_string();
        assert!(
            text.contains("repeated within one statement of an account: 1"),
            "{text}"
        );
        assert!(text.contains("in a pain.002: 1; on an entry: 2"), "{text}");
    }

    #[test]
    fn traces_without_pseudonyms_give_no_matches() {
        let committed = Trace::from_json(include_str!(
            "../fixtures/replay/synthetic-decoupled.trace.json"
        ))
        .expect("fixture loads");
        let report = analyse(&[committed.clone(), committed]);
        assert!(report.booked > 0);
        assert_eq!(report.with_reference, 0);
        assert!(report.repeated.is_empty());
        assert!(report.pending_booked.is_empty());
        assert!(report.sent.is_empty());
    }
}
