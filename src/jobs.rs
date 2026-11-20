//! Jobs as steps (docs/jobs-step-design.md).
//!
//! Every job returns a [`Step`]: `Done` with the value and the bank's
//! notices, or `NeedTan` with a [`TanChallenge`] and a typed [`Resume`]
//! ticket; a [`transfer`] may also pause on its payee check
//! (`NeedVopConfirmation`, `VopPending`). The caller answers on the
//! [`Dialog`]: [`Dialog::submit_tan`] for a typed TAN, [`Dialog::poll`]
//! (one status request per call) for a decoupled confirmation or an
//! unfinished payee check, [`Dialog::confirm_vop`] for a payee check
//! result, or [`Dialog::cancel`]. The library never sleeps and never calls
//! back: the caller waits the bank's `first_wait` / `next_wait` between
//! polls and decides when to give up.
//!
//! ```rust,no_run
//! use fints_client::jobs::{self, BankClient, Credentials, Step, TanKind};
//! # async fn example(client: BankClient, creds: Credentials,
//! #     state: fints_client::SessionState) -> fints_client::Result<()> {
//! let (mut dialog, mut step) = jobs::open(&client, &creds, &state).await?;
//! let opened = loop {
//!     match step {
//!         Step::Done(done) => break done.value,
//!         Step::NeedTan(challenge, resume) => match challenge.kind {
//!             TanKind::Decoupled { .. } => {
//!                 // sleep first_wait / next_wait, then:
//!                 step = dialog.poll(resume).await?;
//!             }
//!             TanKind::Typed { .. } => {
//!                 let tan = "123456"; // ask the user
//!                 step = dialog.submit_tan(resume, tan).await?;
//!             }
//!         },
//!         // Only a transfer pauses on its payee check.
//!         Step::NeedVopConfirmation(_, resume) => step = dialog.confirm_vop(resume).await?,
//!         Step::VopPending(_, resume) => step = dialog.poll(resume).await?,
//!     }
//! };
//! # let _ = opened;
//! let state = dialog.end().await?; // persist (encrypted)
//! # let _ = state;
//! # Ok(())
//! # }
//! ```

use std::marker::PhantomData;
use std::time::Duration;

use jiff::{
    Timestamp,
    civil::{Date, DateTime},
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::banks::{BankConfig, BookingIdSource, InitMode};
use crate::error::{FinTSError, Result};
use crate::formats::mt940::{parse_mt940, parse_mt942};
use crate::protocol::{
    self, Account, Allowance, BankParams, InitResult, Open, Response, Segment, UpdAccount,
};
use crate::return_code::{Category, ClientWarning, Kind, Notice, NoticeDialog, ReturnCode};
use crate::segments::Order;
use crate::segments::read::{extract_camt_data, extract_mt940_data, parse_hiwdu, parse_hiwpd};
use crate::session::SessionState;
use crate::transport::SharedTransport;
use crate::types::*;

// ═══════════════════════════════════════════════════════════════════════════════
// Client and credentials
// ═══════════════════════════════════════════════════════════════════════════════

/// A bank to talk to: its configuration (URL, BLZ, TAN overrides), the
/// product ID registered with the Deutsche Kreditwirtschaft, and the
/// transport (default: HTTPS with feature `http`).
#[derive(Clone)]
pub struct BankClient {
    config: BankConfig,
    product_id: ProductId,
    transport: Option<SharedTransport>,
    quirks: Option<std::sync::Arc<dyn Fn() -> crate::quirks::QuirkChain + Send + Sync>>,
    now: Option<DateTime>,
}

impl BankClient {
    pub fn new(config: BankConfig, product_id: ProductId) -> Self {
        Self {
            config,
            product_id,
            transport: None,
            quirks: None,
            now: None,
        }
    }

    /// Pin the local time for everything the library reads from the clock:
    /// the HNSHK and HNVSK stamps, the pain.001 `CreDtTm` and the date of the
    /// day (the storage-period clamp of [`transactions`]) in this client's
    /// dialogs. For tests and replays, which must not depend on the real
    /// clock; the default is the system's local time. Paused-dialog timing
    /// (`expires_at`, `not_before`) stays on real timestamps.
    pub fn with_now(mut self, now: DateTime) -> Self {
        self.now = Some(now);
        self
    }

    /// The pinned local time ([`BankClient::with_now`]), else the system's.
    /// A consumer that computes date ranges uses it, so its tests can pin
    /// the clock the same way.
    pub fn now(&self) -> DateTime {
        self.now
            .unwrap_or_else(|| jiff::ZonedDateTime::now().datetime())
    }

    /// The date of [`BankClient::now`].
    pub fn today(&self) -> Date {
        self.now().date()
    }

    /// Send through `transport` instead of the default.
    pub fn with_transport(mut self, transport: SharedTransport) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Repair the bank's answers with the chain `quirks` builds (one per
    /// dialog) instead of the default chain.
    pub fn with_quirks(
        mut self,
        quirks: impl Fn() -> crate::quirks::QuirkChain + Send + Sync + 'static,
    ) -> Self {
        self.quirks = Some(std::sync::Arc::new(quirks));
        self
    }

    pub fn config(&self) -> &BankConfig {
        &self.config
    }

    /// The user ID as the bank gets it: completed by the profile's rule.
    pub(crate) fn effective_user_id(&self, user_id: &UserId) -> UserId {
        let Some(rule) = &self.config.profile.user_id else {
            return user_id.clone();
        };
        match rule.apply(user_id.as_str()) {
            std::borrow::Cow::Borrowed(_) => user_id.clone(),
            std::borrow::Cow::Owned(full) => {
                debug!(
                    "[FinTS] User ID completed by the {} profile: {} -> {}",
                    self.config.profile.id,
                    "*".repeat(user_id.as_str().chars().count()),
                    "*".repeat(full.chars().count()),
                );
                UserId::new(full)
            }
        }
    }

    fn new_dialog(&self, creds: &Credentials) -> Result<protocol::Dialog<protocol::New>> {
        let c = &self.config;
        let dialog = protocol::Dialog::new(
            c.url.as_str(),
            &c.blz,
            &self.effective_user_id(&creds.user_id),
            &creds.pin,
            &self.product_id,
        )?
        .with_tan_overrides(c.profile.tan_overrides.clone());
        let dialog = match &self.quirks {
            Some(q) => dialog.with_quirks(q()),
            None => dialog,
        }
        .with_now(self.now);
        Ok(match &self.transport {
            Some(t) => dialog.with_transport(t.clone()),
            None => dialog,
        })
    }
}

impl std::fmt::Debug for BankClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BankClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// User ID and PIN. `Debug` shows neither.
#[derive(Clone)]
pub struct Credentials {
    pub user_id: UserId,
    pub pin: Pin,
}

impl Credentials {
    pub fn new(user_id: UserId, pin: Pin) -> Self {
        Self { user_id, pin }
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials(****)")
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Step, challenge, resume ticket
// ═══════════════════════════════════════════════════════════════════════════════

/// The outcome of one call: the job is done, the bank wants a TAN, or a
/// transfer's payee check needs the user or more time.
#[must_use]
#[derive(Debug)]
pub enum Step<T> {
    Done(Completed<T>),
    /// Answer with [`Dialog::submit_tan`] (typed) or [`Dialog::poll`]
    /// (decoupled), or drop the job with [`Dialog::cancel`].
    NeedTan(TanChallenge, Resume<T>),
    /// The payee check is not a clean match: show the result (and the
    /// bank's explanation), then [`Dialog::confirm_vop`] or
    /// [`Dialog::cancel`].
    NeedVopConfirmation(VopConfirmation, Resume<T>),
    /// The payee check is still running: [`Dialog::poll`] at or after
    /// `not_before`, or [`Dialog::cancel`].
    VopPending(VopWait, Resume<T>),
}

/// A finished job: its value and every notice of every message it sent,
/// those before a `NeedTan` included (return-codes design §4).
#[derive(Debug, Clone)]
pub struct Completed<T> {
    pub value: T,
    pub notices: Vec<Notice>,
}

/// The ticket to resume the job a step paused. The dialog checks it
/// against its pending job ([`FinTSError::WrongResume`]).
#[derive(Debug)]
pub struct Resume<T> {
    ticket: Ticket,
    _value: PhantomData<fn() -> T>,
}

/// What a paused job is resumed by: the bank's task reference for a TAN,
/// a random nonce for the payee check waits (no task reference exists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Ticket {
    Task(TaskReference),
    Local(u64),
}

impl Ticket {
    fn local() -> Self {
        Self::Local(rand::random())
    }
}

/// What a paused job waits for.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum Wait {
    Tan(TanChallenge),
    VopConfirmation(VopConfirmation),
    VopPoll(VopWait),
}

/// What the bank asks the user to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TanChallenge {
    /// The bank's text to show.
    pub challenge: ChallengeText,
    /// The bank's HITANS "Challenge strukturiert" for the TAN method in
    /// use (PIN/TAN, data dictionary, DE "Challenge"): when `true`,
    /// `challenge` may carry the tags `<br> <p> <b> <i> <u> <ul> <ol> <li>
    /// <a href>`, not nested. When `false` it is plain text.
    pub challenge_markup: bool,
    pub kind: TanKind,
    /// Until when the bank accepts the answer (HITAN "Gültigkeitsdatum und
    /// -uhrzeit für Challenge"), in the bank's local time as sent; `None`
    /// when the bank gives none.
    pub valid_until: Option<DateTime>,
    /// Decoupled challenges: the earliest time for the next [`Dialog::poll`],
    /// the time the bank's answer arrived plus the HITANS wait that applies
    /// (`first_wait` before the first poll, `next_wait` after each further
    /// 3956). `None` when the bank gives no wait, and for typed challenges.
    /// It is a wall-clock time and survives freeze and thaw; sleep until it
    /// instead of for a fixed duration.
    pub not_before: Option<Timestamp>,
    /// A transfer's payee check result, to show with the TAN prompt or app
    /// approval (VoP E.8.1 point 3); `None` for every other job and for a
    /// transfer without a payee check.
    pub payee_check: Option<VopResult>,
    /// Why the challenge came back: the bank rejected the TAN typed for
    /// it and keeps the order for another one. `None` on the first ask.
    pub rejected: Option<Box<TanRejected>>,
}

