//! Replay of a recorded dialog: the bank side of a masked trace is played
//! back to the client, and every request the client sends is compared with
//! the recorded one.
//!
//! The pieces:
//!
//! - [`Trace`] (`fints_client::trace`, feature `trace`) is the fixture: a
//!   JSON document (`Trace::from_json`), recorded with
//!   `fints_client::trace::Recorder` (a transport decorator).
//! - [`Replay`] serves a trace over loopback HTTP. The recorded responses
//!   are re-filled with synthetic values where they were starred and made
//!   valid where they were shape-masked (see `refill`), and each request is checked against the recorded one
//!   (see `compare`); differences are [`Violation`]s. A recorded failed
//!   exchange (`error` set) is answered with HTTP 500 and its error text.
//! - [`standard_run`] is the one client run that is recorded and replayed:
//!   login (with its TAN), balance and transactions for every account,
//!   dialog end.
//!
//! Clean room: written from the FinTS 3.0 specification (Formals, PIN/TAN)
//! and our own masking rules (`fints_client::mask`).

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fints_client::codec::{DataElement, RawSegment, wire};
use fints_client::jobs::{
    self, BankClient, Completed, Credentials, Dialog, Format, JobKind, JobValue, Pending, Range,
    Statement, Step, Support, TanChallenge, TanKind,
};
use fints_client::trace::{Trace, unshape_statement, unshape_text};
use fints_client::{
    Account, AccountBalance, BankConfig, FinTSError, Notice, Pin, ProductId, SecurityHolding,
    SessionState, Transaction, UserId,
};
use tokio::task::JoinHandle;

use crate::http::{Responder, listen};
use crate::model::iban_de;
use crate::request::Violation;

/// Synthetic values the replay puts where the trace has stars.
const SYSTEM_ID: &str = "REPLAYSYS0001";
const DEFAULT_USER_ID: &str = "replayuser1";
const CHALLENGE: &str = "Bitte Auftrag in der Test-App freigeben";
const DEFAULT_BLZ: &str = "12345678";

// ── Segments ────────────────────────────────────────────────────────────

fn code(seg: &RawSegment) -> &str {
    seg.segment_type()
}

fn header(seg: &RawSegment, i: usize) -> &str {
    match seg.degs.first().and_then(|d| d.0.get(i)) {
        Some(DataElement::Text(s)) => s,
        _ => "",
    }
}

fn text(seg: &RawSegment, de: usize, ge: usize) -> Option<&str> {
    match seg.degs.get(de)?.0.get(ge)? {
        DataElement::Text(s) => Some(s),
        _ => None,
    }
}

fn set(seg: &mut RawSegment, de: usize, ge: usize, value: DataElement) {
    while seg.degs.len() <= de {
        seg.degs.push(fints_client::codec::DEG(Vec::new()));
    }
    let group = &mut seg.degs[de].0;
    while group.len() <= ge {
        group.push(DataElement::Empty);
    }
    group[ge] = value;
}

fn is_starred(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b == b'*')
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

/// All segments of a message, outer and inner, in wire order (the inner
/// segments right after HNVSD).
fn segments(msg: &[u8]) -> Result<Vec<RawSegment>, String> {
    let outer = wire::tokenize(msg).map_err(|e| format!("does not tokenize: {e}"))?;
    let mut all = Vec::new();
    for mut seg in outer {
        let inner = match hnvsd_payload(&mut seg) {
            Some(payload) => wire::tokenize(payload)
                .map_err(|e| format!("HNVSD content does not tokenize: {e}"))?,
            None => Vec::new(),
        };
        all.push(seg);
        all.extend(inner);
    }
    Ok(all)
}

// ── Comparing a request with the recorded one ───────────────────────────

/// What is compared of a segment: its code, version and number, the
/// number of data elements and of group elements in each, and for HKTAN
/// the TAN process and the segment it refers to. Values are not compared:
/// in the trace most are stars, the rest (dates, references, dialog IDs,
/// message numbers, sizes) change from run to run.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    code: String,
    number: String,
    version: String,
    groups: Vec<usize>,
    keys: Vec<(&'static str, String)>,
}

