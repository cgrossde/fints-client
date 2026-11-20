//! Return codes ("Rückmeldungen") of HIRMG and HIRMS, Formals 2017 B.7.
//!
//! A [`ReturnCode`] keeps everything the bank sent in one code line: the
//! four-digit code, its text, the reference element, the parameters, and
//! what it refers to ([`Scope`]: the whole message for HIRMG, one customer
//! segment for HIRMS). Its [`Class`] and [`Kind`] are derived from the
//! stored code and parameters, so a code this client does not know loses
//! nothing. Code meanings: Rückmeldungscodes 2026-02-03 (RM), B.1–B.4.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::{SecurityFunction, SegmentType, TouchdownPoint};

/// A four-digit return code (`dig 4`, Formals B.7.5.1), kept verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Code([u8; 4]);

impl Code {
    /// The code, if `s` is exactly four ASCII digits.
    pub fn new(s: &str) -> Option<Self> {
        let bytes: [u8; 4] = s.as_bytes().try_into().ok()?;
        bytes.iter().all(u8::is_ascii_digit).then_some(Self(bytes))
    }

    /// The four digits.
    pub fn as_str(&self) -> &str {
        // Four ASCII digits, checked in `new`.
        std::str::from_utf8(&self.0).unwrap_or("????")
    }

    /// The class, from the first digit (Formals B.7.5.1).
    pub fn class(self) -> Class {
        match self.0[0] {
            b'0' => Class::Success,
            b'1' => Class::Note,
            b'3' => Class::Warning,
            b'9' => Class::Error,
            _ => Class::Undefined,
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for Code {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::new(&s).ok_or_else(|| format!("not a four-digit return code: {s:?}"))
    }
}

impl From<Code> for String {
    fn from(c: Code) -> String {
        c.as_str().to_string()
    }
}

/// The class of a return code: its first digit (Formals B.7.5.1, RM B.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// 0xxx: success.
    Success,
    /// 1xxx: note (defined for FinTS 4 only).
    Note,
    /// 3xxx: warning; never fails an order.
    Warning,
    /// 9xxx: error; the message or order was rejected.
    Error,
    /// Any other first digit: not defined by the specification.
    Undefined,
}

/// What a return code refers to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// HIRMG: the whole message (Formals B.7.2).
    Message,
    /// HIRMS: one segment of the customer message (Formals B.7.3).
    Segment {
        /// The customer segment's number, from the HIRMS header's
        /// reference ("Bezugssegment"); 0 if the bank sent none.
        number: u16,
        /// The code of that segment (e.g. `HKSAL`), when the sent message
        /// is known.
        order: Option<SegmentType>,
    },
}

