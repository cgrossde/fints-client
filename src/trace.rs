//! Recording dialogs as masked, anonymised traces (feature `trace`).
//!
//! - [`Recorder`] decorates a [`Transport`] ([`Recorder::wrap`]) and keeps
//!   every exchange that goes through the wrapped transport, masked as it
//!   is recorded, so credentials never sit in memory unmasked: PIN, TAN,
//!   challenges, subaccounts and binary data other than statements, pain
//!   messages and IDs become `*`; names, product names, amounts and the
//!   statements in HIKAZ, HICAZ, HIWPD and HIWDU keep only their shape,
//!   values as `#` in fixed lengths, and every date is moved by the same
//!   few days. Identity values get a stand-in. Account identifiers (IBANs,
//!   account numbers, user and customer ID, creditor IDs) get aliases
//!   numbered in the order they first appear in this trace, from a random
//!   start, with no link to the value; the same value gets the same alias
//!   everywhere in the trace, IBANs keep country and bank code and valid
//!   check digits, and an IBAN's account part matches the alias of the
//!   same account number. The table lives in the recorder's memory only.
//!   References and IDs (booking references, end-to-end, message and
//!   payment information IDs, mandate references, system ID, order, VOP
//!   and polling IDs) get keyed pseudonyms. Their key is a [`Salt`]:
//!   [`Recorder::new`] draws one and forgets it, so the trace links to no
//!   other; traces recorded with the same salt ([`Recorder::with_salt`])
//!   give a reference the same pseudonym, while their account aliases stay
//!   unrelated. The library writes no salt. It records exactly
//!   the dialogs that use the wrapped transport, whatever thread they run
//!   on; two recorders never see each other's traffic. A failed send is
//!   recorded with a short, content-free `error`. It also names the account
//!   each request is about ([`Exchange::account`], an alias such as `#2`
//!   described by a [`TraceAccount`]): aliases are numbered in the order the
//!   accounts first appear in a HIUPD, and the recorder keeps only keyed
//!   hashes of their identifiers (a per-recorder random key that is never
//!   written), never the identifiers themselves.
//! - [`Trace`] is the fixture: one JSON document ([`TRACE_FORMAT`], see
//!   [`Trace::to_json`]). [`Trace::from_exchanges`] anonymises the rest.
//!   [`Trace::summary`] reads it for a human, one [`ExchangeSummary`] per
//!   exchange.
//!
//! Independent of the `TRACE` event the transport boundary emits for log
//! consumers ([`mask::EXCHANGE_TARGET`]).
//!
//! Clean room: written from the FinTS 3.0 specification and our own masking
//! rules ([`mask`]).

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasher, RandomState};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codec::{DataElement, RawSegment, wire};
use crate::error::{FinTSError, Result};
use crate::jobs::AccountInfo;
use crate::mask;
use crate::mask::pseudonym::Identities;
pub use crate::mask::pseudonym::Salt;
use crate::return_code::Kind;
use crate::transport::{BoxFuture, SharedTransport, Transport};
use crate::types::SepaAccount;

/// The `format` of a trace file written by [`Trace::to_json`].
pub const TRACE_FORMAT: &str = "fints-client-trace/4";

/// One request and its response, as wire bytes (ISO 8859-1, with the
/// PIN/TAN envelope), redacted; or the request and the error that ended it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    /// The customer message.
    pub request: Vec<u8>,
    /// The bank's answer; empty if `error` is set.
    pub response: Vec<u8>,
    /// Short summary of why the exchange failed (`HTTP 500`,
    /// `transport: ...`), never message content.
    pub error: Option<String>,
    /// The alias (`#1`, `#2`, …) of the account the request is about, one of
    /// [`Trace::accounts`]; set by the [`Recorder`].
    pub account: Option<String>,
    /// What this client made of the answer, without any value of it: a
    /// segment its definition could not read (code, version, field, kind of
    /// error) and statements with characters outside the SWIFT character
    /// set. Written by the [`Recorder`] from the answer before masking.
    pub notes: Vec<String>,
}

/// The client that recorded a trace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Client {
    /// The version of fints-client.
    pub version: String,
    /// How the recorded run ended: `completed`, or the kind of error that
    /// stopped it ([`Trace::set_outcome`], [`outcome_label`]), never an
    /// error text. `None` when the caller did not say.
    pub outcome: Option<String>,
}

/// An account of a trace, by alias only: what the UPD says about it, never
/// an identifier, a name or the product name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceAccount {
    /// `#1`, `#2`, …: numbered in the order the accounts first appear in a
    /// HIUPD.
    pub alias: String,
    /// The [`crate::types::AccountClass`] of the Kontoart (`checking`,
    /// `depot`, …), or `depot` when there is none and the jobs allow HKWPD
    /// and none of HKSAL, HKKAZ, HKCAZ ([`AccountInfo::class`]).
    pub class: Option<String>,
    /// The Kontoart of the HIUPD.
    pub kontoart: Option<u8>,
    /// The jobs the UPD allows for the account (`HKSAL`, `HKKAZ`, …).
    pub jobs: Vec<String>,
}

impl fmt::Display for TraceAccount {
    /// `account #2 (class depot; Kontoart -; jobs HKWPD)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let jobs = if self.jobs.is_empty() {
            "-".to_string()
        } else {
            self.jobs.join(" ")
        };
        write!(
            f,
            "account {} (class {}; Kontoart {}; jobs {jobs})",
            self.alias,
            self.class.as_deref().unwrap_or("-"),
            self.kontoart
                .map_or_else(|| "-".to_string(), |k| k.to_string()),
        )
    }
}

/// A recorded dialog: the exchanges in order, and the accounts they name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    /// Free text for readers (where and how the trace was recorded).
    pub comment: String,
    /// The client that recorded it, and how its run ended.
    pub client: Client,
    /// Every account an exchange may name, listed once.
    pub accounts: Vec<TraceAccount>,
    /// The exchanges, in the order they happened.
    pub exchanges: Vec<Exchange>,
}

impl Trace {
    /// A trace from redacted exchanges (as [`Recorder::exchanges`] returns
    /// them), anonymised further; no comment, no accounts:
    ///
    /// - dialog IDs (HNHBK) and order references (HITAN) become
    ///   `DIALOG0001`, `TASK0001`, … everywhere they occur;
    /// - the product registration in HKVVB becomes stars.
    ///
    /// Amounts, names and statements are shape-masked by the recorder
    /// already.
    pub fn from_exchanges(exchanges: Vec<Exchange>) -> Trace {
        let mut names: HashMap<String, String> = HashMap::new();
        let (mut dialogs, mut tasks) = (0, 0);
        for ex in &exchanges {
            for_each_segment(&ex.response, |seg| {
                let (kind, counter) = match code(seg) {
                    "HNHBK" => ("DIALOG", &mut dialogs),
                    "HITAN" => ("TASK", &mut tasks),
                    _ => return,
                };
                // HNHBK 4 Dialog-ID, HITAN 4 Auftragsreferenz.
                if let Some(v) = text(seg, 3, 0)
                    && v != "0"
                    && v != "noref"
                    && !is_starred(v)
                    && !names.contains_key(v)
                {
                    *counter += 1;
                    names.insert(v.to_string(), format!("{kind}{:04}", *counter));
                }
            });
        }
        let anonymise = |seg: &mut RawSegment| {
            let code = code(seg).to_string();
            for deg in seg.degs.iter_mut().skip(1) {
                for de in deg.0.iter_mut() {
                    let DataElement::Text(t) = de else { continue };
                    if let Some(name) = names.get(t.as_str()) {
                        *t = name.clone();
                    }
                }
            }
            if code == "HKVVB"
                && let Some(DataElement::Text(t)) = seg.degs.get_mut(4).and_then(|d| d.0.get_mut(0))
            {
                *t = "*".repeat(t.chars().count());
            }
        };
        let exchanges = exchanges
            .into_iter()
            .map(|ex| Exchange {
                request: map_message(&ex.request, anonymise).unwrap_or(ex.request),
                response: map_message(&ex.response, anonymise).unwrap_or(ex.response),
                error: ex.error,
                account: ex.account,
                notes: ex.notes,
            })
            .collect();
        Trace {
            exchanges,
            ..Trace::default()
        }
    }