impl Shape {
    fn of(seg: &RawSegment) -> Shape {
        let code = code(seg).to_string();
        let keys = if code == "HKTAN" {
            vec![
                ("process", text(seg, 1, 0).unwrap_or("").to_string()),
                ("segment code", text(seg, 2, 0).unwrap_or("").to_string()),
            ]
        } else {
            Vec::new()
        };
        Shape {
            number: header(seg, 1).to_string(),
            version: header(seg, 2).to_string(),
            groups: seg.degs.iter().skip(1).map(|d| d.0.len()).collect(),
            keys,
            code,
        }
    }

    fn label(&self) -> String {
        format!("{}:{}", self.code, self.number)
    }
}

fn shapes(msg: &[u8]) -> Result<Vec<Shape>, String> {
    Ok(segments(msg)?.iter().map(Shape::of).collect())
}

/// The differences between the recorded request and the one sent.
fn compare(recorded: &[Shape], sent: &[Shape]) -> Vec<String> {
    fn codes(s: &[Shape]) -> Vec<&str> {
        s.iter().map(|s| s.code.as_str()).collect()
    }
    if codes(recorded) != codes(sent) {
        let list = |s: &[Shape]| {
            s.iter()
                .map(|s| format!("{}:{}", s.label(), s.version))
                .collect::<Vec<_>>()
                .join(" ")
        };
        return vec![format!(
            "segments differ: recorded [{}], sent [{}]",
            list(recorded),
            list(sent)
        )];
    }
    let mut out = Vec::new();
    for (r, s) in recorded.iter().zip(sent) {
        let at = r.label();
        if r.version != s.version {
            out.push(format!(
                "{at}: version {} recorded, {} sent",
                r.version, s.version
            ));
            continue;
        }
        if r.groups.len() != s.groups.len() {
            out.push(format!(
                "{at}: {} data elements recorded, {} sent",
                r.groups.len(),
                s.groups.len()
            ));
        } else {
            for (i, (a, b)) in r.groups.iter().zip(&s.groups).enumerate() {
                if a != b {
                    out.push(format!(
                        "{at} data element {}: {a} group elements recorded, {b} sent",
                        i + 1
                    ));
                }
            }
        }
        for ((name, a), (_, b)) in r.keys.iter().zip(&s.keys) {
            if a != b {
                out.push(format!("{at} {name}: {a:?} recorded, {b:?} sent"));
            }
        }
    }
    out
}

// ── Re-filling a recorded response ──────────────────────────────────────

/// What the replay knows of the client so far.
struct Client {
    user_id: String,
}

/// The recorded response, made valid again for the request just received:
///
/// - the envelope (HNHBK size, message number and reference, HNHBS) is
///   computed from the request; the dialog ID stays as recorded;
/// - a value that is all stars is filled: in the account answers (HISAL,
///   HIKAZ, HICAZ, HIWPD, HIWDU) and HNVSK from the same position of the
///   client's HKSAL, HKKAZ, … or HNVSK; system ID (HISYN), user ID (HIUPA,
///   HIUPD), accounts (HIUPD, numbered per segment) and the challenge
///   (HITAN) with synthetic values;
/// - a shape-masked value is made valid again (`fints_client::trace::
///   unshape_text`, `unshape_statement`): names, product names and amounts
///   of HIUPA, HIUPD and HISAL, and the recorded statements of HIKAZ
///   (MT940), HICAZ (camt), HIWPD and HIWDU (MT535/536), each `#` a `1` in
///   amounts and an `X` in text, so the client parses the bank's own layout;
/// - any other starred value is left out (empty).
fn refill(recorded: &[u8], request: &[u8], client: &Client) -> Result<Vec<u8>, String> {
    let sent = segments(request).map_err(|e| format!("request {e}"))?;
    let hnhbk = sent
        .iter()
        .find(|s| code(s) == "HNHBK")
        .ok_or("request without HNHBK")?;
    let dialog_id = text(hnhbk, 3, 0).unwrap_or("0").to_string();
    let message_number = text(hnhbk, 4, 0).unwrap_or("1").to_string();

    let mut accounts = 0usize;
    let mut fill = |seg: &mut RawSegment| {
        if code(seg) == "HIUPD" {
            accounts += 1;
        }
        fill_segment(seg, &sent, client, accounts.saturating_sub(1));
        match code(seg) {
            "HNHBK" => {
                set(seg, 4, 0, DataElement::Text(message_number.clone()));
                set(seg, 5, 0, DataElement::Text(dialog_id.clone()));
                set(seg, 5, 1, DataElement::Text(message_number.clone()));
            }
            "HNHBS" => set(seg, 1, 0, DataElement::Text(message_number.clone())),
            _ => {}
        }
    };
    let filled = map_message(recorded, &mut fill).ok_or("recorded response does not tokenize")?;

    // The size has a fixed width of 12 digits: measure, then write.
    let mut outer = wire::tokenize(&filled).map_err(|e| e.to_string())?;
    let first = outer.first_mut().ok_or("recorded response is empty")?;
    if code(first) != "HNHBK" {
        return Err("recorded response does not start with HNHBK".into());
    }
    set(first, 1, 0, DataElement::Text("0".repeat(12)));
    let size = wire::serialize(&outer).map_err(|e| e.to_string())?.len();
    set(
        &mut outer[0],
        1,
        0,
        DataElement::Text(format!("{size:012}")),
    );
    wire::serialize(&outer).map_err(|e| e.to_string())
}