/// A typed TAN the bank rejected while keeping the order
/// ([`Dialog::submit_tan`]; docs/jobs-step-design.md §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TanRejected {
    /// The bank's rejection to show, code and text (9941 "TAN ungültig").
    pub code: ReturnCode,
    /// Further TANs the bank says it will check before it locks (9360 P1);
    /// `None` when it does not say.
    pub attempts_left: Option<u32>,
    /// Wrong TANs for this order so far, from 1.
    pub count: u32,
}

/// How the challenge is answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TanKind {
    /// The user types a TAN: [`Dialog::submit_tan`]. `media` is the
    /// flicker code or photoTAN/QR image to show, if the challenge has one.
    Typed { media: Option<TanMedia> },
    /// The user confirms in an app: [`Dialog::poll`]. The waits and the
    /// poll limit are the bank's HITANS v7 values (`None` when it gives
    /// none); `polls_done` counts the status requests sent so far.
    ///
    /// `auto_polling_allowed`: when `Some(false)` the caller must not call
    /// [`Dialog::poll`] on a timer. The bank then expects the user to
    /// trigger each status request (a button, pressing Enter).
    Decoupled {
        first_wait: Option<Duration>,
        next_wait: Option<Duration>,
        max_polls: Option<u32>,
        polls_done: u32,
        auto_polling_allowed: Option<bool>,
    },
}

impl TanChallenge {
    fn from_bank(
        challenge: &protocol::TanChallenge,
        params: &BankParams,
        arrived: Timestamp,
    ) -> Self {
        let mut not_before = None;
        // 3955 alone makes a challenge decoupled; the method does not.
        let kind = if challenge.decoupled {
            let p = params.decoupled_params();
            not_before = not_before_after(arrived, p.wait_before_first_poll);
            TanKind::Decoupled {
                first_wait: p.wait_before_first_poll,
                next_wait: p.wait_before_next_poll,
                max_polls: p.max_polls,
                polls_done: 0,
                auto_polling_allowed: p.auto_polling_allowed,
            }
        } else {
            TanKind::Typed {
                media: challenge
                    .challenge_hhduc
                    .as_ref()
                    .and_then(HhdUcData::decode),
            }
        };
        Self {
            challenge: challenge.challenge.clone(),
            challenge_markup: challenge.challenge_markup || params.challenge_structured(),
            kind,
            valid_until: challenge.valid_until,
            not_before,
            payee_check: None,
            rejected: None,
        }
    }
}

/// `arrived` plus the bank's wait; `None` when the bank gives no wait.
/// Adds `new` to `notices`, skipping a notice already there (same dialog,
/// code, scope, text and parameters). Every message of an order's TAN and
/// status rounds can carry the same 3060; the caller sees it once.
pub(crate) fn extend_notices(notices: &mut Vec<Notice>, new: Vec<Notice>) {
    for notice in new {
        if !notices.contains(&notice) {
            notices.push(notice);
        }
    }
}

fn not_before_after(arrived: Timestamp, wait: Option<Duration>) -> Option<Timestamp> {
    let wait = jiff::SignedDuration::try_from(wait?).ok()?;
    arrived.checked_add(wait).ok()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Job values
// ═══════════════════════════════════════════════════════════════════════════════

/// The value of [`open`]: what the login learned and decided.
#[derive(Debug, Clone)]
pub struct Opened {
    /// Accounts of the UPD, in its order.
    pub accounts: Vec<AccountInfo>,
    /// TAN methods the BPD offers (buildable HITANS versions).
    pub tan_methods: Vec<TanMethod>,
    /// Security functions the bank allows this user (3920).
    pub allowed_security_functions: Vec<SecurityFunction>,
    /// The TAN method the login chose (the one the dialog uses, with the
    /// bank's name for it); `None` for PIN only (999) or a method the BPD
    /// does not announce. The same as in the session state after
    /// [`Dialog::end`].
    pub selected_tan_method: Option<TanMethod>,
}

/// The value type of a job; implemented for the value of every job.
pub trait JobValue: sealed::Sealed + Sized {}
impl JobValue for Opened {}
impl JobValue for Option<AccountBalance> {}
impl JobValue for Statement {}
impl JobValue for Vec<SecurityHolding> {}
impl JobValue for Vec<DepotTransaction> {}
impl JobValue for TransferReceipt {}
impl JobValue for InstantStatus {}

mod sealed {
    use super::*;

    /// Which job a pending job is.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Job {
        Open,
        Balance,
        Transactions,
        Holdings,
        DepotTransactions,
        Transfer,
        InstantStatus,
    }

    /// A finished job's value, whatever the job.
    pub enum Value {
        Opened(Opened),
        Balance(Option<AccountBalance>),
        Statement(Statement),
        Holdings(Vec<SecurityHolding>),
        DepotTransactions(Vec<DepotTransaction>),
        Transfer(TransferReceipt),
        InstantStatus(InstantStatus),
    }

    pub trait Sealed: Sized {
        const JOB: Job;
        fn from_value(value: Value) -> Option<Self>;
    }

    impl Sealed for Opened {
        const JOB: Job = Job::Open;
        fn from_value(value: Value) -> Option<Self> {
            match value {
                Value::Opened(v) => Some(v),
                _ => None,
            }
        }
    }

    impl Sealed for Option<AccountBalance> {
        const JOB: Job = Job::Balance;
        fn from_value(value: Value) -> Option<Self> {
            match value {
                Value::Balance(v) => Some(v),
                _ => None,
            }
        }
    }
}

macro_rules! sealed_value {
    ($($ty:ty => $job:ident, $variant:ident;)*) => {$(
        impl Sealed for $ty {
            const JOB: Job = Job::$job;
            fn from_value(value: Value) -> Option<Self> {
                match value {
                    Value::$variant(v) => Some(v),
                    _ => None,
                }
            }
        }
    )*};
}
sealed_value! {
    Statement => Transactions, Statement;
    Vec<SecurityHolding> => Holdings, Holdings;
    Vec<DepotTransaction> => DepotTransactions, DepotTransactions;
    TransferReceipt => Transfer, Transfer;
    InstantStatus => InstantStatus, InstantStatus;
}
use sealed::{Job, Sealed, Value};

mod accounts;
pub use accounts::AccountInfo;
mod capabilities;
pub use capabilities::{
    AccountCapabilities, Capabilities, JobCapability, JobKind, Support, VopSupport,
};
mod paused;
pub use paused::{AnyResume, PAUSED_DIALOG_FORMAT, PausedDialog, PausedFormat, ResumeKind};
mod instant_status;
pub use instant_status::{InstantStatus, SepaOrderStatus, StatusSource, instant_status};
mod transfer;
pub use transfer::{
    CheckedTransfer, Instant, InvalidReason, InvalidTransfer, PayeeOutcome, PayeeResult, SentAs,
    Transfer, TransferField, TransferReceipt, TransferStatus, VopConfirmation, VopResult, VopWait,
    transfer,
};
use transfer::{PaymentIds, PaymentJob};

/// A booking date range, both days inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub from: Date,
    pub to: Date,
}

impl Range {
    pub fn new(from: Date, to: Date) -> Self {
        Self { from, to }
    }
}

/// Which statement format [`transactions`] asks the bank for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// camt.052 (HKCAZ) when the bank offers a version we build and the UPD
    /// does not rule it out for the account, else MT940 (HKKAZ). Decided
    /// before anything is sent; a bank that rejects HKCAZ is an error, not
    /// a reason to try MT940.
    #[default]
    Auto,
    /// camt.052 (HKCAZ) only.
    Camt,
    /// MT940/MT942 (HKKAZ) only.
    Mt940,
}

/// The format a statement came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatementFormat {
    Camt,
    Mt940,
}

/// What a statement answer says about pending (vorgemerkte) entries. Only
/// [`Pending::Reported`] is a complete list: a caller that deletes stored
/// pending entries which the bank no longer lists must do so on `Reported`
/// only.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Pending {
    /// The answer carried the pending part (camt pending document, MT942)
    /// and it parsed; the entries are marked
    /// [`TransactionStatus::Pending`]. Empty when nothing is pending.
    Reported(Vec<Transaction>),
    /// The answer had no pending part: the bank does not report pending
    /// entries in this format, or none for this request.
    NotSent,
    /// The bank sent a pending part that did not parse (banks fill MT942 in
    /// ways the grammar does not always accept), or an MT942 whose
    /// `:90D:`/`:90C:` counts do not match its bookings (the list would not
    /// be complete; a sum that does not match is only logged); the booked
    /// entries are unaffected. `kind`
    /// and `message` are those of the parse error.
    Unreadable { kind: Category, message: String },
}