    /// Masks the trace again, with the masking rules of this version of
    /// fints-client: for a trace recorded before the rules hid more. The
    /// stand-ins are made anew (aliases and pseudonyms of a fresh random
    /// salt, so they link to no other trace) and every date moves by a
    /// further 3 to 30 days. What the trace already hides stays hidden;
    /// unknown segments stay as they are, starred or kept. `comment`,
    /// `client`, `accounts`, and each exchange's `account`, `error` and
    /// `notes` stay.
    pub fn remask(&mut self) {
        let ids = Identities::new(Salt::random()).with_days(Identities::random_days());
        let exchanges = std::mem::take(&mut self.exchanges)
            .into_iter()
            .map(|ex| Exchange {
                request: mask::mask_for_recorder(&ex.request, &ids),
                response: mask::mask_for_recorder(&ex.response, &ids),
                ..ex
            })
            .collect();
        self.exchanges = Trace::from_exchanges(exchanges).exchanges;
    }

    /// Records how the run ended: `completed`, or the kind of the error
    /// that stopped it (the variant, for a bank error its code), never its
    /// text.
    pub fn set_outcome(&mut self, result: std::result::Result<(), &FinTSError>) {
        self.client.outcome = Some(match result {
            Ok(()) => "completed".to_string(),
            Err(e) => outcome_label(e),
        });
    }

    /// The trace as one pretty-printed JSON document:
    ///
    /// - `format` ([`TRACE_FORMAT`]), `comment` (lines), `bank` (`blz`, `bic`:
    ///   [`Trace::blz`], [`Trace::bic`]), `accounts` ([`TraceAccount`]s);
    /// - `exchanges`, each with `index`, `account` (alias or null),
    ///   `retry_of`, `sent` (`jobs` as `CODE:version`, `tan` as
    ///   `{process, for}` or null), `answer` (`message`: HIRMG codes,
    ///   `segments`: HIRMS codes with the `job` they refer to), `failed`,
    ///   `error`, `request` and `response`.
    ///
    /// `request` and `response` hold the message, one string per segment
    /// (the HNVSD header up to its `@len@` and each segment inside HNVSD get
    /// their own; the closing `'` of HNVSD is a string of its own), each byte as the ISO 8859-1 character
    /// of the same value: joined, they are the bytes again. They are the
    /// only source of truth; `index`, `retry_of`, `sent`, `answer`, `failed`
    /// and `bank` are derived from them for readers (see [`Trace::summary`]),
    /// and [`Trace::from_json`] rejects a file in which they differ.
    pub fn to_json(&self) -> String {
        let mut json =
            serde_json::to_string_pretty(&self.file()).expect("a trace serializes to JSON");
        json.push('\n');
        json
    }

    /// Reads a trace written by [`Trace::to_json`]. Fails on another
    /// `format`, unknown fields, a character outside ISO 8859-1, an
    /// exchange naming an alias that `accounts` does not list, a failed
    /// exchange with a response, and any derived field that differs from
    /// what the raw `request`/`response` give (the error names the exchange
    /// and the field).
    pub fn from_json(json: &str) -> std::result::Result<Trace, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| format!("not JSON: {e}"))?;
        let file: TraceFile = serde_json::from_value(value.clone())
            .map_err(|e| format!("not a {TRACE_FORMAT} trace: {e}"))?;
        if file.format != TRACE_FORMAT {
            return Err(format!(
                "format {:?}; this version reads {TRACE_FORMAT:?}",
                file.format
            ));
        }
        for (i, a) in file.accounts.iter().enumerate() {
            if file.accounts[..i].iter().any(|b| b.alias == a.alias) {
                return Err(format!("account {:?} is listed twice", a.alias));
            }
        }
        let mut exchanges = Vec::with_capacity(file.exchanges.len());
        for (i, ex) in file.exchanges.into_iter().enumerate() {
            if let Some(alias) = &ex.account
                && !file.accounts.iter().any(|a| &a.alias == alias)
            {
                return Err(format!(
                    "exchange {i}: account {alias:?} is not in `accounts`"
                ));
            }
            if ex.error.is_some() && !ex.response.is_empty() {
                return Err(format!(
                    "exchange {i}: a failed exchange (`error` set) has no `response`"
                ));
            }
            exchanges.push(Exchange {
                request: join_segments(&ex.request)
                    .map_err(|e| format!("exchange {i}: request: {e}"))?,
                response: join_segments(&ex.response)
                    .map_err(|e| format!("exchange {i}: response: {e}"))?,
                error: ex.error,
                account: ex.account,
                notes: ex.notes,
            });
        }
        let trace = Trace {
            comment: file.comment.join("\n"),
            client: file.client,
            accounts: file.accounts,
            exchanges,
        };
        let derived = serde_json::to_value(trace.file()).expect("a trace serializes to JSON");
        if let Some((path, found, want)) =
            first_difference("bank", &value["bank"], &derived["bank"])
        {
            return Err(format!(
                "derived field {path} is {found} in the file, but the requests give {want}"
            ));
        }
        for (i, want) in derived["exchanges"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let found = &value["exchanges"][i];
            for field in ["index", "retry_of", "sent", "answer", "failed"] {
                let path = field.to_string();
                if let Some((path, found, want)) =
                    first_difference(&path, &found[field], &want[field])
                {
                    return Err(format!(
                        "exchange {i}: derived field {path} is {found} in the file, \
                         but the recorded request and response give {want}"
                    ));
                }
            }
        }
        Ok(trace)
    }

    /// The bank code the client logged in at (HKIDN of the first request
    /// that has one).
    pub fn blz(&self) -> Option<String> {
        let mut blz = None;
        for ex in &self.exchanges {
            for_each_segment(&ex.request, |seg| {
                if blz.is_none() && code(seg) == "HKIDN" {
                    blz = text(seg, 1, 1).map(str::to_string);
                }
            });
        }
        blz
    }

    /// The bank's BIC: the BIC of the first account element (KTI,
    /// `IBAN:BIC`) a job of a request names.
    pub fn bic(&self) -> Option<String> {
        let mut bic = None;
        for ex in &self.exchanges {
            for_each_segment(&ex.request, |seg| {
                if bic.is_none()
                    && ACCOUNT_JOBS.contains(&code(seg))
                    && let Some(b) = text(seg, 1, 1).filter(|b| is_bic(b))
                {
                    bic = Some(b.to_string());
                }
            });
        }
        bic
    }

    /// The account with this alias.
    pub fn account(&self, alias: &str) -> Option<&TraceAccount> {
        self.accounts.iter().find(|a| a.alias == alias)
    }

    /// One [`ExchangeSummary`] per exchange. Never fails: a message that
    /// does not tokenize is reported as unreadable.
    ///
    /// An exchange is a retry of the one before it (`retry_of`) when that
    /// one was answered with 9075 (strong customer authentication required)
    /// and it sends the same jobs again, now with HKTAN.
    pub fn summary(&self) -> Vec<ExchangeSummary> {
        let mut all: Vec<ExchangeSummary> = self
            .exchanges
            .iter()
            .enumerate()
            .map(|(i, ex)| summarize(i, ex))
            .collect();
        for i in 1..all.len() {
            let (prev, cur) = (&all[i - 1], &all[i]);
            let sca_required = prev
                .global
                .iter()
                .chain(prev.segment.iter().map(|(_, c)| c))
                .any(|c| c.kind() == Some(Kind::ScaRequired));
            let same_jobs = |a: &[String], b: &[String]| {
                let (mut a, mut b) = (a.to_vec(), b.to_vec());
                a.sort();
                b.sort();
                a == b
            };
            if sca_required
                && cur.tan.is_some()
                && !cur.jobs.is_empty()
                && same_jobs(&prev.jobs, &cur.jobs)
            {
                all[i].retry_of = Some(i - 1);
            }
        }
        all
    }

    /// The summary as a table for humans: [`SUMMARY_LEGEND`], the accounts
    /// (one line each), then one line per exchange.
    pub fn summary_table(&self) -> String {
        let mut out = format!("{SUMMARY_LEGEND}\n");
        if !self.client.version.is_empty() {
            out.push_str(&format!(
                "fints-client {}, run {}\n",
                self.client.version,
                self.client.outcome.as_deref().unwrap_or("not stated")
            ));
        }
        for account in &self.accounts {
            out.push_str(&format!("{account}\n"));
        }
        for (line, ex) in self.summary().into_iter().zip(&self.exchanges) {
            out.push_str(&format!("{line}\n"));
            for note in &ex.notes {
                out.push_str(&format!("      note: {note}\n"));
            }
        }
        out
    }

    /// The JSON document, derived fields computed from the raw messages.
    fn file(&self) -> TraceFile {
        let exchanges = self
            .exchanges
            .iter()
            .zip(self.summary())
            .map(|(ex, s)| ExchangeJson {
                index: s.index,
                account: ex.account.clone(),
                retry_of: s.retry_of,
                failed: s.failed(),
                sent: Sent {
                    jobs: s.jobs,
                    tan: s.tan.map(|(process, for_segment)| Tan {
                        process,
                        for_segment,
                    }),
                },
                answer: Answer {
                    message: s.global,
                    segments: s
                        .segment
                        .into_iter()
                        .map(|(job, c)| SegmentCode {
                            job,
                            code: c.code,
                            text: c.text,
                        })
                        .collect(),
                },
                error: ex.error.clone(),
                notes: ex.notes.clone(),
                request: split_segments(&ex.request),
                response: split_segments(&ex.response),
            })
            .collect();
        TraceFile {
            format: TRACE_FORMAT.to_string(),
            comment: self.comment.lines().map(str::to_string).collect(),
            client: self.client.clone(),
            bank: Bank {
                blz: self.blz(),
                bic: self.bic(),
            },
            accounts: self.accounts.clone(),
            exchanges,
        }
    }
}