fn fill_segment(seg: &mut RawSegment, sent: &[RawSegment], client: &Client, account: usize) {
    let code = code(seg).to_string();
    let echo_from = match code.as_str() {
        "HISAL" | "HIKAZ" | "HICAZ" | "HIWPD" | "HIWDU" => Some(format!("HK{}", &code[2..])),
        "HNVSK" | "HNSHK" => Some(code.clone()),
        _ => None,
    };
    let echo = echo_from.and_then(|c| sent.iter().find(|s| self::code(s) == c));
    let blz = text(seg, 1, 3)
        .filter(|b| !is_starred(b) && !b.is_empty())
        .unwrap_or(DEFAULT_BLZ)
        .to_string();
    let number = format!("{:010}", 1_234_567 + account);
    for de in 1..seg.degs.len() {
        for ge in 0..seg.degs[de].0.len() {
            let binary = match &seg.degs[de].0[ge] {
                DataElement::Text(s)
                    if matches!(code.as_str(), "HIUPA" | "HIUPD" | "HISAL") && s.contains('#') =>
                {
                    seg.degs[de].0[ge] = DataElement::Text(unshape_text(s));
                    continue;
                }
                DataElement::Binary(b) if b.contains(&b'#') => {
                    if let Some(data) = unshape_statement(&code, b) {
                        seg.degs[de].0[ge] = DataElement::Binary(data);
                    }
                    continue;
                }
                DataElement::Text(s) if is_starred(s) => false,
                DataElement::Binary(b) if !b.is_empty() && b.iter().all(|&c| c == b'*') => true,
                _ => continue,
            };
            let echoed = echo
                .filter(|_| !(code == "HISAL" && de == 2))
                .and_then(|s| text(s, de, ge))
                .filter(|v| !v.is_empty() && !is_starred(v) && !binary);
            let value = if let Some(v) = echoed {
                DataElement::Text(v.to_string())
            } else {
                synthetic(&code, de, ge, binary, client, &blz, &number)
            };
            seg.degs[de].0[ge] = value;
        }
    }
}

fn synthetic(
    code: &str,
    de: usize,
    ge: usize,
    binary: bool,
    client: &Client,
    blz: &str,
    number: &str,
) -> DataElement {
    let t = |s: &str| DataElement::Text(s.to_string());
    match (code, de, ge, binary) {
        ("HISYN", 1, 0, false) => t(SYSTEM_ID),
        ("HIUPA", 1, 0, false) | ("HIUPD", 3, 0, false) => t(&client.user_id),
        ("HNVSK" | "HNSHK", _, _, false) => t(&client.user_id),
        ("HIUPD", 1, 0, false) => t(number),
        ("HIUPD", 2, 0, false) => t(&iban_de(blz, number)),
        ("HITAN", 4, 0, false) => t(CHALLENGE),
        _ => DataElement::Empty,
    }
}