impl Pending {
    /// The entries when the list is complete ([`Pending::Reported`]).
    pub fn reported(&self) -> Option<&[Transaction]> {
        match self {
            Self::Reported(t) => Some(t),
            _ => None,
        }
    }
}

/// The value of [`transactions`].
#[derive(Debug, Clone)]
pub struct Statement {
    /// Booked entries, in the bank's order, all pages.
    pub booked: Vec<Transaction>,
    /// Pending entries and whether the list is complete.
    pub pending: Pending,
    /// When the bank made the pending list: the MT942 Erstellungszeitpunkt
    /// (`:13D:` with offset, `:13:` without; HBCI 2.2 IX.2.9), the earliest
    /// one when the pages carry several. Set only with
    /// [`Pending::Reported`] from an MT942 that has one; `None` for camt.
    /// A caller can tell an old list (a bank that answers from a cache)
    /// from a fresh one.
    pub pending_created_at: Option<BankTime>,
    /// The format the bank was asked for.
    pub format: StatementFormat,
}

/// A job and its inputs, as plain data. Paging jobs also carry their
/// touchdown point and the pages received so far, so a job that pauses for
/// a TAN on any page keeps them.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum JobInput {
    /// The dialog initialisation (login).
    Open,
    /// HKSAL for an account.
    Balance { account: Account },
    /// HKKAZ or HKCAZ.
    Transactions {
        account: Account,
        range: Range,
        format: StatementFormat,
        touchdown: Option<TouchdownPoint>,
        /// Booked statement data of the pages so far (MT940: raw bytes
        /// joined as sent; camt: one XML document per element).
        booked: Vec<Vec<u8>>,
        pending: Vec<Vec<u8>>,
        /// Whether a page carried the pending part, possibly empty.
        pending_sent: bool,
        /// The status of an instant payment read from the statement (no
        /// HKIPS): the end-to-end ID to look for. The job's value is then
        /// an [`InstantStatus`].
        find_payment: Option<String>,
    },
    /// HKWPD.
    Holdings {
        account: Account,
        currency: Option<Currency>,
        touchdown: Option<TouchdownPoint>,
        found: Vec<SecurityHolding>,
    },
    /// HKWDU.
    DepotTransactions {
        account: Account,
        range: Option<Range>,
        touchdown: Option<TouchdownPoint>,
        found: Vec<DepotTransaction>,
    },
    /// HKCCS or HKIPZ with the payee check.
    Transfer(Box<PaymentJob>),
    /// HKIPS; `min_wait` (seconds) is HIIPSS "Mindestwartezeit".
    InstantStatus {
        account: Account,
        descriptors: Vec<String>,
        order_id: String,
        min_wait: Option<u32>,
    },
}

/// `outcome` of a message that carried a payment's order or TAN (`ids`):
/// every error but a class 9 rejection is an unknown outcome.
fn after_order<T>(ids: Option<&PaymentIds>, outcome: Result<T>) -> Result<T> {
    match ids {
        Some(ids) => outcome.map_err(|e| ids.after_order(e)),
        None => outcome,
    }
}

impl JobInput {
    /// The payment's IDs when the job is a transfer.
    fn payment_ids(&self) -> Option<PaymentIds> {
        match self {
            Self::Transfer(p) => Some(p.ids()),
            _ => None,
        }
    }

    /// What a request of this job carries.
    fn in_flight(&self) -> InFlight {
        match self.payment_ids() {
            Some(ids) => InFlight::Payment(ids),
            None => InFlight::Message,
        }
    }

    /// Segments that go before HKTAN in the next TAN message.
    fn tan_extra(&self) -> Vec<Segment> {
        match self {
            Self::Transfer(p) => p.tan_extra(),
            _ => Vec::new(),
        }
    }

    fn job(&self) -> Job {
        match self {
            Self::Open => Job::Open,
            Self::Balance { .. } => Job::Balance,
            Self::Transactions {
                find_payment: Some(_),
                ..
            } => Job::InstantStatus,
            Self::Transactions { .. } => Job::Transactions,
            Self::Holdings { .. } => Job::Holdings,
            Self::DepotTransactions { .. } => Job::DepotTransactions,
            Self::Transfer(_) => Job::Transfer,
            Self::InstantStatus { .. } => Job::InstantStatus,
        }
    }

    /// The order segment (for the current page) and its response's data
    /// segment. `None` for the login and for a transfer, which has no data
    /// segment and is never sent again (payments design 3.5).
    fn order(&self) -> Option<(Segment, &'static str, &'static str)> {
        Some(match self {
            Self::Open | Self::Transfer(_) => return None,
            Self::Balance { account } => (
                Segment::Balance {
                    account: account.clone(),
                },
                "HKSAL",
                "HISAL",
            ),
            Self::Transactions {
                account,
                range,
                format,
                touchdown,
                ..
            } => match format {
                StatementFormat::Mt940 => (
                    Segment::Transactions {
                        account: account.clone(),
                        start_date: range.from,
                        end_date: range.to,
                        touchdown: touchdown.clone(),
                    },
                    "HKKAZ",
                    "HIKAZ",
                ),
                StatementFormat::Camt => (
                    Segment::CamtTransactions {
                        account: account.clone(),
                        start_date: range.from,
                        end_date: range.to,
                        touchdown: touchdown.clone(),
                    },
                    "HKCAZ",
                    "HICAZ",
                ),
            },
            Self::Holdings {
                account,
                currency,
                touchdown,
                ..
            } => (
                Segment::Holdings {
                    account: account.clone(),
                    currency: currency.clone(),
                    touchdown: touchdown.clone(),
                },
                "HKWPD",
                "HIWPD",
            ),
            Self::DepotTransactions {
                account,
                range,
                touchdown,
                ..
            } => (
                Segment::DepotTransactions {
                    account: account.clone(),
                    start_date: range.map(|r| r.from),
                    end_date: range.map(|r| r.to),
                    touchdown: touchdown.clone(),
                },
                "HKWDU",
                "HIWDU",
            ),
            Self::InstantStatus {
                account,
                descriptors,
                order_id,
                ..
            } => (
                Segment::InstantStatus {
                    account: account.clone(),
                    descriptors: descriptors.clone(),
                    order_id: order_id.clone(),
                },
                "HKIPS",
                "HIIPS",
            ),
        })
    }

    /// Take the page `response` carries into the job. `true`: the bank
    /// has more (3040) and the job's touchdown point now names it. A bank
    /// repeating the touchdown point it was just given would loop forever:
    /// the job ends there, and the bank's 3040 goes to `notices`.
    fn absorb(
        &mut self,
        response: &Response,
        requested_camt: &str,
        notices: &mut Vec<Notice>,
    ) -> bool {
        let next = response.touchdown();
        let (current, more) = match self {
            Self::Open | Self::Balance { .. } | Self::Transfer(_) | Self::InstantStatus { .. } => {
                return false;
            }
            Self::Transactions {
                format,
                touchdown,
                booked,
                pending,
                pending_sent,
                ..
            } => {
                match format {
                    StatementFormat::Mt940 => {
                        let page = extract_mt940_data(&response.segments);
                        booked.push(page.booked);
                        pending.push(page.pending);
                        *pending_sent |= page.pending_sent;
                    }
                    StatementFormat::Camt => {
                        let page = extract_camt_data(&response.segments);
                        for (descriptor, document) in page.descriptors.iter().zip(&page.booked) {
                            crate::camt::note_version_mismatch(
                                requested_camt,
                                descriptor,
                                document,
                            );
                        }
                        booked.extend(page.booked);
                        pending.extend(page.pending);
                        *pending_sent |= page.pending_sent;
                    }
                }
                (touchdown, next)
            }
            Self::Holdings {
                touchdown, found, ..
            } => {
                found.extend(parse_hiwpd(&response.segments));
                (touchdown, next)
            }
            Self::DepotTransactions {
                touchdown, found, ..
            } => {
                found.extend(parse_hiwdu(&response.segments));
                (touchdown, next)
            }
        };
        match more {
            Some(point) if current.as_ref() == Some(&point) => {
                warn!("[FinTS] The bank repeated touchdown point {point}; stopping");
                notices.extend(
                    response
                        .all_codes()
                        .filter(|c| matches!(c.kind(), Kind::Touchdown(_)))
                        .map(|c| Notice::new(NoticeDialog::Job, c.clone())),
                );
                false
            }
            Some(point) => {
                info!("[FinTS] Touchdown {point}: more data");
                *current = Some(point);
                true
            }
            None => false,
        }
    }

    /// The job's value, once the last page is in. `response` is that page.
    /// `booking_id` is the bank profile's rule for statements.
    fn finish(self, response: &Response, booking_id: BookingIdSource) -> Result<Value> {
        Ok(match self {
            Self::Open | Self::Transfer(_) => {
                return Err(FinTSError::Internal("not a read order".into()));
            }
            Self::Balance { .. } => Value::Balance(
                response
                    .find_segment("HISAL")
                    .and_then(crate::segments::read::balance),
            ),
            Self::Transactions {
                format,
                booked,
                pending,
                pending_sent,
                find_payment,
                ..
            } => {
                let statement =
                    parse_statement(format, &booked, &pending, pending_sent, booking_id)?;
                match find_payment {
                    Some(id) => {
                        Value::InstantStatus(instant_status::from_statement(&statement, &id))
                    }
                    None => Value::Statement(statement),
                }
            }
            Self::Holdings { found, .. } => Value::Holdings(found),
            Self::DepotTransactions { found, .. } => Value::DepotTransactions(found),
            Self::InstantStatus { min_wait, .. } => {
                Value::InstantStatus(instant_status::read(response, min_wait)?)
            }
        })
    }
}