/// The meaning of a return code this client knows, derived from the code
/// and its parameters (RM). [`Kind::Other`] for every code not listed; its
/// code, class, text and parameters stay on the [`ReturnCode`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// 0010: message received.
    MessageReceived,
    /// 0020: order executed.
    OrderExecuted,
    /// 0030: order received, security clearance (TAN) required.
    TanRequired,
    /// 0100: dialog ended.
    DialogEnded,
    /// 0900: TAN valid.
    TanValid,
    /// 0025: payee check without name deviation (VoP).
    NoNameDeviation,
    /// 3020: the BIC given is outdated; P1, if present, is the new one.
    BicOutdated { new_bic: Option<String> },
    /// 3045: instant payment status unknown yet; ask with HKIPS.
    InstantStatusQuery,
    /// 3046: check the account's transactions (outcome to be verified).
    CheckTransactions,
    /// 3070: order accepted for processing; P1, if present, is the bank's
    /// reference.
    ProcessingReference { reference: Option<String> },
    /// 3010: not available. RM's examples: no new entries, information
    /// not offered at the moment, data already up to date. After a data
    /// request banks send it for "no entries".
    NotAvailable,
    /// 3040: more data available; P1 is the touchdown point.
    Touchdown(TouchdownPoint),
    /// 3060: some warnings in the message.
    PartialWarnings,
    /// 3072: new access data (RM C.6); P1 user ID, P2 customer ID (the
    /// user ID when absent).
    UserDataChanged {
        user_id: Option<String>,
        customer_id: Option<String>,
    },
    /// 3076: no strong customer authentication required.
    ScaExemption,
    /// 3090: check the payee check result (a deviation or a note).
    VopCheckResult,
    /// 3091: Verification of Payee not required.
    VopNotRequired,
    /// 3092: the bank will require Verification of Payee from P1 on.
    VopRequiredFrom { date: Option<String> },
    /// 3093: payee check still in progress (poll with HKVPP).
    VopInProgress,
    /// 3094: payee check complete.
    VopComplete,
    /// 3095: payee check without result.
    VopNoResult,
    /// 3270: the instant payment is processed as a standard SEPA transfer
    /// (Messages C.10.2.9.1.2, HKIPZ 2 with the conversion flag).
    ConvertedToStandard,
    /// 3920: allowed security functions, P1..n.
    AllowedSecurityFunctions(Vec<SecurityFunction>),
    /// 3945: Verification of Payee approval not possible.
    VopApprovalNotPossible,
    /// 3955: security clearance via a separate channel (decoupled)
    /// initiated.
    DecoupledInitiated,
    /// 3956: strong customer authentication (decoupled) still pending.
    DecoupledPending,
    /// 3957: wait for the push message (decoupled). A notice for the user;
    /// as the answer to a status request without 0020 it means the same as
    /// 3956: still pending.
    DecoupledAwaitPush,
    /// 3958: the approval app does not support the chosen procedure.
    DecoupledMethodUnsupported,
    /// 9000: status indifferent: it is unclear whether the message or
    /// order was processed (RM B.4, C.7). P1 names the order (segment
    /// code) to run to find out, e.g. `HKCAZ`. Never resubmit blindly.
    OutcomeUnknown { query: Option<SegmentType> },
    /// 9010: processing not possible.
    GeneralError,
    /// 9040: authentication of the submitter missing or faulty, order
    /// rejected. RM also lists 9040 for a decompression error, which this
    /// client cannot cause (it sends no compressed messages).
    AuthenticationFailed,
    /// 9050: message partially faulty.
    PartialErrors,
    /// 9075: dialog aborted, strong customer authentication required.
    ScaRequired,
    /// 9076: name check (Verification of Payee) required; the bank no
    /// longer accepts payments without HKVPP/HKVPA or HKVOO.
    VopRequired,
    /// 9078: dialog aborted, the FinTS product is not registered.
    ProductNotRegistered,
    /// 9110: unknown structure.
    UnknownStructure,
    /// 9150: element not allowed in this order (e.g. an execution date
    /// other than 1999-01-01 in a FinTS single order).
    ElementNotAllowed,
    /// 9230: insufficient funds.
    InsufficientFunds,
    /// 9390: duplicate submission; the order is not executed again.
    DuplicateSubmission,
    /// 9120: not expected.
    NotExpected,
    /// 9160: data element missing.
    DataElementMissing,
    /// 9180: no longer accepted (order version outdated).
    NoLongerAccepted,
    /// 9185: HBCI/FinTS version not supported.
    ProtocolVersionUnsupported,
    /// 9250: name check (Verification of Payee) not possible; submit again
    /// later or with opt-out (HKVOO).
    VopUnavailable,
    /// 9255: real-time (instant payment) limit exceeded. P1, if present,
    /// is a link where the user can change the limit.
    InstantLimitExceeded { link: Option<String> },
    /// 9311: PIN/TAN system not available.
    PinTanUnavailable,
    /// 9361: dialog aborted, the security procedure is no longer allowed.
    SecurityProcedureNotAllowed,
    /// 9370: signature authorisation not sufficient.
    SignatureInsufficient,
    /// 9380: user not authorised (for the order, or for the chosen TAN
    /// procedure).
    NotAuthorised,
    /// 9340: signature not accepted (RM: "electronic signature wrong",
    /// "signature faulty", "security profile unknown"). The PIN may be
    /// wrong, or user ID, BLZ or security profile do not match; banks send
    /// it next to a more specific code, which then names the cause.
    SignatureInvalid,
    /// 9942, 9910: PIN wrong. 9942 also means, in a PIN change, "new PIN
    /// invalid / too short / too long"; RM lists 9910 also as "PIN
    /// locked", "TAN list locked", "chip card locked".
    PinWrong,
    /// 9391: fetch a new customer system ID.
    SystemIdInvalid,
    /// 9800: dialog aborted by the bank.
    DialogAborted,
    /// 9930, 9931: PIN or access locked.
    AccountLocked,
    /// 9360 (RM): the TAN list (or signature) is locked after
    /// `attempts_left` further failed attempts (P1). Without P1 it is the
    /// RM text "key locked": the lock happened.
    LockAfterAttempts { attempts_left: Option<u32> },
    /// 9941: TAN (or signature) invalid.
    TanInvalid,
    /// 9943: TAN already used.
    TanUsed,
    /// 9951: timeout in the two-step procedure, TAN invalid.
    TanExpired,
    /// 9959: the SMS could not be sent; repeat.
    SmsNotSent,
    /// Any other code.
    Other,
}