/// The local time a trace was recorded: the signature stamp
/// (`YYYYMMDD:HHMMSS`) of the first HNSHK in its requests.
pub fn recording_time(trace: &Trace) -> Option<jiff::civil::DateTime> {
    trace.exchanges.iter().find_map(|ex| {
        let at = ex.request.windows(6).position(|w| w == b"HNSHK:")?;
        let rest = &ex.request[at..];
        let stamp = rest.windows(16).find(|w| {
            w[0] == b':' && w[9] == b':' && w[1..9].iter().chain(&w[10..16]).all(u8::is_ascii_digit)
        })?;
        let text = std::str::from_utf8(&stamp[1..]).ok()?;
        jiff::civil::DateTime::strptime("%Y%m%d:%H%M%S", text).ok()
    })
}

// ── Serving ─────────────────────────────────────────────────────────────

struct State {
    trace: Trace,
    next: usize,
    violations: Vec<Violation>,
    client: Client,
}

impl State {
    fn respond(&mut self, message: &[u8]) -> Result<Vec<u8>, String> {
        let index = self.next;
        self.next += 1;
        let violate = |state: &mut State, message: String| {
            state.violations.push(Violation {
                exchange: index,
                message: message.clone(),
            });
            message
        };
        let Some(ex) = self.trace.exchanges.get(index).cloned() else {
            let m = format!(
                "the trace has {} exchanges; the client sent one more",
                self.trace.exchanges.len()
            );
            return Err(violate(self, m));
        };
        match (shapes(&ex.request), shapes(message)) {
            (Ok(recorded), Ok(sent)) => {
                for m in compare(&recorded, &sent) {
                    violate(self, m);
                }
            }
            (Err(e), _) => {
                violate(self, format!("recorded request {e}"));
            }
            (_, Err(e)) => return Err(violate(self, format!("request {e}"))),
        }
        for_each_segment(message, |seg| {
            if code(seg) == "HKIDN"
                && let Some(user) = text(seg, 2, 0).filter(|u| !u.is_empty())
            {
                self.client.user_id = user.to_string();
            }
        });
        if let Some(error) = ex.error {
            // A recorded failed exchange: HTTP 500 carrying the error text.
            return Err(error);
        }
        refill(&ex.response, message, &self.client).map_err(|e| violate(self, e))
    }
}

/// A [`Trace`] to serve: answers in the recorded order, checks every
/// request against the recorded one.
pub struct Replay {
    trace: Trace,
}

impl Replay {
    /// A replay of this trace.
    pub fn new(trace: Trace) -> Self {
        Self { trace }
    }

    /// Serves the replay on 127.0.0.1 (free port).
    pub async fn serve(self) -> HttpReplay {
        let blz = self.trace.blz().unwrap_or_else(|| DEFAULT_BLZ.into());
        let state = Arc::new(Mutex::new(State {
            trace: self.trace,
            next: 0,
            violations: Vec::new(),
            client: Client {
                user_id: DEFAULT_USER_ID.into(),
            },
        }));
        let (url, task) = listen(ReplayResponder(Arc::clone(&state))).await;
        HttpReplay {
            url,
            blz,
            state,
            task,
        }
    }
}

struct ReplayResponder(Arc<Mutex<State>>);

impl Responder for ReplayResponder {
    fn respond(&mut self, message: &[u8], _: Option<String>) -> Result<Vec<u8>, String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .respond(message)
    }
}

/// A served [`Replay`]; the server stops when this is dropped.
pub struct HttpReplay {
    url: String,
    blz: String,
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}

impl HttpReplay {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The FinTS URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// A `BankConfig` for the replay: the recorded bank code with its
    /// name, BIC and profile from the registry, this URL. A code outside
    /// the registry gets synthetic data and the generic profile.
    pub fn bank_config(&self) -> BankConfig {
        match fints_client::bank_by_blz(&self.blz) {
            Some(b) => b.with_url(self.url.clone()),
            None => BankConfig::new("Replay", self.blz.clone(), "TESTDEFFXXX", self.url.clone()),
        }
    }

    /// A client for [`HttpReplay::bank_config`].
    ///
    /// "Now" of its dialogs is the time the trace was recorded (the stamp
    /// of the first HNSHK), so the replay does not depend on the real clock.
    pub fn client(&self, product: &ProductId) -> BankClient {
        let client = BankClient::new(self.bank_config(), product.clone());
        match recording_time(&self.lock().trace) {
            Some(at) => client.with_now(at),
            None => client,
        }
    }

    /// Requests answered so far.
    pub fn served(&self) -> usize {
        self.lock().next
    }