/// Parse the statement data of all pages. Booked data that does not parse
/// fails the job; pending data that does not parse is
/// [`Pending::Unreadable`], and no pending part at all is
/// [`Pending::NotSent`].
///
/// `booking_id` (the profile's rule) applies to the booked part only: a
/// pending booking gets no ID, and an ID shared by two booked entries of
/// the statement (on different pages) is dropped from both.
fn parse_statement(
    format: StatementFormat,
    booked: &[Vec<u8>],
    pending: &[Vec<u8>],
    pending_sent: bool,
    booking_id: BookingIdSource,
) -> Result<Statement> {
    let (booked, pending) =
        match format {
            // Banks split the MT940 stream at any byte, also inside a field:
            // pages are joined as they are.
            StatementFormat::Mt940 => (
                parse_mt940(&booked.concat(), TransactionStatus::Booked),
                parse_mt942(&pending.concat()).map(|p| (p.transactions, p.created)),
            ),
            StatementFormat::Camt => (
                booked
                    .iter()
                    .try_fold(Vec::new(), |mut all, doc| {
                        all.extend(crate::camt::parse_camt052(doc, booking_id)?);
                        Ok(all)
                    })
                    .map(|mut all: Vec<Transaction>| {
                        crate::camt::drop_shared_booking_ids(&mut all);
                        all
                    }),
                pending
                    .iter()
                    .try_fold(Vec::new(), |mut all, doc| {
                        let none = BookingIdSource::None;
                        all.extend(crate::camt::parse_camt052(doc, none)?.into_iter().map(
                            |mut t| {
                                t.status = TransactionStatus::Pending;
                                t
                            },
                        ));
                        Ok(all)
                    })
                    .map(|all| (all, None)),
            ),
        };
    let (pending, pending_created_at) = match pending {
        _ if !pending_sent => (Pending::NotSent, None),
        Ok((pending, created)) => (Pending::Reported(pending), created),
        Err(e) => (
            Pending::Unreadable {
                kind: e.kind(),
                message: e.to_string(),
            },
            None,
        ),
    };
    Ok(Statement {
        booked: booked?,
        pending,
        pending_created_at,
        format,
    })
}

/// The job a paused dialog waits on: plain data, no transport, no clock.
/// Part of a [`PausedDialog`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingJob {
    ticket: Ticket,
    wait: Wait,
    job: JobInput,
    /// Notices of the job's messages so far.
    notices: Vec<Notice>,
}

impl PendingJob {
    /// The TAN challenge the job waits on; `None` while a transfer waits
    /// on its payee check.
    pub fn challenge(&self) -> Option<&TanChallenge> {
        match &self.wait {
            Wait::Tan(c) => Some(c),
            _ => None,
        }
    }

    /// The payee check result a transfer waits to have confirmed.
    pub fn vop(&self) -> Option<&VopConfirmation> {
        match &self.wait {
            Wait::VopConfirmation(c) => Some(c),
            _ => None,
        }
    }

    /// The unfinished payee check a transfer waits on.
    pub fn vop_wait(&self) -> Option<&VopWait> {
        match &self.wait {
            Wait::VopPoll(w) => Some(w),
            _ => None,
        }
    }

    /// The bank's task reference of a TAN wait.
    fn task_reference(&self) -> Result<TaskReference> {
        match &self.ticket {
            Ticket::Task(t) => Ok(t.clone()),
            Ticket::Local(_) => Err(FinTSError::Internal(
                "TAN wait without task reference".into(),
            )),
        }
    }
}

/// Where a dialog is.
#[derive(Debug, Clone)]
pub enum Phase {
    /// Ready for a job.
    Open,
    /// A job waits for a TAN, a decoupled confirmation or a payee check.
    Paused(Box<PendingJob>),
    /// Over; every job returns [`FinTSError::DialogEnded`].
    Ended,
}

/// A job's progress inside the dialog, before it is typed for the caller.
enum Outcome {
    Done(Value, Vec<Notice>),
    Paused(Wait, Ticket),
}

impl Outcome {
    fn into_step<T: Sealed>(self) -> Result<Step<T>> {
        match self {
            Self::Done(value, notices) => T::from_value(value)
                .map(|value| Step::Done(Completed { value, notices }))
                .ok_or_else(|| FinTSError::Internal("job finished with another value".into())),
            Self::Paused(wait, ticket) => {
                let resume = Resume {
                    ticket,
                    _value: PhantomData,
                };
                Ok(match wait {
                    Wait::Tan(c) => Step::NeedTan(c, resume),
                    Wait::VopConfirmation(c) => Step::NeedVopConfirmation(c, resume),
                    Wait::VopPoll(w) => Step::VopPending(w, resume),
                })
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Dialog
// ═══════════════════════════════════════════════════════════════════════════════

/// An open dialog with the bank. Holds the paused job, if any; the caller
/// owns it and passes it to every job. End it with [`Dialog::end`] to get
/// the updated [`SessionState`].
///
/// **Not cancel-safe.** The futures of [`open`], of every job and of
/// [`Dialog::submit_tan`], [`Dialog::poll`] and [`Dialog::confirm_vop`]
/// must run to completion: dropping one mid-request (`select!`, a
/// timeout around the call) loses what was sent. The dialog then ends;
/// when the request carried a transfer's order or TAN, the next call
/// returns [`FinTSError::PaymentOutcomeUnknown`] with the payment's IDs
/// once, and [`FinTSError::DialogEnded`] after that. Cancel between calls
/// (in the wait until `not_before`) with [`Dialog::cancel`]; timeouts
/// belong in the transport.
pub struct Dialog {
    inner: protocol::Dialog<Open>,
    /// The bank's BIC, for UPD accounts without one of their own.
    bic: Bic,
    /// The bank profile's rule for [`Transaction::booking_id`].
    booking_id: BookingIdSource,
    phase: Phase,
    /// Wall clock of the last response from the bank.
    last_message_at: Timestamp,
    /// A request is in flight. Still set on the next call when the caller
    /// dropped the future: the message number is then unknown, and a
    /// payment's outcome too.
    in_flight: Option<InFlight>,
}

/// What the request in flight carried.
#[derive(Debug, Clone)]
enum InFlight {
    /// Nothing that moves money.
    Message,
    /// A payment order or its TAN: a dropped future leaves the outcome
    /// unknown.
    Payment(PaymentIds),
}

/// What [`Dialog::cancel`] left behind at the bank.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cancelled {
    /// Nothing the bank could execute is pending: a read job, the login,
    /// or a transfer at its payee check ([`Step::NeedVopConfirmation`],
    /// [`Step::VopPending`]), which the bank executes only after the
    /// execution order.
    NothingSent,
    /// The transfer's order went out and waits for its TAN: the bank may
    /// still execute it (the user approves in the app after the cancel).
    /// Treat it like [`FinTSError::PaymentOutcomeUnknown`]: check the
    /// account by these IDs before sending it again.
    OrderSubmitted {
        end_to_end_id: String,
        message_id: String,
    },
}

impl std::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phase = match &self.phase {
            Phase::Open => "Open",
            Phase::Paused(_) => "Paused",
            Phase::Ended => "Ended",
        };
        f.debug_struct("Dialog")
            .field("blz", self.inner.blz())
            .field("phase", &phase)
            .field("last_message_at", &self.last_message_at)
            .finish_non_exhaustive()
    }
}

/// Open a dialog: synchronise first when `state` has no system ID or BPD
/// (a stored system ID without BPD is kept, see below), then the dialog
/// initialisation with HKTAN process 4. The step is `Done` on an SCA
/// exemption, else `NeedTan` for the login. Login notices (3072, …) arrive
/// on the step's `Done`.
///
/// `state` from another bank or user is ignored. A stored system ID
/// without BPD is reused through a parameter probe instead of HKSYN (which
/// would assign a new one). If the bank rejects the stored system ID
/// (9391), the setup runs once more with a full synchronisation.
pub async fn open(
    client: &BankClient,
    creds: &Credentials,
    state: &SessionState,
) -> Result<(Dialog, Step<Opened>)> {
    let state = state_for(client, creds, state);
    match open_once(client, creds, state).await {
        Err(e)
            if e.kind() == Category::SystemId
                && state.is_some_and(|s| s.system_id.is_assigned()) =>
        {
            warn!("[FinTS] Bank rejected cached system ID ({e}); re-synchronizing");
            open_once(client, creds, None).await
        }
        other => other,
    }
}

async fn open_once(
    client: &BankClient,
    creds: &Credentials,
    state: Option<&SessionState>,
) -> Result<(Dialog, Step<Opened>)> {
    if let InitMode::Direct { security_function } = &client.config().profile.init {
        return open_direct(client, creds, state, security_function).await;
    }
    let mut notices = Vec::new();
    let (mut params, system_id) = match state.filter(|s| s.can_skip_sync()) {
        Some(s) => {
            info!(
                "[FinTS] Using cached BPD v{} / UPD v{} and system ID: no sync dialog",
                s.bpd_version, s.upd_version
            );
            (s.to_params(), s.system_id.clone())
        }
        None => fetch_params(client, creds, state, &mut notices).await?,
    };
    // TAN preferences from the state apply whether or not we synced.
    if let Some(s) = state {
        // A choice in the state wins; without one, keep what params has.
        params.preferred_security_function = s
            .tan_method
            .clone()
            .or(params.preferred_security_function.take());
        params.selected_tan_medium = s.tan_medium.clone().or(params.selected_tan_medium.take());
        params.select_security_function();
    }

    let init = client
        .new_dialog(creds)?
        .with_system_id(&system_id)
        .with_params(&params)
        .init()
        .await?;
    let (inner, challenge) = match init {
        InitResult::Opened(open) => (open, None),
        InitResult::TanRequired(pending, challenge) => (pending.transition(), Some(challenge)),
    };
    let mut dialog = Dialog {
        inner,
        bic: client.config().bic.clone(),
        booking_id: client.config().profile.booking_id,
        phase: Phase::Open,
        last_message_at: Timestamp::now(),
        in_flight: None,
    };
    notices.extend(dialog.inner.take_notices());
    let outcome = match challenge {
        Some(challenge) => dialog.pause(JobInput::Open, &challenge, notices),
        None => Outcome::Done(Value::Opened(dialog.opened()), notices),
    };
    let step = outcome.into_step()?;
    Ok((dialog, step))
}

/// [`InitMode::Direct`]: no synchronisation dialog; the business dialog
/// opens PIN-only with the stored system ID or a client-generated one and
/// the configured security function, and the bank sends its parameters in
/// it.
async fn open_direct(
    client: &BankClient,
    creds: &Credentials,
    state: Option<&SessionState>,
    security_function: &str,
) -> Result<(Dialog, Step<Opened>)> {
    let system_id = state
        .map(|s| &s.system_id)
        .filter(|s| s.is_assigned())
        .cloned()
        .unwrap_or_else(|| SystemId::new(format!("{:016X}", rand::random::<u64>())));
    let (inner, _) = client
        .new_dialog(creds)?
        .with_system_id(&system_id)
        .with_security_function(SecurityFunction::new(security_function))
        .init_no_tan()
        .await?;
    let mut dialog = Dialog {
        inner,
        bic: client.config().bic.clone(),
        booking_id: client.config().profile.booking_id,
        phase: Phase::Open,
        last_message_at: Timestamp::now(),
        in_flight: None,
    };
    let notices = dialog.inner.take_notices();
    let step = Outcome::Done(Value::Opened(dialog.opened()), notices).into_step()?;
    Ok((dialog, step))
}

/// `state` when it belongs to the client's bank and to the user of
/// `creds`. Both user IDs are compared as the bank gets them (completed by
/// the profile's rule), so a state made with the short form of a user ID
/// matches a login with either form.
fn state_for<'a>(
    client: &BankClient,
    creds: &Credentials,
    state: &'a SessionState,
) -> Option<&'a SessionState> {
    let matches = state.blz == client.config().blz
        && client.effective_user_id(&state.user_id) == client.effective_user_id(&creds.user_id);
    if !matches {
        warn!("[FinTS] Ignoring session state: it belongs to another bank or user");
    }
    matches.then_some(state)
}