impl Kind {
    /// The kind of `code` with these parameters. Empty parameters are
    /// skipped.
    pub fn of(code: Code, parameters: &[String]) -> Self {
        let params: Vec<&String> = parameters.iter().filter(|p| !p.is_empty()).collect();
        match code.as_str() {
            "0010" => Self::MessageReceived,
            "0020" => Self::OrderExecuted,
            "0030" => Self::TanRequired,
            "0100" => Self::DialogEnded,
            "0900" => Self::TanValid,
            "0025" => Self::NoNameDeviation,
            "3020" => Self::BicOutdated {
                new_bic: params.first().map(|s| s.to_string()),
            },
            "3045" => Self::InstantStatusQuery,
            "3046" => Self::CheckTransactions,
            "3070" => Self::ProcessingReference {
                reference: params.first().map(|s| s.to_string()),
            },
            "3090" => Self::VopCheckResult,
            "3092" => Self::VopRequiredFrom {
                date: params.first().map(|s| s.to_string()),
            },
            "3093" => Self::VopInProgress,
            "3094" => Self::VopComplete,
            "3095" => Self::VopNoResult,
            "3270" => Self::ConvertedToStandard,
            "9150" => Self::ElementNotAllowed,
            "9230" => Self::InsufficientFunds,
            "9390" => Self::DuplicateSubmission,
            "3010" => Self::NotAvailable,
            "3040" => Self::Touchdown(TouchdownPoint::new(
                params.first().map(|s| s.as_str()).unwrap_or(""),
            )),
            "3060" => Self::PartialWarnings,
            "3072" => Self::UserDataChanged {
                user_id: params.first().map(|s| s.to_string()),
                customer_id: params.get(1).or(params.first()).map(|s| s.to_string()),
            },
            "3076" => Self::ScaExemption,
            "3091" => Self::VopNotRequired,
            "3920" => Self::AllowedSecurityFunctions(
                params.into_iter().map(SecurityFunction::new).collect(),
            ),
            "3945" => Self::VopApprovalNotPossible,
            "3955" => Self::DecoupledInitiated,
            "3956" => Self::DecoupledPending,
            "3957" => Self::DecoupledAwaitPush,
            "3958" => Self::DecoupledMethodUnsupported,
            "9000" => Self::OutcomeUnknown {
                query: params.first().map(|s| SegmentType::new(s.as_str())),
            },
            "9010" => Self::GeneralError,
            "9040" => Self::AuthenticationFailed,
            "9050" => Self::PartialErrors,
            "9075" => Self::ScaRequired,
            "9076" => Self::VopRequired,
            "9078" => Self::ProductNotRegistered,
            "9110" => Self::UnknownStructure,
            "9120" => Self::NotExpected,
            "9160" => Self::DataElementMissing,
            "9180" => Self::NoLongerAccepted,
            "9185" => Self::ProtocolVersionUnsupported,
            "9250" => Self::VopUnavailable,
            "9255" => Self::InstantLimitExceeded {
                link: params.first().map(|s| s.to_string()),
            },
            "9311" => Self::PinTanUnavailable,
            "9360" => Self::LockAfterAttempts {
                attempts_left: params.first().and_then(|p| p.trim().parse().ok()),
            },
            "9361" => Self::SecurityProcedureNotAllowed,
            "9370" => Self::SignatureInsufficient,
            "9380" => Self::NotAuthorised,
            "9340" => Self::SignatureInvalid,
            "9910" | "9942" => Self::PinWrong,
            "9391" => Self::SystemIdInvalid,
            "9800" => Self::DialogAborted,
            "9930" | "9931" => Self::AccountLocked,
            "9941" => Self::TanInvalid,
            "9943" => Self::TanUsed,
            "9951" => Self::TanExpired,
            "9959" => Self::SmsNotSent,
            _ => Self::Other,
        }
    }
}

/// What a caller can do about an error: one category per different
/// reaction (docs/return-codes-design.md section 2). Every
/// [`FinTSError`](crate::FinTSError) has one
/// ([`FinTSError::kind`](crate::FinTSError::kind)); a bank error's comes
/// from its selected [`ReturnCode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Category {
    /// 9942, 9910, 9930, 9931, 9340: the PIN is wrong, the access is
    /// locked, or the bank did not accept the signature. Ask the user;
    /// never retry on your own (a few wrong PINs in a row lock the login).
    Credentials(Credentials),
    /// 9391: the bank rejected the customer system ID; the client
    /// resynchronises once.
    SystemId,
    /// 9075: strong customer authentication required. The client retries
    /// with HKTAN; this reaches the caller only if that is impossible.
    ScaRequired,
    /// 9941, 9943, 9951, and a TAN the user did not give or confirm in
    /// time: get a new TAN from the user.
    Tan,
    /// 9800: the bank ended the dialog; open a new one.
    DialogEnded,
    /// 9076, 9078, 9180, 9185, 9361, and a version mismatch found before
    /// sending: update the client or use another order or procedure.
    Unsupported,
    /// 9040: the bank refused the submitter's authentication. Banks also
    /// send it when the TAN method is locked after too many challenges.
    /// Show the bank's text; do not open another dialog on your own (each
    /// login asks for a new challenge and can extend the lock). Retry later,
    /// when the user asks.
    AuthenticationRefused,
    /// 9370, 9380: the user may not run this order or use this account.
    NotPermitted,
    /// 9000: unclear whether the bank processed the order (RM C.7). Never
    /// resubmit; check the outcome another way ([`Kind::OutcomeUnknown`]
    /// names the order to ask with).
    OutcomeUnknown,
    /// 9250, 9311, 9959, transport errors, timeouts, HTTP 5xx/408/429: retry
    /// later unchanged.
    Temporary,
    /// 9110, 9120, 9130-9170, unreadable answers, and requests this client
    /// cannot build or was called wrongly for: a bug in this client or in
    /// its use; report it.
    ClientFault,
    /// The caller's data cannot be sent (an invalid transfer: SEPA
    /// charset, length, IBAN, BIC, amount). Found before contact; nothing
    /// was sent. Fix the data and retry.
    InvalidInput,
    /// Every other bank error (9010, 9050, 9210, ...): show code and text.
    Rejected,
}