    /// Every difference so far, plus one when the client has not sent all
    /// recorded requests.
    pub fn violations(&self) -> Vec<Violation> {
        let state = self.lock();
        let mut v = state.violations.clone();
        let total = state.trace.exchanges.len();
        if state.next < total {
            v.push(Violation {
                exchange: state.next,
                message: format!(
                    "the client stopped after {} of {total} recorded exchanges",
                    state.next
                ),
            });
        }
        v
    }

    /// Panics if a request differed from the recording or the client
    /// stopped early.
    pub fn assert_clean(&self) {
        let v = self.violations();
        assert!(
            v.is_empty(),
            "the client's requests differ from the recording:\n{}",
            v.iter().map(|v| format!("  {v}\n")).collect::<String>()
        );
    }
}

impl Drop for HttpReplay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// ── The recorded run ────────────────────────────────────────────────────

/// What a TAN step reports to [`RunOptions::on_event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    /// A decoupled TAN is waiting for the user; `challenge` is the bank's
    /// text to show.
    AwaitingConfirmation { challenge: String },
    /// The bank answered a status request with "not yet" (`poll`, 1-based).
    StillPending { poll: u32 },
    /// The confirmation went through.
    Confirmed,
}

/// How [`standard_run`] handles TANs and how far back it fetches.
#[derive(Clone)]
pub struct RunOptions {
    /// Days of transactions to fetch.
    pub days: u32,
    /// Use the bank's HITANS waits (live runs): sleep until the
    /// challenge's `not_before`. `false` ignores them and always waits
    /// `first_wait`/`next_wait`: replays and tests must not sleep the real
    /// bank's times.
    pub bank_waits: bool,
    /// Wait before the first status request (when the bank gives no value
    /// or `bank_waits` is off).
    pub first_wait: Duration,
    /// Wait between status requests (same rule as `first_wait`).
    pub next_wait: Duration,
    /// Give up on one decoupled TAN after this long.
    pub timeout: Duration,
    /// Told about each decoupled step (e.g. to print "confirm in the app").
    pub on_event: fn(RunEvent),
    /// Called before each status request of a bank that does not allow
    /// automatic polling (`auto_polling_allowed == Some(false)`): block
    /// until the user asks for the status request (e.g. presses Enter).
    pub wait_for_user: fn(&TanChallenge),
    /// Asked for a typed TAN (login or order).
    pub typed_tan: fn(&TanChallenge) -> String,
}

impl RunOptions {
    /// `days` of transactions, default waits, no output, typed TAN
    /// `123456`.
    pub fn new(days: u32) -> Self {
        Self {
            days,
            bank_waits: true,
            first_wait: Duration::from_secs(5),
            next_wait: Duration::from_secs(5),
            timeout: Duration::from_secs(300),
            on_event: |_| {},
            wait_for_user: |_| {},
            typed_tan: |_| "123456".into(),
        }
    }
}

/// What the jobs of one account returned.
#[derive(Debug, Default)]
pub struct AccountRun {
    /// `None` without a balance job or when the bank sent no HISAL.
    pub balance: Option<AccountBalance>,
    /// Booked entries (all pages).
    pub transactions: Vec<Transaction>,
    /// The pending part and its creation time, as [`Statement`] has them;
    /// `None` without a transactions job or when it failed.
    pub pending: Option<(Pending, Option<fints_client::BankTime>)>,
    pub holdings: Vec<SecurityHolding>,
    /// The notices of every job's messages, in order.
    pub notices: Vec<Notice>,
    /// The jobs that failed, with their error. The others still ran.
    pub errors: Vec<(JobKind, RunError)>,
}

/// Why a job or the run of [`standard_run`] failed.
#[derive(Debug)]
pub enum RunError {
    /// The library returned an error.
    Fints(FinTSError),
    /// A decoupled TAN was not confirmed within [`RunOptions::timeout`];
    /// the job was cancelled.
    TanTimeout,
}

impl From<FinTSError> for RunError {
    fn from(e: FinTSError) -> Self {
        Self::Fints(e)
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fints(e) => e.fmt(f),
            Self::TanTimeout => f.write_str("TAN timeout: not confirmed within the allowed time"),
        }
    }
}

impl std::error::Error for RunError {}