// ── Rebuilding shapes ───────────────────────────────────────────────────

/// The binary data of a recorded answer segment made valid again, if the
/// segment holds a shape-masked statement (HIKAZ: MT940, HICAZ: camt.052,
/// HIWPD/HIWDU: MT535/536): each `#` becomes `1` in an amount and `X` in
/// text, read with the same field rules as the mask, so the result has the
/// bank's layout. `None` for other segments.
pub fn unshape_statement(segment: &str, data: &[u8]) -> Option<Vec<u8>> {
    mask::shape::Format::of(segment).map(|f| mask::shape::unshape(f, data))
}

/// A shape-masked text element made valid again: `####,##` becomes
/// `1111,11`, any other `#` becomes `X`.
pub fn unshape_text(text: &str) -> String {
    mask::shape::unshape_text(text)
}

// ── JSON document ───────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceFile {
    format: String,
    comment: Vec<String>,
    client: Client,
    bank: Bank,
    accounts: Vec<TraceAccount>,
    exchanges: Vec<ExchangeJson>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bank {
    blz: Option<String>,
    bic: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExchangeJson {
    index: usize,
    account: Option<String>,
    retry_of: Option<usize>,
    sent: Sent,
    answer: Answer,
    failed: bool,
    error: Option<String>,
    notes: Vec<String>,
    request: Vec<String>,
    response: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sent {
    jobs: Vec<String>,
    tan: Option<Tan>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tan {
    process: String,
    #[serde(rename = "for")]
    for_segment: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    message: Vec<Code>,
    segments: Vec<SegmentCode>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SegmentCode {
    job: String,
    code: String,
    text: String,
}

/// The first place, as a path (`sent.jobs[0]`), where `found` differs from
/// `want`, with both values there. A missing field counts as null.
fn first_difference<'a>(
    path: &str,
    found: &'a Value,
    want: &'a Value,
) -> Option<(String, &'a Value, &'a Value)> {
    match (found, want) {
        (Value::Object(f), Value::Object(w)) if f.keys().all(|k| w.contains_key(k)) => {
            w.iter().find_map(|(k, wv)| {
                first_difference(&format!("{path}.{k}"), f.get(k).unwrap_or(&Value::Null), wv)
            })
        }
        (Value::Array(f), Value::Array(w)) if f.len() == w.len() => f
            .iter()
            .zip(w)
            .enumerate()
            .find_map(|(i, (fv, wv))| first_difference(&format!("{path}[{i}]"), fv, wv)),
        _ if found == want => None,
        _ => Some((path.to_string(), found, want)),
    }
}

/// The message as one string per segment, each byte the ISO 8859-1
/// character of the same value. A segment ends at a `'` that is not
/// escaped (`?'`) and not inside binary data (`@len@…`); the binary data of
/// HNVSD is split too, since it holds the inner segments: its header up to
/// `@len@` is a string, each inner segment one, its closing `'` one. Never
/// fails: what
/// does not end in `'` is the last string.
fn split_segments(msg: &[u8]) -> Vec<String> {
    let latin1 = |b: &[u8]| b.iter().copied().map(char::from).collect::<String>();
    let mut parts = Vec::new();
    let (mut start, mut segment, mut i) = (0, 0, 0);
    while i < msg.len() {
        match msg[i] {
            b'?' => i += 2,
            b'\'' => {
                i += 1;
                parts.push(latin1(&msg[start..i]));
                start = i;
                segment = i;
            }
            b'@' => match binary_header(msg, i) {
                Some((_, data)) if msg[segment..].starts_with(b"HNVSD:") => {
                    parts.push(latin1(&msg[start..data]));
                    i = data;
                    start = data;
                    segment = data;
                }
                Some((len, data)) => i = data.saturating_add(len),
                None => i += 1,
            },
            _ => i += 1,
        }
    }
    if start < msg.len() {
        parts.push(latin1(&msg[start..]));
    }
    parts
}

/// `@len@` at `at`: the length and where the data starts.
fn binary_header(msg: &[u8], at: usize) -> Option<(usize, usize)> {
    let digits = msg[at + 1..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let end = at + 1 + digits;
    if digits == 0 || msg.get(end) != Some(&b'@') {
        return None;
    }
    let len = std::str::from_utf8(&msg[at + 1..end]).ok()?.parse().ok()?;
    Some((len, end + 1))
}

/// The bytes of [`split_segments`] again.
fn join_segments(parts: &[String]) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for (n, part) in parts.iter().enumerate() {
        for c in part.chars() {
            out.push(
                u8::try_from(u32::from(c))
                    .map_err(|_| format!("string {n}: {c:?} is not an ISO 8859-1 character"))?,
            );
        }
    }
    Ok(out)
}

// ── Summary ─────────────────────────────────────────────────────────────

/// The first line of [`Trace::summary_table`]: how to read the lines below.
pub const SUMMARY_LEGEND: &str = "#ex  jobs +HKTAN(process,job)  | HIRMG codes | job HIRMS codes | transport error  \
     FAILED = 9xxx or error  retry of #ex = resent with HKTAN after 9075  account #n; \
     texts for 9xxx only";

/// A return code with its text (one element of HIRMG or HIRMS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Code {
    /// The four-digit code, e.g. `9050`.
    pub code: String,
    /// The bank's text, as recorded.
    pub text: String,
}

impl Code {
    fn is_error(&self) -> bool {
        self.code.starts_with('9')
    }

    /// The code's kind; `None` if the recorded code is not four digits.
    fn kind(&self) -> Option<Kind> {
        crate::return_code::Code::new(&self.code).map(|c| Kind::of(c, &[]))
    }
}

/// One exchange of a trace, read for a human: what was asked, what the
/// bank answered. Built by [`Trace::summary`]; never fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeSummary {
    /// Position in the trace, from 0.
    pub index: usize,
    /// Every non-envelope request segment except HKTAN, as `CODE:version`
    /// (`(unreadable)` if the request does not tokenize).
    pub jobs: Vec<String>,
    /// HKTAN's process and the segment it refers to, e.g. `("4", "HKKAZ")`.
    pub tan: Option<(String, String)>,
    /// The codes of HIRMG.
    pub global: Vec<Code>,
    /// The codes of HIRMS, with the code of the request segment each
    /// refers to.
    pub segment: Vec<(String, Code)>,
    /// The transport error, or `(unreadable response)`.
    pub error: Option<String>,
    /// The exchange's [`Exchange::account`]: the alias of the account it is
    /// about.
    pub account: Option<String>,
    /// The exchange this one repeats with HKTAN after a 9075 answer.
    pub retry_of: Option<usize>,
}

impl ExchangeSummary {
    /// True if the exchange has an error or any 9xxx code.
    pub fn failed(&self) -> bool {
        self.error.is_some()
            || self.global.iter().any(Code::is_error)
            || self.segment.iter().any(|(_, c)| c.is_error())
    }
}

impl fmt::Display for ExchangeSummary {
    /// One line:
    /// `#08  HKKAZ:7 +HKTAN(4,HKKAZ)  | 9050 | HKKAZ 9010 text  FAILED  retry of #07  account #2`
    /// (see [`SUMMARY_LEGEND`]). Texts are printed for 9xxx codes only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code_text = |c: &Code| {
            if c.is_error() && !c.text.is_empty() {
                format!("{} {}", c.code, c.text)
            } else {
                c.code.clone()
            }
        };
        write!(f, "#{:02}  {}", self.index, self.jobs.join(" "))?;
        if let Some((process, seg)) = &self.tan {
            if seg.is_empty() {
                write!(f, " +HKTAN({process})")?;
            } else {
                write!(f, " +HKTAN({process},{seg})")?;
            }
        }
        write!(
            f,
            "  | {}",
            self.global
                .iter()
                .map(code_text)
                .collect::<Vec<_>>()
                .join(" ")
        )?;
        if !self.segment.is_empty() {
            let parts: Vec<String> = self
                .segment
                .iter()
                .map(|(seg, c)| format!("{seg} {}", code_text(c)))
                .collect();
            write!(f, " | {}", parts.join(", "))?;
        }
        if let Some(e) = &self.error {
            write!(f, " | {e}")?;
        }
        if self.failed() {
            write!(f, "  FAILED")?;
        }
        if let Some(i) = self.retry_of {
            write!(f, "  retry of #{i:02}")?;
        }
        if let Some(alias) = &self.account {
            write!(f, "  account {alias}")?;
        }
        Ok(())
    }
}