/// Learn the bank's parameters and the TAN methods it allows this user,
/// without opening a business dialog, so the caller can choose a method
/// ([`SessionState::with_tan_method`], [`SessionState::with_tan_medium`])
/// before [`open`].
///
/// With a stored system ID the parameter probe runs (HKIDN + HKVVB, PIN
/// only, which keeps the system ID), else the synchronisation dialog
/// (HKSYN). Neither carries HKTAN, so no TAN is asked for and the result is
/// not a [`Step`]. The returned state has the BPD, the UPD, the system ID
/// and the allowed security functions (3920); [`SessionState::tan_methods`]
/// lists the methods. `tan_method` holds the caller's preference when the
/// bank allows it, else the method [`open`] would choose itself (the
/// highest decoupled one). TAN medium and learned TAN codes of `state` are
/// kept. A user ID change the bank reports (3072) is in `user_id` and
/// `previous_user_id`; the notices of the dialog are in the result
/// ([`Completed::notices`]; none when nothing is sent).
///
/// `state` from another bank or user is ignored, as in [`open`]. With
/// [`InitMode::Direct`] (no synchronisation dialog, a fixed security
/// function) nothing is sent: the state comes back with `tan_method` set to
/// the profile's security function, which [`open`] uses whatever the
/// caller sets. A rejected system ID (9391) is an error here; [`open`]
/// recovers from it, or [`SessionState::reset`] and synchronise again.
pub async fn synchronise(
    client: &BankClient,
    creds: &Credentials,
    state: &SessionState,
) -> Result<Completed<SessionState>> {
    let config = client.config();
    let user_id = client.effective_user_id(&creds.user_id);
    let state = state_for(client, creds, state);
    if let InitMode::Direct { security_function } = &config.profile.init {
        let mut state = state
            .cloned()
            .unwrap_or_else(|| SessionState::new(&config.blz, &user_id));
        state.tan_method = Some(SecurityFunction::new(security_function));
        return Ok(Completed {
            value: state,
            notices: Vec::new(),
        });
    }
    let mut notices = Vec::new();
    let (mut params, system_id) = fetch_params(client, creds, state, &mut notices).await?;
    for n in &notices {
        info!("[FinTS] Synchronisation: {n}");
    }
    if let Some(s) = state {
        params.preferred_security_function = s.tan_method.clone();
        params.selected_tan_medium = s.tan_medium.clone();
        params
            .tan_required_learned
            .extend(s.tan_required_learned.iter().cloned());
        params.select_security_function();
    }
    Ok(Completed {
        value: SessionState::capture(&config.blz, &user_id, &system_id, &params),
        notices,
    })
}

/// Parameters and system ID without a usable state: a parameter probe
/// with a stored system ID, else a synchronisation dialog.
async fn fetch_params(
    client: &BankClient,
    creds: &Credentials,
    state: Option<&SessionState>,
    notices: &mut Vec<Notice>,
) -> Result<(BankParams, SystemId)> {
    let stored = state.map(|s| &s.system_id).filter(|s| s.is_assigned());
    if let Some(sid) = stored {
        match client
            .new_dialog(creds)?
            .with_system_id(sid)
            .probe_parameters()
            .await
        {
            Ok(found) => {
                info!("[FinTS] Reusing the stored system ID; no sync dialog");
                return Ok(found);
            }
            Err(e) if matches!(e.kind(), Category::Credentials(_) | Category::SystemId) => {
                return Err(e);
            }
            Err(e) => warn!(
                "[FinTS] Parameter probe with the stored system ID failed ({e}); synchronizing"
            ),
        }
    }
    let mut sync = client.new_dialog(creds)?;
    if let Some(sid) = stored {
        sync = sync.with_system_id(sid);
    }
    let (mut synced, _) = sync.sync().await?;
    notices.extend(synced.take_notices());
    let (params, system_id) = synced.end().await?;
    let system_id = if system_id.is_assigned() {
        system_id
    } else {
        stored.cloned().unwrap_or_else(SystemId::unassigned)
    };
    Ok((params, system_id))
}

/// The balance of `account` (HKSAL). `None` when the bank answers with
/// success but without HISAL (3010 "no entries", which is a notice).
pub async fn balance(
    dialog: &mut Dialog,
    account: &Account,
) -> Result<Step<Option<AccountBalance>>> {
    dialog
        .start(JobInput::Balance {
            account: account.clone(),
        })
        .await?
        .into_step()
}

/// The transactions of `account` booked in `range`: all touchdown pages,
/// in one call. The format follows `format` ([`Format::Auto`] prefers
/// camt.052); a TAN can be required on any page, and the job then pauses
/// with the pages so far. A failure on a later page fails the job and drops
/// the pages: retry with a narrower range.
///
/// A `range.from` before today minus the bank's storage period
/// ([`Support::Available`]'s `days_stored`) is raised to that day, and the
/// notices of the step carry [`ClientWarning::RangeClamped`]. Today is the
/// system's local date unless the client pins it ([`BankClient::with_now`]).
///
/// Fails with [`FinTSError::NoBpd`] or [`FinTSError::VersionMismatch`]
/// (for the chosen format) before anything is sent. A UPD that does not
/// list the order never blocks it.
pub async fn transactions(
    dialog: &mut Dialog,
    account: &Account,
    range: Range,
    format: Format,
) -> Result<Step<Statement>> {
    let today = dialog.inner.now().date();
    transactions_on(dialog, account, range, format, today).await
}

async fn transactions_on(
    dialog: &mut Dialog,
    account: &Account,
    range: Range,
    format: Format,
    today: Date,
) -> Result<Step<Statement>> {
    let format = choose_format(dialog.bank_params(), account, format)?;
    let order = match format {
        StatementFormat::Mt940 => Order::Transactions,
        StatementFormat::Camt => Order::Camt,
    };
    let days = dialog.bank_params().days_stored(order);
    let (range, clamped) = clamp_range(range, today, days);
    let notices = clamped
        .map(|requested| {
            let warning = ClientWarning::RangeClamped {
                requested,
                used: range.from,
            };
            warn!("[FinTS] {warning}");
            Notice::client(NoticeDialog::Job, warning)
        })
        .into_iter()
        .collect();
    dialog
        .start_with(
            JobInput::Transactions {
                account: account.clone(),
                range,
                format,
                touchdown: None,
                booked: Vec::new(),
                pending: Vec::new(),
                pending_sent: false,
                find_payment: None,
            },
            notices,
        )
        .await?
        .into_step()
}