/// What [`standard_run`] got.
pub struct RunReport {
    /// The accounts of the UPD, in order.
    pub accounts: Vec<fints_client::jobs::AccountInfo>,
    /// The login's notices.
    pub notices: Vec<Notice>,
    /// One result per account; `Err` only when the account cannot be
    /// addressed (no IBAN and BIC), job errors are in [`AccountRun::errors`].
    pub fetched: Vec<Result<AccountRun, RunError>>,
    /// Per account, the orders left out because its UPD does not allow
    /// them.
    pub skipped: Vec<Vec<&'static str>>,
}

impl RunReport {
    /// Every error of the run: accounts that cannot be addressed and
    /// failed jobs.
    pub fn errors(&self) -> Vec<&RunError> {
        self.fetched
            .iter()
            .flat_map(|r| match r {
                Ok(run) => run.errors.iter().map(|(_, e)| e).collect(),
                Err(e) => vec![e],
            })
            .collect()
    }
}

/// Answers the TANs of `step` until the job is done: a typed TAN from
/// `opts.typed_tan`, a decoupled one by polling after the bank's wait (or
/// the option's when the bank gives none). This is the loop a caller
/// writes; the library never sleeps.
async fn complete<T: JobValue>(
    dialog: &mut Dialog,
    mut step: Step<T>,
    opts: &RunOptions,
) -> Result<Completed<T>, RunError> {
    let started = Instant::now();
    loop {
        let (challenge, resume) = match step {
            Step::Done(done) => return Ok(done),
            Step::NeedTan(challenge, resume) => (challenge, resume),
            // Only a transfer pauses on its payee check; the run sends none.
            Step::NeedVopConfirmation(..) | Step::VopPending(..) => {
                return Err(
                    FinTSError::Internal("a read job paused on a payee check".into()).into(),
                );
            }
        };
        step = match challenge.kind {
            TanKind::Typed { .. } => {
                let tan = (opts.typed_tan)(&challenge);
                dialog.submit_tan(resume, &tan).await?
            }
            TanKind::Decoupled {
                polls_done,
                auto_polling_allowed,
                ..
            } => {
                if polls_done == 0 {
                    (opts.on_event)(RunEvent::AwaitingConfirmation {
                        challenge: challenge.challenge.to_string(),
                    });
                }
                if started.elapsed() > opts.timeout {
                    // The run sends reads only: nothing is left at the bank.
                    let _ = dialog.cancel(resume)?;
                    return Err(RunError::TanTimeout);
                }
                let own = if polls_done == 0 {
                    opts.first_wait
                } else {
                    opts.next_wait
                };
                let wait = match challenge.not_before.filter(|_| opts.bank_waits) {
                    Some(at) => {
                        std::time::Duration::try_from(at.duration_since(jiff::Timestamp::now()))
                            .unwrap_or_default()
                    }
                    None => own,
                };
                tokio::time::sleep(wait).await;
                if auto_polling_allowed == Some(false) {
                    (opts.wait_for_user)(&challenge);
                }
                let next = dialog.poll(resume).await?;
                (opts.on_event)(match &next {
                    Step::Done(_) => RunEvent::Confirmed,
                    Step::NeedTan(..) | Step::NeedVopConfirmation(..) | Step::VopPending(..) => {
                        RunEvent::StillPending {
                            poll: polls_done + 1,
                        }
                    }
                });
                next
            }
        };
    }
}