/// Every segment of a message, the ones inside HNVSD first (as
/// `for_each_segment`); `None` if anything does not tokenize.
fn all_segments(msg: &[u8]) -> Option<Vec<RawSegment>> {
    let mut outer = wire::tokenize(msg).ok()?;
    let mut all = Vec::new();
    for seg in &mut outer {
        if let Some(inner) = hnvsd_payload(seg) {
            all.extend(wire::tokenize(inner).ok()?);
        }
    }
    all.extend(outer);
    Some(all)
}

/// The codes of a HIRMG/HIRMS segment: its data elements 1.. are
/// `code:reference:text:parameters`.
fn return_codes(seg: &RawSegment) -> Vec<Code> {
    (1..seg.degs.len())
        .filter_map(|i| {
            Some(Code {
                code: text(seg, i, 0)?.to_string(),
                text: text(seg, i, 2).unwrap_or_default().to_string(),
            })
        })
        .collect()
}

fn summarize(index: usize, ex: &Exchange) -> ExchangeSummary {
    const ENVELOPE: [&str; 6] = ["HNHBK", "HNVSK", "HNVSD", "HNSHK", "HNSHA", "HNHBS"];
    let mut s = ExchangeSummary {
        index,
        jobs: Vec::new(),
        tan: None,
        global: Vec::new(),
        segment: Vec::new(),
        error: ex.error.clone(),
        account: ex.account.clone(),
        retry_of: None,
    };
    let request = all_segments(&ex.request);
    let mut numbers: HashMap<u16, String> = HashMap::new();
    match &request {
        Some(segs) => {
            for seg in segs {
                numbers.insert(seg.segment_number(), code(seg).to_string());
                match code(seg) {
                    c if ENVELOPE.contains(&c) => {}
                    "HKTAN" => {
                        s.tan = Some((
                            text(seg, 1, 0).unwrap_or_default().to_string(),
                            text(seg, 2, 0).unwrap_or_default().to_string(),
                        ));
                    }
                    c => s.jobs.push(format!("{c}:{}", seg.segment_version())),
                }
            }
        }
        None => s.jobs.push("(unreadable)".into()),
    }
    if ex.error.is_none() {
        match all_segments(&ex.response) {
            Some(segs) => {
                for seg in &segs {
                    match code(seg) {
                        "HIRMG" => s.global.extend(return_codes(seg)),
                        "HIRMS" => {
                            let target = seg
                                .segment_reference()
                                .and_then(|r| numbers.get(&r).cloned())
                                .unwrap_or_else(|| "?".into());
                            s.segment
                                .extend(return_codes(seg).into_iter().map(|c| (target.clone(), c)));
                        }
                        _ => {}
                    }
                }
            }
            None => s.error = Some("(unreadable response)".into()),
        }
    }
    s
}

// ── Segments ────────────────────────────────────────────────────────────

fn code(seg: &RawSegment) -> &str {
    seg.segment_type()
}

fn text(seg: &RawSegment, de: usize, ge: usize) -> Option<&str> {
    match seg.degs.get(de)?.0.get(ge)? {
        DataElement::Text(s) => Some(s),
        _ => None,
    }
}

fn is_starred(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b == b'*')
}