/// `range` with `from` raised to `today - days_stored`; the second value
/// is the `from` that was asked for when it was raised. `to` is raised
/// along when the whole range lies before the storage period.
fn clamp_range(range: Range, today: Date, days_stored: Option<u32>) -> (Range, Option<Date>) {
    let Some(oldest) = days_stored
        .and_then(|d| today.checked_sub(jiff::Span::new().days(i64::from(d))).ok())
        .filter(|oldest| range.from < *oldest)
    else {
        return (range, None);
    };
    (Range::new(oldest, range.to.max(oldest)), Some(range.from))
}

/// The statement format to ask for, decided from the BPD and UPD.
fn choose_format(
    params: &BankParams,
    account: &Account,
    wanted: Format,
) -> Result<StatementFormat> {
    choose_statement_format(params, wanted, || upd_allowance(params, account, "HKCAZ"))
}

/// [`choose_format`] with the UPD's word on HKCAZ given lazily, so that
/// [`Capabilities`] (which knows the UPD account) decides the same way.
fn choose_statement_format(
    params: &BankParams,
    wanted: Format,
    camt_allowance: impl FnOnce() -> Allowance,
) -> Result<StatementFormat> {
    match wanted {
        Format::Camt => Ok(StatementFormat::Camt),
        Format::Mt940 => Ok(StatementFormat::Mt940),
        Format::Auto => match params.order_version(Order::Camt) {
            Ok(_) if camt_allowance() == Allowance::No => {
                info!("[FinTS] The UPD does not list HKCAZ for the account; using MT940");
                Ok(StatementFormat::Mt940)
            }
            Ok(_) => Ok(StatementFormat::Camt),
            Err(FinTSError::VersionMismatch(why)) => {
                info!("[FinTS] camt not available ({why}); using MT940");
                Ok(StatementFormat::Mt940)
            }
            Err(e) => Err(e),
        },
    }
}

/// The UPD account that `account` names ([`Upd::find`]: by key,
/// subaccount included).
fn find_upd_account<'a>(params: &'a BankParams, account: &Account) -> Option<&'a UpdAccount> {
    params.upd()?.find(account)
}

/// What the UPD says about `code` for `account`; `Unknown` when the UPD
/// does not name the account.
fn upd_allowance(params: &BankParams, account: &Account, code: &str) -> Allowance {
    match (params.upd(), find_upd_account(params, account)) {
        (Some(upd), Some(a)) => upd.allows(a, code),
        _ => Allowance::Unknown,
    }
}

/// The securities in the depot `account` (HKWPD, MT535): all touchdown
/// pages. `currency` asks for the valuation currency.
pub async fn holdings(
    dialog: &mut Dialog,
    account: &Account,
    currency: Option<Currency>,
) -> Result<Step<Vec<SecurityHolding>>> {
    dialog
        .start(JobInput::Holdings {
            account: account.clone(),
            currency,
            touchdown: None,
            found: Vec::new(),
        })
        .await?
        .into_step()
}

/// The securities movements of the depot `account` (HKWDU, MT536): all
/// touchdown pages. Without `range` the bank chooses the period.
pub async fn depot_transactions(
    dialog: &mut Dialog,
    account: &Account,
    range: Option<Range>,
) -> Result<Step<Vec<DepotTransaction>>> {
    dialog
        .start(JobInput::DepotTransactions {
            account: account.clone(),
            range,
            touchdown: None,
            found: Vec::new(),
        })
        .await?
        .into_step()
}

impl Dialog {
    /// Where the dialog is.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The paused job, if any.
    pub fn pending(&self) -> Option<&PendingJob> {
        match &self.phase {
            Phase::Paused(p) => Some(p.as_ref()),
            _ => None,
        }
    }

    /// Wall clock of the bank's last response.
    pub fn last_message_at(&self) -> Timestamp {
        self.last_message_at
    }

    /// What the bank offers per account (`accounts`, or the UPD's accounts
    /// when empty), from this dialog's BPD and UPD, the learned TAN codes
    /// and the client's TAN overrides. Sends nothing.
    pub fn capabilities(&self, accounts: &[Account]) -> Capabilities {
        Capabilities::build(self.bank_params(), self.inner.tan_overrides(), accounts)
    }

    /// The bank and user parameters as the dialog knows them now.
    #[cfg(feature = "tooling")]
    pub fn bank_params(&self) -> &BankParams {
        self.inner.bank_params()
    }

    #[cfg(not(feature = "tooling"))]
    pub(crate) fn bank_params(&self) -> &BankParams {
        self.inner.bank_params()
    }

    /// Send the TAN for the paused job (HKTAN process 2, PIN/TAN B.4.2),
    /// for the login and for any order. The bank's confirmation carries
    /// the order's data; if it does not, a read order is sent once more (a
    /// transfer never). A transfer whose payee check the bank wants
    /// released sends HKVPA with its first TAN message.
    ///
    /// A wrong TAN the bank rejects with 9941 alone (the order kept, the
    /// dialog open) gives `NeedTan` again for the same order, with
    /// [`TanChallenge::rejected`] set: show the bank's text and ask again,
    /// or [`Dialog::cancel`]. Nothing is resent unless the caller submits
    /// another TAN. Any other rejection fails the job (the order deleted,
    /// the access locked, the dialog ended): an order's dialog stays open
    /// unless the bank ended it, a login's dialog ends. On a transfer
    /// every error but a class 9 rejection is
    /// [`FinTSError::PaymentOutcomeUnknown`].
    ///
    /// Not cancel-safe (see [`Dialog`]).
    pub async fn submit_tan<T: JobValue>(
        &mut self,
        resume: Resume<T>,
        tan: &str,
    ) -> Result<Step<T>> {
        let pending = self.pending_for(&resume)?;
        match &pending.wait {
            Wait::Tan(c) if matches!(c.kind, TanKind::Decoupled { .. }) => {
                return Err(FinTSError::Dialog(
                    "decoupled challenge: confirm in the app and poll".into(),
                ));
            }
            Wait::Tan(_) => {}
            _ => {
                return Err(FinTSError::Dialog(
                    "the job waits for its payee check, not for a TAN".into(),
                ));
            }
        }
        let mut pending = self.take_pending();
        let ids = pending.job.payment_ids();
        let outcome = async {
            let task_reference = pending.task_reference()?;
            let extra = pending.job.tan_extra();
            self.in_flight = Some(pending.job.in_flight());
            let response = self.inner.send_tan(&extra, &task_reference, tan).await;
            let response = self.settle(&pending.job, response)?;
            // HKVPA goes with the first TAN message only, as with a
            // decoupled approval.
            if let JobInput::Transfer(p) = &mut pending.job {
                p.tan_extra_sent();
            }
            if !matches!(self.phase, Phase::Ended)
                && let Some((code, attempts_left)) = response.tan_rejection()
                && let Wait::Tan(challenge) = &mut pending.wait
            {
                let count = challenge.rejected.as_ref().map_or(0, |r| r.count) + 1;
                warn!(
                    "[FinTS] TAN rejected ({} {}); the bank keeps the order for another TAN",
                    code.code, code.text
                );
                challenge.rejected = Some(Box::new(TanRejected {
                    code,
                    attempts_left,
                    count,
                }));
                extend_notices(&mut pending.notices, self.inner.take_notices());
                let outcome = Outcome::Paused(pending.wait.clone(), pending.ticket.clone());
                self.phase = Phase::Paused(Box::new(pending));
                return Ok(outcome);
            }
            self.confirmed(pending, response).await
        }
        .await;
        after_order(ids.as_ref(), outcome)?.into_step()
    }

    /// One decoupled status request for the paused job (HKTAN process S;
    /// process 2 before HKTAN 7). Still pending (3956): `NeedTan` again
    /// with `polls_done + 1` and a new `not_before`. Once the bank's
    /// `max_polls` are used up, [`FinTSError::DecoupledMaxPolls`] without
    /// sending, and the job is dropped; on a transfer that error is
    /// [`FinTSError::PaymentOutcomeUnknown`] (the bank may still execute
    /// it), as is every error of a transfer's status request but a class 9
    /// rejection. The library does not wait; sleep `first_wait` /
    /// `next_wait` before calling (or until `not_before`). When the
    /// challenge's `auto_polling_allowed` is `Some(false)` the caller must
    /// not poll automatically: the bank expects the user to trigger each
    /// status request. `poll` itself does not check it.
    ///
    /// On a transfer's unfinished payee check ([`Step::VopPending`]) it
    /// sends the next HKVPP instead; HKVPA goes with a transfer's first
    /// decoupled status request when the bank wants the check released.
    ///
    /// Not cancel-safe (see [`Dialog`]).
    pub async fn poll<T: JobValue>(&mut self, resume: Resume<T>) -> Result<Step<T>> {
        let pending = self.pending_for(&resume)?;
        let (max_polls, polls_done) = match &pending.wait {
            Wait::VopPoll(_) => {
                let pending = self.take_pending();
                return self.vop_poll(pending).await?.into_step();
            }
            Wait::VopConfirmation(_) => {
                return Err(FinTSError::Dialog(
                    "payee check result: confirm_vop or cancel".into(),
                ));
            }
            Wait::Tan(TanChallenge {
                kind:
                    TanKind::Decoupled {
                        max_polls,
                        polls_done,
                        ..
                    },
                ..
            }) => (*max_polls, *polls_done),
            Wait::Tan(_) => {
                return Err(FinTSError::Dialog(
                    "typed TAN challenge: submit the TAN".into(),
                ));
            }
        };
        let mut pending = self.take_pending();
        let ids = pending.job.payment_ids();
        if let Some(max) = max_polls.filter(|max| polls_done >= *max) {
            self.job_failed(&pending.job);
            return Err(match ids {
                Some(ids) => {
                    warn!("[FinTS] Transfer approval not confirmed within {max} status requests");
                    ids.unknown()
                }
                None => FinTSError::DecoupledMaxPolls(max),
            });
        }
        let outcome = async {
            let task_reference = pending.task_reference()?;
            let extra = pending.job.tan_extra();
            self.in_flight = Some(pending.job.in_flight());
            let response = self.inner.send_tan_status(&extra, &task_reference).await;
            let response = self.settle(&pending.job, response)?;
            if let JobInput::Transfer(p) = &mut pending.job {
                p.tan_extra_sent();
            }
            for c in response.all_codes() {
                info!("[FinTS] Poll: {} - {}", c.code, c.text);
            }
            if response.is_decoupled_pending() {
                extend_notices(&mut pending.notices, self.inner.take_notices());
                if let Wait::Tan(challenge) = &mut pending.wait
                    && let TanKind::Decoupled {
                        polls_done,
                        next_wait,
                        ..
                    } = &mut challenge.kind
                {
                    *polls_done += 1;
                    let next_wait = *next_wait;
                    challenge.not_before = not_before_after(Timestamp::now(), next_wait);
                }
                let outcome = Outcome::Paused(pending.wait.clone(), pending.ticket.clone());
                self.phase = Phase::Paused(Box::new(pending));
                return Ok(outcome);
            }
            self.confirmed(pending, response).await
        }
        .await;
        after_order(ids.as_ref(), outcome)?.into_step()
    }