/// Which credentials problem ([`Category::Credentials`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Credentials {
    /// 9942: PIN wrong.
    PinWrong,
    /// 9340: signature not accepted. The PIN may be wrong, or user ID, BLZ
    /// or security profile do not match; check them all.
    SignatureInvalid,
    /// 9930, 9931: PIN or access locked.
    Locked,
    /// 9910: RM lists it both as "PIN wrong" and as "PIN locked" / "TAN
    /// list locked"; the bank text tells the user which.
    Unclear,
}

impl Category {
    /// The category of a class 9 code; `None` for every other class.
    pub fn of(code: Code, kind: &Kind) -> Option<Self> {
        if code.class() != Class::Error {
            return None;
        }
        Some(match kind {
            Kind::PinWrong if code.as_str() == "9910" => Self::Credentials(Credentials::Unclear),
            Kind::PinWrong => Self::Credentials(Credentials::PinWrong),
            Kind::SignatureInvalid => Self::Credentials(Credentials::SignatureInvalid),
            Kind::AccountLocked
            | Kind::LockAfterAttempts {
                attempts_left: None,
            } => Self::Credentials(Credentials::Locked),
            Kind::LockAfterAttempts { .. } => Self::Tan,
            Kind::SystemIdInvalid => Self::SystemId,
            Kind::ScaRequired => Self::ScaRequired,
            Kind::TanInvalid | Kind::TanUsed | Kind::TanExpired => Self::Tan,
            Kind::DialogAborted => Self::DialogEnded,
            Kind::VopRequired
            | Kind::ProductNotRegistered
            | Kind::NoLongerAccepted
            | Kind::ProtocolVersionUnsupported
            | Kind::SecurityProcedureNotAllowed => Self::Unsupported,
            Kind::AuthenticationFailed => Self::AuthenticationRefused,
            Kind::SignatureInsufficient | Kind::NotAuthorised => Self::NotPermitted,
            Kind::OutcomeUnknown { .. } => Self::OutcomeUnknown,
            Kind::VopUnavailable | Kind::PinTanUnavailable | Kind::SmsNotSent => Self::Temporary,
            Kind::UnknownStructure | Kind::NotExpected => Self::ClientFault,
            // 9130-9170: syntax, length, occupancy, missing, too often.
            _ if ("9130".."9171").contains(&code.as_str()) => Self::ClientFault,
            _ => Self::Rejected,
        })
    }
}

/// One code line of HIRMG or HIRMS ("Rückmeldung"), with everything the
/// bank sent in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnCode {
    /// The four-digit code.
    pub code: Code,
    /// The bank's text, to be shown with the code (Formals B.7.5.1).
    pub text: String,
    /// The message (HIRMG) or the customer segment (HIRMS) it refers to.
    pub scope: Scope,
    /// "Bezugsdatenelement": the position of the data element the code
    /// refers to, if the bank named one (HIRMS only).
    pub reference_element: Option<String>,
    /// "Rückmeldungsparameter", verbatim and in order (up to 10). An empty
    /// parameter on the wire is an absent repetition and does not appear.
    pub parameters: Vec<String>,
}

impl ReturnCode {
    /// A code line without reference element and parameters.
    pub fn new(code: Code, text: impl Into<String>, scope: Scope) -> Self {
        Self {
            code,
            text: text.into(),
            scope,
            reference_element: None,
            parameters: Vec::new(),
        }
    }

    /// The same code line with these parameters.
    pub fn with_parameters(mut self, parameters: Vec<String>) -> Self {
        self.parameters = parameters;
        self
    }

    /// The same code line with this reference element.
    pub fn with_reference_element(mut self, element: impl Into<String>) -> Self {
        self.reference_element = Some(element.into());
        self
    }

    /// The class, from the first digit.
    pub fn class(&self) -> Class {
        self.code.class()
    }

    /// The typed meaning, derived from code and parameters.
    pub fn kind(&self) -> Kind {
        Kind::of(self.code, &self.parameters)
    }

    /// The error category of a class 9 code; `None` for other classes.
    pub fn category(&self) -> Option<Category> {
        Category::of(self.code, &self.kind())
    }