/// The run that is recorded at a bank and replayed in the tests: a first
/// login without stored state (synchronisation, then the dialog with its
/// TAN, decoupled or typed), then for every account of the UPD balance and
/// transactions (holdings too for a depot), each only if the account's UPD
/// allows the order and the bank offers it (`Dialog::capabilities`), each
/// job answering its TAN if the bank asks, then the dialog end. A failed
/// job is reported and the run goes on.
pub async fn standard_run(
    client: &BankClient,
    user: &UserId,
    pin: &Pin,
    opts: &RunOptions,
) -> Result<RunReport, RunError> {
    let creds = Credentials::new(user.clone(), pin.clone());
    let state = SessionState::new(&client.config().blz, user);
    let (mut dialog, step) = jobs::open(client, &creds, &state).await?;
    let opened = complete(&mut dialog, step, opts).await?;
    let accounts = opened.value.accounts.clone();
    let mut fetched = Vec::new();
    let mut skipped = Vec::new();
    for acc in &accounts {
        let account = match acc.account() {
            Ok(a) => a,
            Err(e) => {
                fetched.push(Err(e.into()));
                skipped.push(Vec::new());
                continue;
            }
        };
        let caps = dialog.capabilities(std::slice::from_ref(&account));
        let offered = |job: JobKind| {
            caps.accounts
                .first()
                .and_then(|a| a.job(job))
                .is_some_and(|j| matches!(j.support, Support::Available { .. }))
        };
        let mut left_out = Vec::new();
        let mut want = |code: &'static str, job: JobKind, wanted: bool| {
            let allowed = wanted && acc.allows(job);
            if wanted && !allowed {
                left_out.push(code);
            }
            allowed && offered(job)
        };
        let balance = want("HKSAL", JobKind::Balance, true);
        let transactions = want("HKKAZ", JobKind::Transactions, true);
        let holdings = want("HKWPD", JobKind::Holdings, acc.is_depot());
        skipped.push(left_out);
        fetched.push(Ok(run_account(
            &mut dialog,
            &account,
            [balance, transactions, holdings],
            opts,
            client.today(),
        )
        .await));
    }
    dialog.end().await?;
    Ok(RunReport {
        accounts,
        notices: opened.notices,
        fetched,
        skipped,
    })
}

/// Balance, transactions and holdings of one account, as far as `wanted`
/// says; a failed job is recorded and the next one runs.
async fn run_account(
    dialog: &mut Dialog,
    account: &Account,
    wanted: [bool; 3],
    opts: &RunOptions,
    today: jiff::civil::Date,
) -> AccountRun {
    let mut run = AccountRun::default();
    if wanted[0] {
        let result = async {
            let step = jobs::balance(dialog, account).await?;
            complete(dialog, step, opts).await
        }
        .await;
        match result {
            Ok(done) => {
                run.balance = done.value;
                run.notices.extend(done.notices);
            }
            Err(e) => run.errors.push((JobKind::Balance, e)),
        }
    }
    if wanted[1] {
        let range = Range::new(
            today - jiff::Span::new().days(opts.days.max(1) as i64),
            today,
        );
        let result = async {
            let step = jobs::transactions(dialog, account, range, Format::Auto).await?;
            complete(dialog, step, opts).await
        }
        .await;
        match result {
            Ok(done) => {
                let statement: Statement = done.value;
                run.transactions = statement.booked;
                run.pending = Some((statement.pending, statement.pending_created_at));
                run.notices.extend(done.notices);
            }
            Err(e) => run.errors.push((JobKind::Transactions, e)),
        }
    }
    if wanted[2] {
        let result = async {
            let step = jobs::holdings(dialog, account, None).await?;
            complete(dialog, step, opts).await
        }
        .await;
        match result {
            Ok(done) => {
                run.holdings = done.value;
                run.notices.extend(done.notices);
            }
            Err(e) => run.errors.push((JobKind::Holdings, e)),
        }
    }
    run
}

// ── Expected differences (`.expect` sidecar) ────────────────────────────

/// The `.expect` sidecar of the trace at `trace_path`: `<name>.expect` next
/// to `<name>.trace.json`.
pub fn expect_path(trace_path: &std::path::Path) -> std::path::PathBuf {
    let name = trace_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let stem = name.strip_suffix(".trace.json").unwrap_or(name);
    trace_path.with_file_name(format!("{stem}.expect"))
}

/// Reads the lines of a `.expect` sidecar: `#` comment lines and blank
/// lines are skipped; every other line is `<exchange> <message>`, as
/// [`Violation`] prints it.
pub fn expected_violations(text: &str) -> Result<Vec<Violation>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.trim_start().is_empty() || line.starts_with('#') {
            continue;
        }
        let (exchange, message) = line
            .split_once(' ')
            .ok_or_else(|| format!("line {}: expected `<exchange> <message>`", n + 1))?;
        let exchange = exchange
            .parse()
            .map_err(|_| format!("line {}: {exchange:?} is not an exchange number", n + 1))?;
        if message.trim().is_empty() {
            return Err(format!("line {}: no message", n + 1));
        }
        out.push(Violation {
            exchange,
            message: message.to_string(),
        });
    }
    Ok(out)
}