/// A BIC: 8 or 11 letters and digits.
fn is_bic(s: &str) -> bool {
    matches!(s.len(), 8 | 11) && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The HNVSD payload of an outer segment, if it has one.
fn hnvsd_payload(seg: &mut RawSegment) -> Option<&mut Vec<u8>> {
    if code(seg) != "HNVSD" {
        return None;
    }
    match seg.degs.get_mut(1)?.0.get_mut(0)? {
        DataElement::Binary(b) => Some(b),
        _ => None,
    }
}

/// Calls `f` for every outer segment and every segment inside HNVSD.
fn for_each_segment(msg: &[u8], mut f: impl FnMut(&RawSegment)) {
    let Ok(mut outer) = wire::tokenize(msg) else {
        return;
    };
    for seg in &mut outer {
        if let Some(inner) = hnvsd_payload(seg)
            && let Ok(inner) = wire::tokenize(inner)
        {
            inner.iter().for_each(&mut f);
        }
        f(seg);
    }
}

/// Rewrites every segment (inside HNVSD first, then the outer ones).
fn map_message(msg: &[u8], mut f: impl FnMut(&mut RawSegment)) -> Option<Vec<u8>> {
    let mut outer = wire::tokenize(msg).ok()?;
    for seg in &mut outer {
        if let Some(payload) = hnvsd_payload(seg) {
            let mut inner = wire::tokenize(payload).ok()?;
            inner.iter_mut().for_each(&mut f);
            *payload = wire::serialize(&inner).ok()?;
        }
        f(seg);
    }
    wire::serialize(&outer).ok()
}

// ── Account aliases ─────────────────────────────────────────────────────

/// Segments whose data element 1 addresses an account (KTV or KTI).
const ACCOUNT_JOBS: [&str; 8] = [
    "HKSAL", "HKKAZ", "HKCAZ", "HKWPD", "HKWDU", "HKSPA", "HKCCS", "HKIPZ",
];

/// What identifies an account: the IBAN, or the national triple
/// number (leading zeros dropped), subaccount (empty if none), BLZ.
#[derive(Hash)]
enum IdKey<'a> {
    Iban(&'a str),
    National(&'a str, &'a str, &'a str),
}

impl<'a> IdKey<'a> {
    fn iban(iban: &'a str) -> Option<Self> {
        (!iban.is_empty() && !is_starred(iban)).then_some(IdKey::Iban(iban))
    }

    fn national(number: &'a str, sub: &'a str, blz: &'a str) -> Option<Self> {
        let number = number.trim_start_matches('0');
        (!number.is_empty() && !blz.is_empty() && !is_starred(number))
            .then_some(IdKey::National(number, sub, blz))
    }
}

/// The accounts of the UPDs a [`Recorder`] has seen, found by keyed hashes
/// of their identifiers. Holds no identifier.
#[derive(Default)]
struct AccountIndex {
    /// Random per recorder, never written: the hashes mean nothing outside.
    keys: RandomState,
    /// Hash of an [`IdKey`] → position in `accounts`.
    by_key: HashMap<u64, usize>,
    /// The accounts; alias `#n` is position + 1.
    accounts: Vec<TraceAccount>,
}

impl AccountIndex {
    fn hash(&self, key: &IdKey) -> u64 {
        self.keys.hash_one(key)
    }

    fn find(&self, keys: &[Option<IdKey>]) -> Option<usize> {
        keys.iter()
            .flatten()
            .find_map(|k| self.by_key.get(&self.hash(k)).copied())
    }

    fn keys(acc: &SepaAccount) -> [Option<IdKey<'_>>; 2] {
        [
            IdKey::iban(acc.iban.as_str()),
            IdKey::national(&acc.account_number, &acc.sub_account, acc.blz.as_str()),
        ]
    }

    /// Learns every HIUPD of an unmasked answer: a new account gets the
    /// next alias, a known one (same IBAN or national triple) keeps its own.
    fn learn(&mut self, answer: &[u8]) {
        let Some(segs) = all_segments(answer) else {
            return;
        };
        for seg in segs.iter().filter(|s| code(s) == "HIUPD") {
            let Some(acc) = crate::segments::read::upd_account(seg) else {
                continue;
            };
            let keys = Self::keys(&acc);
            if keys.iter().all(Option::is_none) {
                continue;
            }
            let at = self.find(&keys).unwrap_or(self.accounts.len());
            let account = trace_account(at + 1, &acc);
            if at == self.accounts.len() {
                self.accounts.push(account);
            } else {
                self.accounts[at] = account;
            }
            for key in keys.iter().flatten() {
                self.by_key.insert(self.hash(key), at);
            }
        }
    }

    /// The alias of the first account a job segment of an unmasked request
    /// addresses: data element 1, read as KTI (`IBAN:BIC[:number:sub:280:BLZ]`)
    /// or KTV (`number:sub:280:BLZ`).
    fn alias_of_request(&self, request: &[u8]) -> Option<String> {
        let segs = all_segments(request)?;
        segs.iter()
            .filter(|s| ACCOUNT_JOBS.contains(&code(s)))
            .find_map(|seg| {
                let g = |i| text(seg, 1, i).unwrap_or_default();
                self.find(&[
                    IdKey::iban(g(0)),
                    IdKey::national(g(2), g(3), g(5)),
                    IdKey::national(g(0), g(1), g(3)),
                ])
            })
            .map(|at| self.accounts[at].alias.clone())
    }

    fn account_of(&self, acc: &AccountInfo) -> Option<TraceAccount> {
        let blz = match &acc.key {
            crate::types::AccountKey::National { blz, .. } => blz.as_str(),
            crate::types::AccountKey::Iban { .. } => "",
        };
        let keys = [
            IdKey::iban(acc.iban.as_ref().map_or("", |i| i.as_str())),
            IdKey::national(
                acc.number.as_deref().unwrap_or_default(),
                acc.subaccount.as_deref().unwrap_or_default(),
                blz,
            ),
        ];
        self.find(&keys).map(|at| self.accounts[at].clone())
    }
}

/// The [`TraceAccount`] `#alias` of a UPD account: class, Kontoart, jobs.
fn trace_account(alias: usize, acc: &SepaAccount) -> TraceAccount {
    use crate::types::AccountClass::*;
    let class = match acc.classify().map(|(class, _)| class) {
        Some(Checking) => Some("checking"),
        Some(Savings) => Some("savings"),
        Some(FixedDeposit) => Some("fixed-deposit"),
        Some(Depot) => Some("depot"),
        Some(Loan) => Some("loan"),
        Some(CreditCard) => Some("credit-card"),
        Some(FundDepot) => Some("fund-depot"),
        Some(HomeSavings) => Some("home-savings"),
        Some(Insurance) => Some("insurance"),
        Some(Other) => Some("other"),
        None => None,
    };
    TraceAccount {
        alias: format!("#{alias}"),
        class: class.map(str::to_string),
        kontoart: acc.account_type,
        jobs: acc.allowed_jobs.clone(),
    }
}

// ── Recording ───────────────────────────────────────────────────────────

/// Records the exchanges of the transports it wraps. Cheap to clone; clones
/// share the log. Wrap a transport ([`Recorder::wrap`]), hand it to the
/// dialogs (`with_transport`), run the client, then take
/// [`Recorder::trace`].
#[derive(Clone)]
pub struct Recorder(Arc<Shared>);

/// A segment the recorder has no field list for, as the person recording
/// may look at it before deciding whether a trace shows it
/// ([`Recorder::unknown_segments`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSegment {
    /// The segment code, e.g. `HIXYZ`.
    pub code: String,
    pub version: u16,
    /// How many of them the dialog had.
    pub count: usize,
    /// The first one as sent, unmasked: it may hold personal data.
    /// ISO 8859-1 text; a byte that is not printable shows as `.`.
    pub example: String,
}

struct Shared {
    /// The exchanges, masked, with the unknown segments still as sent
    /// (`mask::mask_for_recorder`); they are starred when a trace is taken.
    log: Mutex<Vec<Exchange>>,
    accounts: Mutex<AccountIndex>,
    ids: Identities,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::with_salt(Salt::random())
    }
}

impl Recorder {
    /// An empty recorder with a fresh random [`Salt`] that is never
    /// written: its pseudonyms are consistent within its trace and link to
    /// no other trace. The default.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty recorder whose pseudonyms are keyed by `salt`: every trace
    /// recorded with the same salt gives a reference or ID the same
    /// pseudonym. Account identifiers still get aliases of this trace
    /// only; the salt does not reach them.
    pub fn with_salt(salt: Salt) -> Self {
        Recorder(Arc::new(Shared {
            log: Mutex::default(),
            accounts: Mutex::default(),
            ids: Identities::new(salt).with_days(Identities::random_days()),
        }))
    }

    /// A transport that sends through `inner` and records every exchange
    /// (redacted) in this recorder, in the order the sends finish.
    pub fn wrap(&self, inner: SharedTransport) -> SharedTransport {
        Arc::new(Recording {
            inner,
            log: self.clone(),
        })
    }

    /// The exchanges so far, redacted but not anonymised. Unknown segments
    /// show only their header and the length of each element.
    pub fn exchanges(&self) -> Vec<Exchange> {
        self.exchanges_keeping(&[])
    }

    fn exchanges_keeping(&self, keep: &[&str]) -> Vec<Exchange> {
        let keep = |code: &str| keep.contains(&code);
        self.raw_log()
            .into_iter()
            .map(|ex| Exchange {
                request: mask::hide_unknown(&ex.request, &keep),
                response: mask::hide_unknown(&ex.response, &keep),
                ..ex
            })
            .collect()
    }