    /// True for a class 9 code.
    pub fn is_error(&self) -> bool {
        self.class() == Class::Error
    }
}

impl fmt::Display for ReturnCode {
    /// `9210: Keine gueltige Kontoverbindung`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.text)
    }
}

/// A message for the caller: a bank code that is neither success, error
/// nor consumed by the protocol (Formals B.7.5.1: show every bank message),
/// or a warning the client itself raised about an answer.
/// Examples of bank codes: 3050 (order version outdated), 3060, 3072,
/// 3075, 3010, 3957, 3958.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Notice {
    /// A code line the bank sent.
    Bank {
        /// The dialog whose answer carried the code. The sync dialog and
        /// the job's dialog may both carry 3060 or 3072, and each notice
        /// says which one it came from. Within one dialog, an order's
        /// rounds (check, execution, status requests) give an identical
        /// notice once.
        dialog: NoticeDialog,
        /// The code line as the bank sent it. Its `scope` says whether it
        /// answers the message (HIRMG) or a segment (HIRMS, with the
        /// segment code of the order it answers, e.g. `HKSAL`).
        code: ReturnCode,
    },
    /// A warning the client generated; the bank sent no code for it.
    Client {
        /// The dialog whose answer raised it.
        dialog: NoticeDialog,
        /// What the client noticed.
        warning: ClientWarning,
    },
}

/// What the client noticed about an answer without the bank saying so.
/// Never an error: the answer is used as decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ClientWarning {
    /// The free text of the answer (HIRMG/HIRMS) contains UTF-8 byte
    /// sequences although the answer is ISO 8859-1 (Formals B.1): umlauts
    /// read as `Ã¤`, `Ã¼`, `Ã¶`, `ÃŸ`, the euro sign as `â‚¬`. The text is
    /// shown as decoded. Found by a heuristic, once per answer.
    Utf8TextInLatin1,
    /// The requested statement range began before the bank's storage
    /// period (Speicherzeitraum of HIKAZS/HICAZS), so the request was sent
    /// from `used` instead of `requested`. Entries before `used` are not
    /// available from the bank.
    RangeClamped {
        requested: jiff::civil::Date,
        used: jiff::civil::Date,
    },
}

/// Which dialog of an operation a [`Notice`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeDialog {
    /// The synchronisation dialog that fetches parameters and system ID.
    Sync,
    /// The dialog the job (or the login) itself runs in.
    Job,
}

impl Notice {
    /// The notice for this bank code line, from this dialog.
    pub fn new(dialog: NoticeDialog, code: ReturnCode) -> Self {
        Self::Bank { dialog, code }
    }

    /// The notice for a client-side warning, from this dialog.
    pub fn client(dialog: NoticeDialog, warning: ClientWarning) -> Self {
        Self::Client { dialog, warning }
    }

    /// The dialog the notice came from.
    pub fn dialog(&self) -> NoticeDialog {
        match self {
            Self::Bank { dialog, .. } | Self::Client { dialog, .. } => *dialog,
        }
    }

    /// The bank's code line; `None` for a client warning.
    pub fn code(&self) -> Option<&ReturnCode> {
        match self {
            Self::Bank { code, .. } => Some(code),
            Self::Client { .. } => None,
        }
    }

    /// The typed meaning of the bank's code; `None` for a client warning.
    pub fn kind(&self) -> Option<Kind> {
        self.code().map(ReturnCode::kind)
    }

    /// The client warning; `None` for a bank code.
    pub fn client_warning(&self) -> Option<ClientWarning> {
        match self {
            Self::Client { warning, .. } => Some(*warning),
            Self::Bank { .. } => None,
        }
    }

    /// True for codes the protocol acts on itself; they are not notices.
    /// 3090, 3093 and 3094 only in an answer to HKVPP or HKVPA (payments
    /// design section 4); elsewhere they stay notices.
    pub(crate) fn is_consumed(code: &ReturnCode) -> bool {
        let payee_check = matches!(
            &code.scope,
            Scope::Segment { order: Some(o), .. } if matches!(o.as_str(), "HKVPP" | "HKVPA")
        );
        match code.kind() {
            Kind::TanRequired
            | Kind::Touchdown(_)
            | Kind::ScaExemption
            | Kind::AllowedSecurityFunctions(_)
            | Kind::DecoupledInitiated
            | Kind::DecoupledPending
            | Kind::VopNotRequired
            | Kind::VopApprovalNotPossible
            | Kind::InstantStatusQuery => true,
            Kind::VopCheckResult | Kind::VopInProgress | Kind::VopComplete => payee_check,
            _ => false,
        }
    }
}

impl fmt::Display for NoticeDialog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sync => "sync",
            Self::Job => "job",
        })
    }
}