    /// Drop the paused job without sending anything. After an order the
    /// dialog is open for other jobs; after the login it has ended
    /// ([`Dialog::end`] still sends HKEND).
    ///
    /// The result says what is left at the bank. A read, the login and a
    /// transfer at its payee check are [`Cancelled::NothingSent`]: the bank
    /// lets the task expire (and a transfer's VOP-ID: a corrected payee
    /// name is a new transfer). A transfer waiting for its TAN is
    /// [`Cancelled::OrderSubmitted`]: its order is at the bank, and a late
    /// approval in the app may still execute it.
    pub fn cancel<T: JobValue>(&mut self, resume: Resume<T>) -> Result<Cancelled> {
        self.pending_for(&resume)?;
        let pending = self.take_pending();
        if pending.job.job() == Job::Open {
            self.phase = Phase::Ended;
        }
        Ok(match &pending.job {
            JobInput::Transfer(p) => p.cancelled(&pending.wait),
            _ => Cancelled::NothingSent,
        })
    }

    /// End the dialog (HKEND, unless the bank already ended it) and return
    /// the updated session state: system ID, BPD/UPD, TAN method, codes
    /// learned from 9075. A paused dialog ends the same way. When a
    /// transfer's request was dropped mid-flight, it fails with
    /// [`FinTSError::PaymentOutcomeUnknown`] instead and sends nothing.
    pub async fn end(mut self) -> Result<SessionState> {
        self.check_in_flight()?;
        self.inner.end_quietly().await;
        Ok(self.capture_state())
    }

    /// A snapshot of the session state as it is now: system ID, BPD/UPD,
    /// TAN method, codes learned from 9075. It sends nothing and changes
    /// nothing, so the dialog stays usable.
    ///
    /// The host stores it after [`open`] and after each job. A crash then
    /// keeps a freshly assigned system ID and the updated BPD/UPD, which
    /// [`Dialog::end`] would only hand over at the end. The snapshot is the
    /// state [`Dialog::end`] returns, minus whatever the dialog changes
    /// later.
    ///
    /// It also works while a transfer's request is in flight (a dropped
    /// future): it takes `&self` and sends nothing, and the pending
    /// [`FinTSError::PaymentOutcomeUnknown`] stays for the next call that
    /// sends, [`Dialog::end`] or [`Dialog::freeze`].
    pub fn session_state(&self) -> SessionState {
        self.capture_state()
    }

    fn capture_state(&self) -> SessionState {
        SessionState::capture(
            self.inner.blz(),
            self.inner.user_id(),
            self.inner.system_id(),
            self.inner.bank_params(),
        )
    }

    // ── Internals ──────────────────────────────────────────────────────

    fn opened(&self) -> Opened {
        let p = self.inner.bank_params();
        Opened {
            accounts: p
                .upd()
                .map(|u| AccountInfo::from_upd(u, &self.bic))
                .unwrap_or_default(),
            tan_methods: p.tan_methods.clone(),
            allowed_security_functions: p.allowed_security_functions.clone(),
            selected_tan_method: p.selected_tan_method().cloned(),
        }
    }

    /// A request was dropped mid-flight: the message number is unknown,
    /// so the dialog ends. When the request carried a payment, its outcome
    /// is unknown too: this call says so, the next one finds the dialog
    /// ended.
    fn check_in_flight(&mut self) -> Result<()> {
        let Some(in_flight) = self.in_flight.take() else {
            return Ok(());
        };
        warn!("[FinTS] A request was dropped mid-flight; the dialog has ended");
        self.end_locally();
        match in_flight {
            InFlight::Message => Ok(()),
            InFlight::Payment(ids) => Err(ids.unknown()),
        }
    }

    /// The dialog is over and the bank will not take another message in it.
    fn end_locally(&mut self) {
        self.phase = Phase::Ended;
        self.inner.forget_dialog();
    }

    /// Check before a new job: open, not paused, not ended.
    fn ready(&mut self) -> Result<()> {
        self.check_in_flight()?;
        match self.phase {
            Phase::Open => Ok(()),
            Phase::Paused(_) => Err(FinTSError::DialogPaused),
            Phase::Ended => Err(FinTSError::DialogEnded),
        }
    }

    /// The pending job `resume` belongs to.
    fn pending_for<T: Sealed>(&mut self, resume: &Resume<T>) -> Result<&PendingJob> {
        self.check_in_flight()?;
        match &self.phase {
            Phase::Paused(p) if p.ticket == resume.ticket && p.job.job() == T::JOB => Ok(p),
            Phase::Ended => Err(FinTSError::DialogEnded),
            _ => Err(FinTSError::WrongResume),
        }
    }

    fn take_pending(&mut self) -> PendingJob {
        match std::mem::replace(&mut self.phase, Phase::Open) {
            Phase::Paused(p) => *p,
            other => {
                self.phase = other;
                unreachable!("take_pending after pending_for")
            }
        }
    }

    /// A job failed: an order leaves the dialog open, a failed login ends it.
    fn job_failed(&mut self, job: &JobInput) {
        if job.job() == Job::Open {
            self.phase = Phase::Ended;
        }
    }

    /// After a request: record the response time, end the dialog when the
    /// bank did (9800) or the message state is unknown (transport error,
    /// unreadable answer).
    fn settle(&mut self, job: &JobInput, sent: Result<Response>) -> Result<Response> {
        self.settle_message(job.job() == Job::Open, sent)
    }

    /// [`Dialog::settle`] for any message; `login` when a failure ends
    /// the dialog.
    fn settle_message(&mut self, login: bool, sent: Result<Response>) -> Result<Response> {
        self.in_flight = None;
        match sent {
            Ok(response) => {
                self.last_message_at = Timestamp::now();
                if response
                    .all_codes()
                    .any(|c| c.kind() == Kind::DialogAborted)
                {
                    self.end_locally();
                }
                Ok(response)
            }
            Err(e) => {
                if matches!(
                    e,
                    FinTSError::Transport { .. }
                        | FinTSError::Timeout
                        | FinTSError::Http { .. }
                        | FinTSError::Parse(_)
                        | FinTSError::InvalidResponse(_)
                        | FinTSError::Io(_)
                ) {
                    self.end_locally();
                } else if login {
                    self.phase = Phase::Ended;
                }
                Err(e)
            }
        }
    }

    /// Pause on `challenge`; returns the `NeedTan` outcome.
    fn pause(
        &mut self,
        job: JobInput,
        challenge: &protocol::TanChallenge,
        notices: Vec<Notice>,
    ) -> Outcome {
        let tan = TanChallenge::from_bank(challenge, self.inner.bank_params(), Timestamp::now());
        let ticket = Ticket::Task(challenge.task_reference.clone());
        self.pause_on(job, Wait::Tan(tan), ticket, notices)
    }

    /// Pause the job on `wait`; returns the matching outcome.
    fn pause_on(
        &mut self,
        job: JobInput,
        wait: Wait,
        ticket: Ticket,
        notices: Vec<Notice>,
    ) -> Outcome {
        let outcome = Outcome::Paused(wait.clone(), ticket.clone());
        self.phase = Phase::Paused(Box::new(PendingJob {
            ticket,
            wait,
            job,
            notices,
        }));
        outcome
    }

    /// Start a job on an open dialog.
    async fn start(&mut self, job: JobInput) -> Result<Outcome> {
        self.start_with(job, Vec::new()).await
    }

    /// [`start`](Self::start) with notices the client raised before
    /// sending.
    async fn start_with(&mut self, job: JobInput, notices: Vec<Notice>) -> Result<Outcome> {
        self.ready()?;
        // Nothing from earlier calls belongs to this job.
        self.inner.take_notices();
        self.send_order(job, notices).await
    }