    fn raw_log(&self) -> Vec<Exchange> {
        self.0.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The segments of the dialog so far that the recorder has no field
    /// list for, one entry per code and version. A trace shows only their
    /// header and element lengths, which is rarely enough to support a bank
    /// that sends them. Show the examples to the person recording; the
    /// codes they agree to go to [`Recorder::trace_keeping`].
    pub fn unknown_segments(&self) -> Vec<UnknownSegment> {
        let mut found: Vec<UnknownSegment> = Vec::new();
        for ex in self.raw_log() {
            for bytes in [&ex.request, &ex.response]
                .into_iter()
                .flat_map(|m| mask::unknown_segments(m))
            {
                let Some(seg) = wire::tokenize(&bytes).ok().and_then(|mut s| s.pop()) else {
                    continue;
                };
                let (code, version) = (code(&seg).to_string(), seg.segment_version());
                match found
                    .iter_mut()
                    .find(|u| u.code == code && u.version == version)
                {
                    Some(known) => known.count += 1,
                    None => found.push(UnknownSegment {
                        code,
                        version,
                        count: 1,
                        example: bytes
                            .iter()
                            .map(|&b| match b {
                                0x20..=0x7e | 0xa0..=0xff => char::from(b),
                                _ => '.',
                            })
                            .collect(),
                    }),
                }
            }
        }
        found
    }

    /// The accounts seen so far, in alias order.
    pub fn accounts(&self) -> Vec<TraceAccount> {
        self.index().accounts.clone()
    }

    /// The exchanges so far, anonymised ([`Trace::from_exchanges`]), with
    /// the accounts they name. Unknown segments show only their header and
    /// the length of each element.
    pub fn trace(&self) -> Trace {
        self.trace_keeping(&[])
    }

    /// [`Recorder::trace`] in which the unknown segments with these codes
    /// stay as the bank sent them, unmasked. Only for codes whose examples
    /// ([`Recorder::unknown_segments`]) a person has read.
    pub fn trace_keeping(&self, codes: &[&str]) -> Trace {
        Trace {
            accounts: self.accounts(),
            client: Client {
                version: env!("CARGO_PKG_VERSION").to_string(),
                outcome: None,
            },
            ..Trace::from_exchanges(self.exchanges_keeping(codes))
        }
    }

    /// The alias and description the recorder gives `account` (same IBAN
    /// or the same number, subaccount and BLZ as a HIUPD account it has
    /// seen), e.g. `account #2 (class checking; Kontoart 1; jobs HKSAL HKKAZ)`.
    pub fn account_of(&self, account: &AccountInfo) -> Option<TraceAccount> {
        self.index().account_of(account)
    }

    fn index(&self) -> std::sync::MutexGuard<'_, AccountIndex> {
        self.0.accounts.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Recording {
    inner: SharedTransport,
    log: Recorder,
}

/// Short, content-free summary of a failed send (an HTTP body is never
/// included: it may echo request data); a single line.
/// The kind of an error without its text: the variant, with the HTTP
/// status, the transport kind, the bank's code or the unreadable segment.
pub fn outcome_label(e: &FinTSError) -> String {
    let debug = format!("{e:?}");
    let variant: String = debug
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    match e {
        FinTSError::Http { status, .. } => format!("Http {status}"),
        FinTSError::Transport { kind, .. } => format!("Transport {kind}"),
        FinTSError::Bank(bank) => format!("Bank {}", bank.code().code),
        FinTSError::SegmentUnreadable(error) => format!("SegmentUnreadable {}", codec_note(error)),
        _ => variant,
    }
}

/// A codec error without the value: the kind and the field.
fn codec_note(error: &crate::codec::CodecError) -> String {
    let kind: String = format!("{:?}", error.kind)
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    format!("{kind} at {}", error.field)
}

/// What the client makes of an unmasked answer ([`Exchange::notes`]).
fn client_notes(answer: &[u8]) -> Vec<String> {
    let Some(segments) = all_segments(answer) else {
        return vec!["the answer does not tokenize".to_string()];
    };
    let mut notes = Vec::new();
    // SWIFT character set X: letters, digits, / - ? : ( ) . , ' + space.
    let swift = |b: u8| b.is_ascii_alphanumeric() || b"/-?:().,'+ \r\n".contains(&b);
    for seg in &segments {
        if !matches!(code(seg), "HIKAZ" | "HIWPD" | "HIWDU") {
            continue;
        }
        let odd = seg
            .degs
            .iter()
            .flat_map(|d| &d.0)
            .filter_map(DataElement::as_bytes)
            .flatten()
            .filter(|&&b| !swift(b))
            .count();
        if odd > 0 {
            notes.push(format!(
                "{}:{}: {odd} characters outside the SWIFT character set",
                code(seg),
                seg.segment_version()
            ));
        }
    }
    for parsed in crate::codec::parse_segments(segments) {
        if let crate::codec::ParsedSegment::Raw { segment, error } = parsed
            && error.kind != crate::codec::ErrorKind::UnknownSegment
        {
            notes.push(format!(
                "{}:{}: not read, {}",
                code(&segment),
                segment.segment_version(),
                codec_note(&error)
            ));
        }
    }
    notes
}

fn error_summary(e: &FinTSError) -> String {
    match e {
        FinTSError::Http { status, .. } => format!("HTTP {status}"),
        FinTSError::Transport { kind, message } => {
            format!("transport {kind}: {}", message.replace(['\r', '\n'], " "))
        }
        _ => "error".to_string(),
    }
}

impl Transport for Recording {
    fn send<'a>(&'a self, url: &'a str, message: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let ids = &self.log.0.ids;
            let request = mask::mask_for_recorder(message, ids);
            let result = self.inner.send(url, message).await;
            // The unmasked messages are read here and only hashed.
            let account = {
                let mut accounts = self.log.index();
                if let Ok(answer) = &result {
                    accounts.learn(answer);
                }
                accounts.alias_of_request(message)
            };
            let exchange = match &result {
                Ok(answer) => Exchange {
                    request,
                    response: mask::mask_for_recorder(answer, ids),
                    error: None,
                    account,
                    notes: client_notes(answer),
                },
                Err(e) => Exchange {
                    request,
                    response: Vec::new(),
                    error: Some(error_summary(e)),
                    account,
                    notes: Vec::new(),
                },
            };
            self.log
                .0
                .log
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(exchange);
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::TransportKind;

    fn wrap(inner: &str) -> Vec<u8> {
        format!(
            "HNHBK:1:3+0+300+D1+1'HNVSD:999:1+@{}@{inner}''HNHBS:9:1+1'",
            inner.len() + 1
        )
        .into_bytes()
    }

    fn exchange(request: Vec<u8>, response: Vec<u8>) -> Exchange {
        Exchange {
            request,
            response,
            error: None,
            account: None,
            notes: Vec::new(),
        }
    }

    #[test]
    fn summary_of_three_exchanges() {
        let ok = wrap("HIRMG:2:2+0010::Ok'HIRMS:3:2:3+0020::Done");
        let busy =
            wrap("HIRMG:2:2+9050::Teilweise fehlerhaft'HIRMS:3:2:3+9075::Freigabe in der App");
        let request = |job: &str| wrap(&format!("{job}:3:7+1'HKTAN:4:7+4+HKKAZ"));
        let trace = Trace {
            exchanges: vec![
                exchange(request("HKSAL"), ok),
                exchange(request("HKKAZ"), busy),
                Exchange {
                    request: b"garbage".to_vec(),
                    response: Vec::new(),
                    error: Some("HTTP 500".into()),
                    account: None,
                    notes: Vec::new(),
                },
            ],
            ..Trace::default()
        };
        let sum = trace.summary();
        assert_eq!(sum.len(), 3);
        assert_eq!(sum[0].jobs, ["HKSAL:7"]);
        assert_eq!(sum[0].tan, Some(("4".into(), "HKKAZ".into())));
        assert!(!sum[0].failed());
        assert_eq!(sum[1].segment[0].0, "HKKAZ");
        assert_eq!(sum[1].segment[0].1.code, "9075");
        assert!(sum[1].failed());
        let line = sum[1].to_string();
        assert!(
            line.starts_with("#01  HKKAZ:7 +HKTAN(4,HKKAZ)  | 9050 Teilweise"),
            "{line}"
        );
        assert!(
            line.ends_with("HKKAZ 9075 Freigabe in der App  FAILED"),
            "{line}"
        );
        assert!(!sum[0].to_string().contains("Done"));
        assert!(sum[2].failed());
        assert_eq!(sum[2].jobs, ["(unreadable)"]);
        assert!(sum[2].to_string().contains("| HTTP 500  FAILED"));
        assert!(sum.iter().all(|s| s.retry_of.is_none()));
    }

    /// HKKAZ without HKTAN, answered 9075; the same HKKAZ with HKTAN.
    fn sca_retry() -> Trace {
        let sca = wrap("HIRMG:2:2+9050::Teilweise fehlerhaft'HIRMS:3:2:3+9075::SCA erforderlich");
        let ok = wrap("HIRMG:2:2+0010::Ok'HIRMS:3:2:3+0020::Ok'HIRMS:4:2:4+0030::Freigabe");
        let mut first = exchange(wrap("HKKAZ:3:7+1+N"), sca);
        first.account = Some("#1".into());
        let mut retry = exchange(wrap("HKKAZ:3:7+1+N'HKTAN:4:7+4+HKKAZ"), ok);
        retry.account = Some("#1".into());
        Trace {
            comment: "Synthetic SCA retry.".into(),
            client: Client::default(),
            accounts: vec![TraceAccount {
                alias: "#1".into(),
                class: Some("checking".into()),
                kontoart: Some(1),
                jobs: vec!["HKKAZ".into()],
            }],
            exchanges: vec![first, retry.clone(), retry],
        }
    }

    #[test]
    fn a_resend_with_hktan_after_9075_is_a_retry() {
        let trace = sca_retry();
        let sum = trace.summary();
        assert_eq!(sum[1].retry_of, Some(0));
        assert_eq!(sum[2].retry_of, None, "the answer before was not 9075");
        let json = trace.to_json();
        assert!(json.contains("\"retry_of\": 0"), "{json}");
        assert!(
            sum[1].to_string().ends_with("  retry of #00  account #1"),
            "{}",
            sum[1]
        );
        let table = trace.summary_table();
        let mut lines = table.lines();
        assert_eq!(lines.next(), Some(SUMMARY_LEGEND));
        assert_eq!(
            lines.next(),
            Some("account #1 (class checking; Kontoart 1; jobs HKKAZ)")
        );
        assert!(lines.next().unwrap().starts_with("#00  HKKAZ:7  | 9050"));
    }

    const REQUEST: &[u8] =
        b"HNHBK:1:3+000000000060+300+0+1'HKIDN:2:2+280:12345678+****+0+1'HNHBS:3:1+1'";

    #[test]
    fn json_round_trips() {
        let mut trace = sca_retry();
        trace.comment = "line one\nline two".into();
        trace.exchanges.push(Exchange {
            request: REQUEST.to_vec(),
            response: Vec::new(),
            error: Some("transport: timed out".into()),
            account: None,
            notes: Vec::new(),
        });
        trace.exchanges.push(exchange(
            REQUEST.to_vec(),
            b"HNHBK:1:3+0+300+0+1'HIRMS:2:2+0010::Gr\xfc\\e\r\n\"'".to_vec(),
        ));
        let json = trace.to_json();
        assert!(
            json.starts_with("{\n  \"format\": \"fints-client-trace/4\""),
            "{json}"
        );
        assert!(json.contains("\"blz\": \"12345678\""), "{json}");
        assert!(json.contains("\"for\": \"HKKAZ\""), "{json}");
        assert!(json.contains("\"job\": \"HKKAZ\""), "{json}");
        assert!(json.contains("Grü\\\\e\\r\\n\\\"'"), "{json}");
        assert_eq!(Trace::from_json(&json).unwrap(), trace);
        assert_eq!(Trace::from_json(&json).unwrap().to_json(), json);
    }

    #[test]
    fn a_hand_edited_derived_field_is_rejected() {
        let json = sca_retry().to_json();
        let cases = [
            (
                "\"HKKAZ:7\"",
                "\"HKKAZ:6\"",
                "exchange 0: derived field sent.jobs[0]",
            ),
            (
                "\"retry_of\": 0",
                "\"retry_of\": null",
                "exchange 1: derived field retry_of",
            ),
            (
                "\"code\": \"9075\"",
                "\"code\": \"0020\"",
                "exchange 0: derived field answer.segments[0].code",
            ),
            (
                "\"failed\": true",
                "\"failed\": false",
                "exchange 0: derived field failed",
            ),
            (
                "\"index\": 2",
                "\"index\": 7",
                "exchange 2: derived field index",
            ),
            (
                "\"blz\": null",
                "\"blz\": \"99999999\"",
                "derived field bank.blz",
            ),
        ];
        for (from, to, error) in cases {
            assert!(json.contains(from), "{from} not in {json}");
            let edited = json.replacen(from, to, 1);
            let e = Trace::from_json(&edited).unwrap_err();
            assert!(e.starts_with(error), "{from} -> {to}: {e}");
        }
        let unknown = json.replacen("\"account\": \"#1\"", "\"account\": \"#9\"", 1);
        assert_eq!(
            Trace::from_json(&unknown).unwrap_err(),
            "exchange 0: account \"#9\" is not in `accounts`"
        );
        let other = json.replace(TRACE_FORMAT, "fints-client-trace/2");
        assert!(Trace::from_json(&other).unwrap_err().starts_with("format"));
    }

    #[test]
    fn a_message_splits_into_one_string_per_segment() {
        // Inside HNVSD: an escaped `'` and `?`, binary data holding `'` and
        // `@`, and ISO 8859-1 letters (ü, ß as single bytes).
        let inner: &[u8] = b"HKIDN:2:2+280:12345678+a?'b??+0+1'HKXYZ:3:1+@6@x'y@z'+M\xfc\xdfe'";
        let mut msg = format!("HNHBK:1:3+0+300+0+1'HNVSD:999:1+@{}@", inner.len()).into_bytes();
        msg.extend_from_slice(inner);
        msg.extend_from_slice(b"'HNHBS:3:1+1'");
        let parts = split_segments(&msg);
        assert_eq!(
            parts,
            [
                "HNHBK:1:3+0+300+0+1'",
                "HNVSD:999:1+@59@",
                "HKIDN:2:2+280:12345678+a?'b??+0+1'",
                "HKXYZ:3:1+@6@x'y@z'+Müße'",
                "'",
                "HNHBS:3:1+1'",
            ]
        );
        assert_eq!(join_segments(&parts).unwrap(), msg);
        let trace = Trace {
            exchanges: vec![exchange(msg.clone(), b"HNHBK:1:3+0+300+0+1".to_vec())],
            ..Trace::default()
        };
        assert_eq!(Trace::from_json(&trace.to_json()).unwrap(), trace);
        assert_eq!(split_segments(b"no end"), ["no end"]);
        assert_eq!(split_segments(b"X:1:1+@99@short"), ["X:1:1+@99@short"]);
        assert!(
            join_segments(&["€".into()])
                .unwrap_err()
                .contains("ISO 8859-1")
        );
    }

    #[test]
    fn anonymise_renames_dialogs_and_tasks() {
        let trace = Trace::from_exchanges(vec![exchange(
            b"HNHBK:1:3+000000000010+300+0+1'HKVVB:2:3+0+0+0+REALPRODUCT+1.0'".to_vec(),
            b"HNHBK:1:3+000000000010+300+abc123+1+0:1'HITAN:2:7+4++tsk-9+****'HISAL:3:7+***+####+EUR+C:####,##:EUR:20260101'"
                .to_vec(),
        )]);
        let text = trace.to_json();
        assert!(
            !text.contains("abc123") && text.contains("DIALOG0001"),
            "{text}"
        );
        assert!(
            !text.contains("tsk-9") && text.contains("TASK0001"),
            "{text}"
        );
        assert!(
            !text.contains("REALPRODUCT") && text.contains("***********"),
            "{text}"
        );
        assert!(text.contains("C:####,##:EUR"), "shapes stay: {text}");
    }

    struct Fixed(std::result::Result<Vec<u8>, &'static str>);

    impl Transport for Fixed {
        fn send<'a>(&'a self, _: &'a str, _: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>>> {
            Box::pin(async move {
                self.0
                    .clone()
                    .map_err(|e| FinTSError::transport(TransportKind::Io, e.to_string()))
            })
        }
    }

    #[tokio::test]
    async fn no_product_name_purpose_amount_or_account_survives() {
        let mt940 = ":61:260102C12,34NTRFNONREF\r\n:86:166?00Gutschrift?20SVWZ+Geheimzweck\r\n";
        let camt =
            "<Ntry><Amt Ccy=\"EUR\">98765.43</Amt><AddtlNtryInf>Geheimzweck</AddtlNtryInf></Ntry>";
        let response = format!(
            "HNHBK:1:3+0+300+D1+1'\
             HIUPA:2:4:4+secretuser+3+0+Erika Muster+Geheime Erweiterung'\
             HIUPD:3:6:4+4711:+DE02120300000000004711+secretuser+1+EUR+Erika Muster++Spargiro Spezial+\
             :5000,:EUR+HKSAL:1+HKKAZ:1+Geheime Kontoerweiterung'\
             HISAL:4:7:3+DE02120300000000004711::280:12030000+Spargiro Spezial+EUR+C:1234,56:EUR:20260102+\
             +5000,:EUR+777,77:EUR'\
             HIKAZ:5:7:3+@{}@{mt940}'\
             HICAZ:6:1:3+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+@{}@{camt}'",
            mt940.len(),
            camt.len(),
        );
        let rec = Recorder::new();
        let bank = rec.wrap(Arc::new(Fixed(Ok(response.into_bytes()))));
        let request = b"HNHBK:1:3+0+300+0+1'HKIDN:2:2+280:12030000+secretuser+0+1'";
        bank.send("http://x/", request).await.unwrap();
        let text = rec.trace().to_json();
        for secret in [
            "Spargiro",
            "Geheim",
            "Erika",
            "Muster",
            "4711",
            "1234,56",
            "777,77",
            "5000,",
            "12,34",
            "98765",
            "secretuser",
        ] {
            assert!(!text.contains(secret), "{secret:?} survives:\n{text}");
        }
        assert!(text.contains("12030000"), "the BLZ stays");
        // Statements keep their shape: markup and codes stay.
        for shape in [
            "C#,##NTRFNONREF\\r\\n:86:166?00########?20SVWZ+########",
            "<Amt Ccy=\\\"EUR\\\">#.##</Amt><AddtlNtryInf>########</AddtlNtryInf>",
            "+########+EUR+C:#,##:EUR:",
            "++#,##:EUR+#,##:EUR'",
        ] {
            assert!(text.contains(shape), "{shape:?} not in:\n{text}");
        }
        // Every date is moved by the days of this recorder (3 to 30 back).
        for date in [":61:260102", "20260102"] {
            assert!(!text.contains(date), "{date:?} was not moved:\n{text}");
        }
        assert_eq!(
            unshape_statement("HIKAZ", b":61:260102C##,##NTRFNONREF\r\n:86:166?00####").unwrap(),
            b":61:260102C11,11NTRFNONREF\r\n:86:166?00XXXX"
        );
        assert_eq!(unshape_statement("HISAL", b"#"), None);
    }

    /// Answers with the next message of a script.
    struct Script(Mutex<std::collections::VecDeque<Vec<u8>>>);

    impl Transport for Script {
        fn send<'a>(&'a self, _: &'a str, _: &'a [u8]) -> BoxFuture<'a, Result<Vec<u8>>> {
            Box::pin(async move {
                let next = self.0.lock().unwrap().pop_front();
                next.ok_or_else(|| FinTSError::transport(TransportKind::Io, "end of script"))
            })
        }
    }