impl fmt::Display for ClientWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8TextInLatin1 => f.write_str(
                "bank text looks like UTF-8 in an ISO 8859-1 answer (umlauts garbled); shown as decoded",
            ),
            Self::RangeClamped { requested, used } => write!(
                f,
                "statement range starts {requested}, before the bank's storage period; requested from {used}"
            ),
        }
    }
}

impl fmt::Display for Notice {
    /// `3060 [sync, HIRMG]: text`, or `3072 [job, HKTAN]: text` for a code
    /// that answers the order `HKTAN` (`HIRMS #3` if its segment is not
    /// known). A client warning reads `client [job]: text`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (dialog, code) = match self {
            Self::Bank { dialog, code } => (dialog, code),
            Self::Client { dialog, warning } => {
                return write!(f, "client [{dialog}]: {warning}");
            }
        };
        write!(f, "{} [{}, ", code.code, dialog)?;
        match &code.scope {
            Scope::Message => f.write_str("HIRMG")?,
            Scope::Segment {
                order: Some(order), ..
            } => write!(f, "{order}")?,
            Scope::Segment { number, .. } => write!(f, "HIRMS #{number}")?,
        }
        write!(f, "]: {}", code.text)
    }
}

/// True if `text` (one `char` per ISO 8859-1 byte) holds a complete,
/// valid UTF-8 multi-byte sequence read as Latin-1: a lead byte (0xC2-0xF4)
/// followed by the right number of continuation bytes (0x80-0xBF). Real
/// German text does not match: `Ã`, `â` and the other lead characters are
/// not followed by a control or symbol character there.
pub(crate) fn looks_like_utf8_as_latin1(text: &str) -> bool {
    let bytes: Vec<u8> = match text.chars().map(u8::try_from).collect() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mut i = 0;
    while i < bytes.len() {
        let len = match bytes[i] {
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => 1,
        };
        if len > 1
            && bytes
                .get(i..i + len)
                .is_some_and(|s| std::str::from_utf8(s).is_ok())
        {
            return true;
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod mojibake_tests {
    use super::looks_like_utf8_as_latin1 as detect;

    fn latin1(utf8: &str) -> String {
        utf8.bytes().map(char::from).collect()
    }

    #[test]
    fn utf8_read_as_latin1_is_found() {
        for s in ["Zahlung für Müller", "Straße", "Betrag in €", "schön"] {
            assert!(detect(&latin1(s)), "{s}");
        }
    }

    #[test]
    fn real_latin1_umlauts_are_not_found() {
        for s in [
            "Zahlung für Müller, Größe, Äpfel, schön, Straße",
            "ÄÖÜ äöü ß ÂÃ ÄÅ",
            "Überweisung ausgeführt",
            "plain ascii",
            "",
        ] {
            assert!(!detect(s), "{s}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(code: &str, params: &[&str]) -> (Class, Kind) {
        let params: Vec<String> = params.iter().map(|s| s.to_string()).collect();
        let c = ReturnCode::new(Code::new(code).unwrap(), "t", Scope::Message)
            .with_parameters(params.clone());
        assert_eq!(c.parameters, params, "{code}: parameters kept verbatim");
        (c.class(), c.kind())
    }

    /// Every code of the inventory (docs/return-codes-design.md section 1)
    /// with its class and kind (RM B.1-B.4).
    #[test]
    fn inventory() {
        use Class::*;
        let sf = |s: &str| SecurityFunction::new(s);
        let cases: Vec<(&str, &[&str], Class, Kind)> = vec![
            ("0010", &[], Success, Kind::MessageReceived),
            ("0020", &[], Success, Kind::OrderExecuted),
            ("0030", &[], Success, Kind::TanRequired),
            ("0100", &[], Success, Kind::DialogEnded),
            ("0900", &[], Success, Kind::TanValid),
            ("0999", &[], Success, Kind::Other),
            ("0025", &[], Success, Kind::NoNameDeviation),
            (
                "3020",
                &["NEWBDEFFXXX"],
                Warning,
                Kind::BicOutdated {
                    new_bic: Some("NEWBDEFFXXX".into()),
                },
            ),
            ("3045", &[], Warning, Kind::InstantStatusQuery),
            ("3046", &[], Warning, Kind::CheckTransactions),
            (
                "3070",
                &["REF-1"],
                Warning,
                Kind::ProcessingReference {
                    reference: Some("REF-1".into()),
                },
            ),
            ("3090", &[], Warning, Kind::VopCheckResult),
            (
                "3092",
                &["2026-10-09"],
                Warning,
                Kind::VopRequiredFrom {
                    date: Some("2026-10-09".into()),
                },
            ),
            ("3093", &[], Warning, Kind::VopInProgress),
            ("3094", &[], Warning, Kind::VopComplete),
            ("3095", &[], Warning, Kind::VopNoResult),
            ("3270", &[], Warning, Kind::ConvertedToStandard),
            ("9150", &[], Error, Kind::ElementNotAllowed),
            ("9230", &[], Error, Kind::InsufficientFunds),
            ("9390", &[], Error, Kind::DuplicateSubmission),
            ("3010", &[], Warning, Kind::NotAvailable),
            (
                "3040",
                &["TD1"],
                Warning,
                Kind::Touchdown(TouchdownPoint::new("TD1")),
            ),
            ("3060", &[], Warning, Kind::PartialWarnings),
            (
                "3072",
                &["new-user", "new-customer"],
                Warning,
                Kind::UserDataChanged {
                    user_id: Some("new-user".into()),
                    customer_id: Some("new-customer".into()),
                },
            ),
            (
                "3072",
                &["new-user"],
                Warning,
                Kind::UserDataChanged {
                    user_id: Some("new-user".into()),
                    customer_id: Some("new-user".into()),
                },
            ),
            (
                "3072",
                &[],
                Warning,
                Kind::UserDataChanged {
                    user_id: None,
                    customer_id: None,
                },
            ),
            ("3076", &[], Warning, Kind::ScaExemption),
            ("3091", &[], Warning, Kind::VopNotRequired),
            (
                "3920",
                &["912", "913", "940"],
                Warning,
                Kind::AllowedSecurityFunctions(vec![sf("912"), sf("913"), sf("940")]),
            ),
            ("3945", &[], Warning, Kind::VopApprovalNotPossible),
            ("3955", &[], Warning, Kind::DecoupledInitiated),
            ("3956", &[], Warning, Kind::DecoupledPending),
            ("3957", &[], Warning, Kind::DecoupledAwaitPush),
            ("3958", &[], Warning, Kind::DecoupledMethodUnsupported),
            ("3999", &["P"], Warning, Kind::Other),
            (
                "9000",
                &["HKCAZ"],
                Error,
                Kind::OutcomeUnknown {
                    query: Some(SegmentType::new("HKCAZ")),
                },
            ),
            ("9000", &[], Error, Kind::OutcomeUnknown { query: None }),
            ("9010", &[], Error, Kind::GeneralError),
            ("9040", &[], Error, Kind::AuthenticationFailed),
            ("9050", &[], Error, Kind::PartialErrors),
            ("9075", &[], Error, Kind::ScaRequired),
            ("9076", &[], Error, Kind::VopRequired),
            ("9078", &[], Error, Kind::ProductNotRegistered),
            ("9110", &[], Error, Kind::UnknownStructure),
            ("9120", &[], Error, Kind::NotExpected),
            ("9160", &[], Error, Kind::DataElementMissing),
            ("9180", &[], Error, Kind::NoLongerAccepted),
            ("9185", &[], Error, Kind::ProtocolVersionUnsupported),
            ("9250", &[], Error, Kind::VopUnavailable),
            (
                "9255",
                &["https://bank.example/limit"],
                Error,
                Kind::InstantLimitExceeded {
                    link: Some("https://bank.example/limit".into()),
                },
            ),
            (
                "9255",
                &[],
                Error,
                Kind::InstantLimitExceeded { link: None },
            ),
            ("9311", &[], Error, Kind::PinTanUnavailable),
            (
                "9360",
                &["2"],
                Error,
                Kind::LockAfterAttempts {
                    attempts_left: Some(2),
                },
            ),
            (
                "9360",
                &[],
                Error,
                Kind::LockAfterAttempts {
                    attempts_left: None,
                },
            ),
            ("9361", &[], Error, Kind::SecurityProcedureNotAllowed),
            ("9370", &[], Error, Kind::SignatureInsufficient),
            ("9380", &[], Error, Kind::NotAuthorised),
            ("9340", &[], Error, Kind::SignatureInvalid),
            ("9910", &[], Error, Kind::PinWrong),
            ("9942", &[], Error, Kind::PinWrong),
            ("9391", &[], Error, Kind::SystemIdInvalid),
            ("9800", &[], Error, Kind::DialogAborted),
            ("9930", &[], Error, Kind::AccountLocked),
            ("9931", &[], Error, Kind::AccountLocked),
            ("9941", &[], Error, Kind::TanInvalid),
            ("9943", &[], Error, Kind::TanUsed),
            ("9951", &[], Error, Kind::TanExpired),
            ("9959", &[], Error, Kind::SmsNotSent),
            // Seen at banks, no own kind.
            ("9210", &[], Error, Kind::Other),
            ("9999", &["A", "B"], Error, Kind::Other),
            // FinTS 4 notes and undefined first digits keep code and class.
            ("1234", &[], Note, Kind::Other),
            ("5000", &[], Undefined, Kind::Other),
        ];
        for (code, params, class, want) in cases {
            assert_eq!(kind(code, params), (class, want), "{code} {params:?}");
        }
    }

    /// The category table (docs/return-codes-design.md section 2).
    #[test]
    fn categories() {
        use Category::*;
        let cases: &[(&str, Option<Category>)] = &[
            ("0020", None),
            ("3040", None),
            ("3999", None),
            ("1234", None),
            ("5000", None),
            (
                "9340",
                Some(Credentials(super::Credentials::SignatureInvalid)),
            ),
            ("9942", Some(Credentials(super::Credentials::PinWrong))),
            ("9930", Some(Credentials(super::Credentials::Locked))),
            ("9931", Some(Credentials(super::Credentials::Locked))),
            ("9910", Some(Credentials(super::Credentials::Unclear))),
            ("9391", Some(SystemId)),
            ("9075", Some(ScaRequired)),
            ("9941", Some(Tan)),
            ("9943", Some(Tan)),
            ("9951", Some(Tan)),
            ("9800", Some(DialogEnded)),
            ("9076", Some(Unsupported)),
            ("9078", Some(Unsupported)),
            ("9180", Some(Unsupported)),
            ("9185", Some(Unsupported)),
            ("9360", Some(Credentials(super::Credentials::Locked))),
            ("9361", Some(Unsupported)),
            ("9370", Some(NotPermitted)),
            ("9380", Some(NotPermitted)),
            ("9000", Some(OutcomeUnknown)),
            ("9250", Some(Temporary)),
            ("9311", Some(Temporary)),
            ("9959", Some(Temporary)),
            ("9110", Some(ClientFault)),
            ("9120", Some(ClientFault)),
            ("9130", Some(ClientFault)),
            ("9140", Some(ClientFault)),
            ("9145", Some(ClientFault)),
            ("9150", Some(ClientFault)),
            ("9160", Some(ClientFault)),
            ("9170", Some(ClientFault)),
            ("9010", Some(Rejected)),
            ("9040", Some(AuthenticationRefused)),
            ("9050", Some(Rejected)),
            ("9171", Some(Rejected)),
            ("9210", Some(Rejected)),
            ("9230", Some(Rejected)),
            ("9390", Some(Rejected)),
            ("9255", Some(Rejected)),
            ("9999", Some(Rejected)),
        ];
        for (code, want) in cases {
            let c = ReturnCode::new(Code::new(code).unwrap(), "t", Scope::Message);
            assert_eq!(c.category(), *want, "{code}");
        }
        // 9360 with P1 announces a lock after further attempts: a TAN
        // problem, not yet a lock.
        let c = ReturnCode::new(Code::new("9360").unwrap(), "t", Scope::Message)
            .with_parameters(vec!["2".into()]);
        assert_eq!(c.category(), Some(Tan));
    }

    #[test]
    fn empty_parameters_are_kept_but_skipped_for_the_kind() {
        let c = ReturnCode::new(Code::new("3072").unwrap(), "t", Scope::Message)
            .with_parameters(vec!["".into(), "cust".into()]);
        assert_eq!(c.parameters, ["", "cust"]);
        assert_eq!(
            c.kind(),
            Kind::UserDataChanged {
                user_id: Some("cust".into()),
                customer_id: Some("cust".into()),
            }
        );
    }

    /// 3090, 3093, 3094 are consumed only in answers to HKVPP and HKVPA.
    #[test]
    fn payee_check_codes_are_consumed_only_for_hkvpp_and_hkvpa() {
        let on = |code: &str, order: &str| {
            ReturnCode::new(
                Code::new(code).unwrap(),
                "t",
                Scope::Segment {
                    number: 3,
                    order: Some(SegmentType::new(order)),
                },
            )
        };
        for code in ["3090", "3093", "3094"] {
            assert!(Notice::is_consumed(&on(code, "HKVPP")), "{code}");
            assert!(Notice::is_consumed(&on(code, "HKVPA")), "{code}");
            assert!(!Notice::is_consumed(&on(code, "HKCCS")), "{code}");
        }
        assert!(Notice::is_consumed(&on("3045", "HKTAN")));
        for code in ["3070", "3092", "3270", "3046", "3020"] {
            assert!(!Notice::is_consumed(&on(code, "HKCCS")), "{code}");
        }
    }

    #[test]
    fn code_is_four_digits() {
        for bad in ["", "123", "12345", "XXXX", "12a4", "١٢٣٤"] {
            assert_eq!(Code::new(bad), None, "{bad}");
        }
        assert_eq!(Code::new("0010").unwrap().to_string(), "0010");
    }

    #[test]
    fn serde_round_trip() {
        let c = ReturnCode::new(
            Code::new("9210").unwrap(),
            "Konto unbekannt",
            Scope::Segment {
                number: 4,
                order: Some(SegmentType::new("HKSAL")),
            },
        )
        .with_reference_element("3")
        .with_parameters(vec!["A1".into()]);
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"code\":\"9210\""), "{json}");
        assert_eq!(serde_json::from_str::<ReturnCode>(&json).unwrap(), c);
        assert!(serde_json::from_str::<Code>("\"92\"").is_err());
        assert_eq!(c.to_string(), "9210: Konto unbekannt");
    }
}