/// The comment lines (starting with `#`) of a `.expect` sidecar, in order.
pub fn expect_comments(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// A `.expect` sidecar: the comment lines (each starting with `#`), then one
/// line per violation, sorted by exchange and message.
pub fn expect_text(comments: &[String], violations: &[Violation]) -> String {
    let mut sorted: Vec<&Violation> = violations.iter().collect();
    sorted.sort_by(|a, b| (a.exchange, &a.message).cmp(&(b.exchange, &b.message)));
    let mut out = String::new();
    for c in comments {
        out.push_str(c);
        out.push('\n');
    }
    for v in sorted {
        out.push_str(&format!("{v}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expect_sidecar_round_trips() {
        let text = "# client: KTI with national parts; re-record to clear\n\n\
                    7 HKKAZ:3 data element 1: 2 group elements recorded, 6 sent\n\
                    3 segments differ: recorded [HKSAL:3:6], sent [HKWPD:3:6]\n";
        let v = expected_violations(text).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].exchange, 7);
        assert_eq!(v[1].exchange, 3);
        assert_eq!(
            v[1].message,
            "segments differ: recorded [HKSAL:3:6], sent [HKWPD:3:6]"
        );
        assert_eq!(v[0].to_string(), text.lines().nth(2).unwrap());
        let comments = expect_comments(text);
        assert_eq!(
            comments,
            ["# client: KTI with national parts; re-record to clear"]
        );
        let written = expect_text(&comments, &v);
        // Sorted by exchange; reading it back gives the same lines.
        assert!(written.lines().nth(1).unwrap().starts_with("3 "));
        let mut again = expected_violations(&written).unwrap();
        let mut orig = v;
        again.sort_by_key(|v| v.exchange);
        orig.sort_by_key(|v| v.exchange);
        assert_eq!(again, orig);
    }

    #[test]
    fn expect_sidecar_rejects_bad_lines() {
        assert!(
            expected_violations("seven message\n")
                .unwrap_err()
                .contains("line 1")
        );
        assert!(
            expected_violations("# c\n7\n")
                .unwrap_err()
                .contains("line 2")
        );
        assert!(expected_violations("# only comments\n").unwrap().is_empty());
    }

    #[test]
    fn expect_path_sits_next_to_the_trace() {
        let p = expect_path(std::path::Path::new("a/local/bank.trace.json"));
        assert_eq!(p, std::path::Path::new("a/local/bank.expect"));
    }

    #[test]
    fn a_recorded_failed_exchange_is_answered_with_its_error() {
        let request = "HNHBK:1:3+0+300+0+1'HKIDN:2:2+280:12345678+u+0+1'";
        let trace = Trace {
            exchanges: vec![fints_client::trace::Exchange {
                request: request.as_bytes().to_vec(),
                response: Vec::new(),
                error: Some("HTTP 500".into()),
                account: None,
                notes: Vec::new(),
            }],
            ..Trace::default()
        };
        let mut state = State {
            trace,
            next: 0,
            violations: Vec::new(),
            client: Client {
                user_id: DEFAULT_USER_ID.into(),
            },
        };
        assert_eq!(state.respond(request.as_bytes()), Err("HTTP 500".into()));
        assert!(state.violations.is_empty());
    }

    #[test]
    fn compare_reports_versions_counts_and_tan_process() {
        let shapes = |s: &str| shapes(s.as_bytes()).unwrap();
        let recorded = shapes("HKSAL:3:7+a:b+N'HKTAN:4:7+4+HKSAL'");
        assert!(compare(&recorded, &shapes("HKSAL:3:7+x:y+J'HKTAN:4:7+4+HKSAL'")).is_empty());
        assert_eq!(
            compare(&recorded, &shapes("HKSAL:3:6+a:b+N'HKTAN:4:7+4+HKSAL'")),
            ["HKSAL:3: version 7 recorded, 6 sent"]
        );
        assert_eq!(
            compare(&recorded, &shapes("HKSAL:3:7+a+N'HKTAN:4:7+S+HKSAL'")),
            [
                "HKSAL:3 data element 1: 2 group elements recorded, 1 sent",
                "HKTAN:4 process: \"4\" recorded, \"S\" sent"
            ]
        );
        assert!(compare(&recorded, &shapes("HKSAL:3:7+a:b+N'"))[0].starts_with("segments differ"));
    }
}