    /// Send the job's order (with HKTAN as `order_tan` decides, and the
    /// 9075 retry) and handle the answer: a challenge pauses the job, an
    /// error fails it, a page with more to come sends the next one, else
    /// the job is done.
    async fn send_order(&mut self, mut job: JobInput, mut notices: Vec<Notice>) -> Result<Outcome> {
        loop {
            let Some((segment, code, _)) = job.order() else {
                return Err(FinTSError::Internal("the login is not an order".into()));
            };
            self.in_flight = Some(InFlight::Message);
            let response = self.inner.send_order(segment, code).await;
            let response = self.settle(&job, response)?;
            notices.extend(self.inner.take_notices());
            if let Some(challenge) = response.order_tan_challenge()? {
                return Ok(self.pause(job, &challenge, notices));
            }
            response.check_errors()?;
            if !job.absorb(
                &response,
                &self.inner.bank_params().camt052_format(),
                &mut notices,
            ) {
                return Ok(Outcome::Done(
                    job.finish(&response, self.booking_id)?,
                    notices,
                ));
            }
        }
    }

    /// The bank answered a TAN or a status request with something other
    /// than "still pending".
    async fn confirmed(&mut self, mut pending: PendingJob, response: Response) -> Result<Outcome> {
        extend_notices(&mut pending.notices, self.inner.take_notices());
        if let Err(e) = response.check_errors() {
            self.job_failed(&pending.job);
            return Err(e);
        }
        // A transfer ends with the answer to its TAN: never resent.
        if let JobInput::Transfer(p) = &pending.job {
            let receipt = p.receipt(&response, self.inner.bank_params(), self.inner.now().date());
            return Ok(Outcome::Done(Value::Transfer(receipt), pending.notices));
        }
        self.inner.ingest(&response);
        let PendingJob {
            mut job,
            mut notices,
            ..
        } = pending;
        match job.order() {
            None => Ok(Outcome::Done(Value::Opened(self.opened()), notices)),
            // 3010: "no entries", a complete answer without a data segment.
            Some((_, _, data))
                if response.find_segment(data).is_some()
                    || response.has_no_entries()
                    || (matches!(job, JobInput::InstantStatus { .. })
                        && response.has(Kind::InstantStatusQuery)) =>
            {
                if job.absorb(
                    &response,
                    &self.inner.bank_params().camt052_format(),
                    &mut notices,
                ) {
                    self.send_order(job, notices).await
                } else {
                    Ok(Outcome::Done(
                        job.finish(&response, self.booking_id)?,
                        notices,
                    ))
                }
            }
            Some((_, code, data)) => {
                info!("[FinTS] {code}: confirmation carried no {data}; sending the order again");
                self.send_order(job, notices).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn range_is_clamped_to_the_storage_period() {
        let today = date(2026, 11, 18);
        let r = |from, to| Range::new(from, to);
        // 60 days before 2026-11-18 is 2026-09-19.
        let (clamped, asked) = clamp_range(r(date(2026, 1, 1), today), today, Some(60));
        assert_eq!(clamped, r(date(2026, 9, 19), today));
        assert_eq!(asked, Some(date(2026, 1, 1)));
        // Inside the period, on its edge, no period known: untouched.
        for (from, days) in [
            (date(2026, 10, 1), Some(60)),
            (date(2026, 9, 19), Some(60)),
            (date(2000, 1, 1), None),
        ] {
            assert_eq!(
                clamp_range(r(from, today), today, days),
                (r(from, today), None)
            );
        }
        // A range wholly before the period becomes one day.
        let (clamped, _) = clamp_range(r(date(2020, 1, 1), date(2020, 2, 1)), today, Some(60));
        assert_eq!(clamped, r(date(2026, 9, 19), date(2026, 9, 19)));
    }

    /// A decoupled method asks for a typed TAN when the bank sends 0030
    /// without 3955 (PIN/TAN TAN-Prozess 4): only 3955 makes the challenge
    /// decoupled.
    #[test]
    fn only_3955_makes_a_challenge_decoupled() {
        use crate::types::{TanMethod, TanProcess};
        let mut params = BankParams::new();
        params.tan_methods = vec![TanMethod {
            security_function: SecurityFunction::new("901"),
            tan_process: TanProcess::TwoStep,
            name: "App".into(),
            needs_tan_medium: false,
            decoupled_max_polls: -1,
            wait_before_first_poll: 0,
            wait_before_next_poll: 0,
            is_decoupled: true,
            hktan_version: 7,
            zka_name: None,
            decoupled_auto_polling: None,
            challenge_structured: false,
        }];
        params.selected_security_function = SecurityFunction::new("901");
        let bank = |decoupled| protocol::TanChallenge {
            challenge: ChallengeText::new("TAN eingeben"),
            challenge_hhduc: None,
            task_reference: TaskReference::new("TASK-1"),
            decoupled,
            challenge_markup: false,
            valid_until: None,
        };
        let now = Timestamp::now();
        let typed = TanChallenge::from_bank(&bank(false), &params, now);
        assert!(matches!(typed.kind, TanKind::Typed { .. }), "{typed:?}");
        let app = TanChallenge::from_bank(&bank(true), &params, now);
        assert!(matches!(app.kind, TanKind::Decoupled { .. }), "{app:?}");
    }

    /// One booked camt entry with the entry reference `reference`.
    fn camt_entry(reference: &str) -> Vec<u8> {
        format!(
            "<Document><BkToCstmrAcctRpt><Rpt><Ntry><Amt Ccy=\"EUR\">1.00</Amt>\
             <CdtDbtInd>DBIT</CdtDbtInd><Sts><Cd>BOOK</Cd></Sts>\
             <BookgDt><Dt>2026-03-02</Dt></BookgDt>\
             <AcctSvcrRef>{reference}</AcctSvcrRef></Ntry></Rpt></BkToCstmrAcctRpt></Document>"
        )
        .into_bytes()
    }

    fn ids(txs: &[Transaction]) -> Vec<Option<&str>> {
        txs.iter().map(|t| t.booking_id.as_deref()).collect()
    }

    /// The profile rule reaches the booked camt part; the pending part gets
    /// no ID (its `Sts` says BOOK, the HICAZ field says pending), and a
    /// reference on two pages is dropped from both.
    #[test]
    fn statement_booking_ids_follow_the_rule_on_booked_camt_only() {
        let rule = BookingIdSource::CamtEntryAcctSvcrRef;
        let booked = [camt_entry("TESTREF-1"), camt_entry("TESTREF-2")];
        let pending = [camt_entry("TESTREF-3")];
        let s = parse_statement(StatementFormat::Camt, &booked, &pending, true, rule).unwrap();
        assert_eq!(ids(&s.booked), [Some("TESTREF-1"), Some("TESTREF-2")]);
        let Pending::Reported(p) = &s.pending else {
            panic!("{:?}", s.pending)
        };
        assert_eq!(ids(p), [None]);

        let generic = BookingIdSource::None;
        let s = parse_statement(StatementFormat::Camt, &booked, &[], false, generic).unwrap();
        assert_eq!(ids(&s.booked), [None, None]);

        let repeated = [camt_entry("TESTREF-1"), camt_entry("TESTREF-1")];
        let s = parse_statement(StatementFormat::Camt, &repeated, &[], false, rule).unwrap();
        assert_eq!(ids(&s.booked), [None, None]);
    }

    /// MT940 never gives a booking ID, also with a `:61:` bank reference
    /// and the camt rule of a profile.
    #[test]
    fn mt940_bookings_never_get_a_booking_id() {
        let mt940 = b":20:STARTUMS\r\n:25:12030000/0000000201\r\n:28C:1\r\n\
            :60F:C260301EUR100,00\r\n\
            :61:2603020302D1,00NTRFNONREF//TESTBANKREF1\r\n:86:177?20SVWZ+Test\r\n\
            :62F:C260302EUR99,00\r\n-"
            .to_vec();
        let rule = BookingIdSource::CamtEntryAcctSvcrRef;
        let s = parse_statement(StatementFormat::Mt940, &[mt940], &[], false, rule).unwrap();
        assert_eq!(s.booked.len(), 1);
        assert_eq!(s.booked[0].bank_reference.as_deref(), Some("TESTBANKREF1"));
        assert_eq!(s.booked[0].booking_id, None);
    }

    /// The paused job is plain data: it survives JSON unchanged.
    #[test]
    fn pending_job_round_trips_through_json() {
        let pending = PendingJob {
            ticket: Ticket::Task(TaskReference::new("TASK0001")),
            wait: Wait::Tan(TanChallenge {
                challenge: ChallengeText::new("Bitte in der App freigeben"),
                challenge_markup: false,
                kind: TanKind::Decoupled {
                    first_wait: Some(Duration::from_secs(5)),
                    next_wait: Some(Duration::from_secs(2)),
                    max_polls: Some(10),
                    polls_done: 3,
                    auto_polling_allowed: Some(false),
                },
                valid_until: None,
                not_before: Some(Timestamp::constant(1_740_830_405, 0)),
                payee_check: None,
                rejected: None,
            }),
            job: JobInput::Balance {
                account: Account::new("DE02120300000000202051", "BYLADEM1001").unwrap(),
            },
            notices: Vec::new(),
        };
        let json = serde_json::to_string(&pending).unwrap();
        let back: PendingJob = serde_json::from_str(&json).unwrap();
        assert_eq!(back.challenge(), pending.challenge());
        assert_eq!(back.ticket, pending.ticket);
        assert_eq!(back.job.job(), Job::Balance);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }
}