    const UPD: &str = "HNHBK:1:3+0+300+D1+1'\
        HIUPD:2:6:4+0009876543::280:12345678+DE02123456780009876543+user1+1+EUR+Erika Muster++Girokonto Plus++HKSAL:1+HKKAZ:1'\
        HIUPD:3:6:4+7000999::280:12345678++user1++EUR+Erika Muster++Mein Depot++HKWPD:1'\
        HIUPD:4:4:4+5550001::280:12345678+user1+EUR+Erika Muster++Sparbuch Extra++HKSAL:1'";

    #[tokio::test]
    async fn recorder_names_accounts_by_alias_only() {
        let ok = b"HNHBK:1:3+0+300+D1+2'HIRMG:2:2+0010::ok'".to_vec();
        let answers = [
            UPD.as_bytes().to_vec(),
            ok.clone(),
            ok.clone(),
            ok.clone(),
            UPD.as_bytes().to_vec(),
            ok,
        ];
        let rec = Recorder::new();
        let bank = rec.wrap(Arc::new(Script(Mutex::new(answers.into()))));
        let requests: [&[u8]; 6] = [
            b"HNHBK:1:3+0+300+0+1'HKIDN:2:2+280:12345678+user1+0+1'",
            // KTI with the national parts (HISPAS allows them).
            b"HNHBK:1:3+0+300+D1+2'HKSAL:3:7+DE02123456780009876543:TESTDEFFXXX:0009876543::280:12345678+N'HKTAN:4:7+4+HKSAL'",
            // KTV, account number with leading zeros the UPD did not have.
            b"HNHBK:1:3+0+300+D1+3'HKWPD:3:6+0007000999::280:12345678+EUR'",
            b"HNHBK:1:3+0+300+D1+4'HKKAZ:3:5+5550001::280:12345678+N'",
            b"HNHBK:1:3+0+300+0+1'HKIDN:2:2+280:12345678+user1+0+1'",
            b"HNHBK:1:3+0+300+D2+2'HKSAL:3:6+0009876543::280:12345678+N'",
        ];
        for request in requests {
            bank.send("http://x/", request).await.unwrap();
        }
        let aliases: Vec<_> = rec.exchanges().into_iter().map(|e| e.account).collect();
        let alias = |a: &str| Some(a.to_string());
        assert_eq!(
            aliases,
            // A second UPD keeps the aliases.
            [
                None,
                alias("#1"),
                alias("#2"),
                alias("#3"),
                None,
                alias("#1")
            ]
        );
        let trace = rec.trace();
        let described: Vec<String> = trace.accounts.iter().map(|a| a.to_string()).collect();
        assert_eq!(
            described,
            [
                "account #1 (class checking; Kontoart 1; jobs HKSAL HKKAZ)",
                "account #2 (class depot; Kontoart -; jobs HKWPD)",
                "account #3 (class -; Kontoart -; jobs HKSAL)",
            ]
        );
        let upd = crate::protocol::Upd::from_segments(
            &wire::tokenize(
                b"HIUPA:2:4:4+user1+1+0'HIUPD:3:6:4+7000999::280:12345678++user1++EUR+X++Y++HKWPD:1'",
            )
            .unwrap(),
        )
        .unwrap();
        let depot = AccountInfo::from_upd(&upd, &crate::types::Bic::new("TESTDEFFXXX"));
        assert_eq!(rec.account_of(&depot[0]).as_ref(), trace.account("#2"));
        let text = trace.to_json();
        assert!(text.contains("\"account\": \"#1\""), "{text}");
        assert!(text.contains("\"bic\": \"TESTDEFFXXX\""), "{text}");
        for secret in [
            "DE02123456780009876543",
            "9876543",
            "7000999",
            "5550001",
            "user1",
            "Erika",
            "Girokonto",
            "Depot ",
            "Mein",
            "Sparbuch",
        ] {
            assert!(!text.contains(secret), "{secret:?} survives:\n{text}");
        }
        assert_eq!(Trace::from_json(&text).unwrap(), trace);
        let summary = trace.summary();
        assert!(
            summary[2].to_string().ends_with("  account #2"),
            "{}",
            summary[2]
        );
    }

    #[tokio::test]
    async fn recorder_redacts_and_records_failures() {
        let rec = Recorder::new();
        let ok = rec.wrap(Arc::new(Fixed(Ok(
            b"HNHBK:1:3+0+300+D1+1'HIRMS:2:2+0010::ok'".to_vec(),
        ))));
        let bad = rec.wrap(Arc::new(Fixed(Err("connection refused"))));
        let request = b"HNHBK:1:3+0+300+0+1'HKIDN:2:2+280:12345678+secretuser+0+1'";
        assert!(ok.send("http://x/", request).await.is_ok());
        assert!(bad.send("http://x/", request).await.is_err());
        let ex = rec.exchanges();
        assert_eq!(ex.len(), 2);
        assert!(!String::from_utf8_lossy(&ex[0].request).contains("secretuser"));
        assert!(!ex[0].response.is_empty() && ex[0].error.is_none());
        assert!(ex[1].response.is_empty());
        assert_eq!(
            ex[1].error.as_deref(),
            Some("transport io: connection refused")
        );
    }
}
