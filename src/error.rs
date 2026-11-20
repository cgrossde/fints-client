use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::return_code::{Category, ReturnCode};

// The types `FinTSError::SegmentUnreadable` carries, nameable without the
// `tooling` feature (which makes the whole `codec` module public).
pub use crate::codec::{CodecError, ErrorKind, SegmentId, WireError, WireErrorKind};

/// Errors that can occur during FinTS operations.
#[derive(Error, Debug)]
pub enum FinTSError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Serialization error: {0}")]
    Serialize(String),

    /// The message did not reach the bank or the answer could not be read
    /// back; `kind` says which step failed. A request timeout is
    /// [`FinTSError::Timeout`] and an HTTP status [`FinTSError::Http`].
    #[error("Transport error ({kind}): {message}")]
    Transport {
        kind: TransportKind,
        message: String,
    },

    /// An HTTP request did not finish within the transport's timeout.
    #[error("Request timed out")]
    Timeout,

    #[error("HTTP error (status {status}): {message}")]
    Http { status: u16, message: String },

    #[error("Dialog error: {0}")]
    Dialog(String),

    /// The bank rejected the message or an order (a class 9 code): the
    /// selected code line plus every code of the response. Its category
    /// says what to do ([`BankError::category`]).
    #[error("{0}")]
    Bank(BankError),

    /// No version of the order is both offered by the bank (BPD) and
    /// implemented by this client (Formals C.10). Names both sides. A
    /// client-side check before anything is sent.
    #[error("{0}")]
    VersionMismatch(crate::protocol::VersionMismatch),

    /// Stored state (session state) carries another format tag than this
    /// version reads, older or newer. There is no migration: start from an
    /// empty state.
    #[error("stored state has format {found:?}; this version reads {expected:?}")]
    StateFormat {
        found: String,
        expected: &'static str,
    },

    /// A paused dialog was thawed with a client or credentials it was not
    /// frozen with (`field`: `blz`, `url` or `user_id`). Nothing was sent.
    #[error("paused dialog belongs to another {field}")]
    PausedDialogMismatch { field: &'static str },

    /// An order was built without bank parameter data. Orders only run in
    /// a dialog opened after synchronisation, so there is no version to
    /// choose and none is guessed.
    #[error("{code}: no bank parameter data (BPD) known; synchronise first")]
    NoBpd { code: crate::types::SegmentType },

    /// The negotiated version of an order cannot address this account
    /// (national vs. international account details, Messages B.3). No
    /// lower version is tried.
    #[error("{code} v{version} cannot address this account: {reason}")]
    AccountNotAddressable {
        code: crate::types::SegmentType,
        version: u16,
        reason: AddressingReason,
    },

    /// A bug in this client, for example a segment version it claims to
    /// implement but cannot build.
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Invalid response from bank: {0}")]
    InvalidResponse(String),

    /// A received segment that matters for the dialog (HITAN) was rejected
    /// by its codec definition; carries the segment, field and rule.
    #[error("Unreadable segment from bank: {0}")]
    SegmentUnreadable(crate::codec::CodecError),

    #[error("MT940 parse error: {0}")]
    Mt940(String),

    #[error("CAMT parse error: {0}")]
    Camt(String),
    /// The payee check report (pain.002) the bank sent could not be read.
    /// The raw document is in the trace; no result is guessed.
    #[error("pain.002 error: {0}")]
    Pain002(String),
    /// The transfer the caller described cannot be sent: a field breaks a
    /// rule (SEPA charset, length, IBAN, BIC, amount). Nothing was sent;
    /// fix the data and retry.
    #[error("invalid transfer: {0}")]
    InvalidTransfer(#[from] crate::jobs::InvalidTransfer),

    /// After a message carried a payment order or its TAN, the bank's
    /// decision is unknown: a transport error, an unreadable answer
    /// (HIRMS, HITAN, HIVPP, pain.002), an answer that fits no case, a
    /// decoupled approval past the bank's `max_polls`, or a future the
    /// caller dropped mid-request (reported once, by the next call on the
    /// dialog). Every error after the order but a class 9 rejection is
    /// this one. The bank may or may not execute the payment. Check the
    /// account (by `end_to_end_id`, pain.001 `message_id`) before trying
    /// again; the library never resends. The dialog has ended when the
    /// message number is unknown.
    #[error(
        "payment outcome unknown (end-to-end ID {end_to_end_id}, message ID {message_id}): check the account before trying again"
    )]
    PaymentOutcomeUnknown {
        end_to_end_id: String,
        message_id: String,
    },

    /// The bank's maximum of decoupled status requests is used up (a
    /// read or the login; a transfer gets
    /// [`FinTSError::PaymentOutcomeUnknown`]).
    #[error("Decoupled TAN: bank's maximum of {0} status requests reached")]
    DecoupledMaxPolls(u32),

    /// A job was started while the dialog waits for a TAN or decoupled
    /// confirmation of another one: resume (`submit_tan`, `poll`) or
    /// `cancel` that job first. Nothing was sent.
    #[error("the dialog is paused on a TAN step of another job")]
    DialogPaused,

    /// The resume ticket does not belong to the job this dialog waits for
    /// (another task reference or job, or no job pending). Nothing was
    /// sent.
    #[error("resume ticket does not match the dialog's pending job")]
    WrongResume,

    /// The dialog is over: the bank ended it (9800), a transport error or a
    /// dropped request left its message number unknown, or its login
    /// failed. Open a new dialog.
    #[error("the dialog has ended; open a new one")]
    DialogEnded,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl FinTSError {
    /// A [`FinTSError::Transport`] of `kind`.
    pub fn transport(kind: TransportKind, message: impl Into<String>) -> Self {
        Self::Transport {
            kind,
            message: message.into(),
        }
    }

    /// What the caller can do about this error. Bank errors take the
    /// category of their selected code; client-side errors have a fixed
    /// one.
    pub fn kind(&self) -> Category {
        match self {
            Self::Bank(e) => e.category(),
            Self::Transport { .. } | Self::Timeout | Self::Io(_) => Category::Temporary,
            Self::Http { status, .. } if *status >= 500 || *status == 408 || *status == 429 => {
                Category::Temporary
            }
            Self::Http { .. } => Category::Rejected,
            Self::DecoupledMaxPolls(_) => Category::Tan,
            Self::VersionMismatch(_) | Self::AccountNotAddressable { .. } => Category::Unsupported,
            Self::DialogEnded => Category::DialogEnded,
            Self::InvalidTransfer(_) => Category::InvalidInput,
            Self::PaymentOutcomeUnknown { .. } => Category::OutcomeUnknown,
            Self::Parse(_)
            | Self::DialogPaused
            | Self::WrongResume
            | Self::Serialize(_)
            | Self::Dialog(_)
            | Self::NoBpd { .. }
            | Self::StateFormat { .. }
            | Self::PausedDialogMismatch { .. }
            | Self::Internal(_)
            | Self::InvalidResponse(_)
            | Self::SegmentUnreadable(_)
            | Self::Mt940(_)
            | Self::Camt(_)
            | Self::Pain002(_) => Category::ClientFault,
        }
    }

    /// The bank error, if this is one.
    pub fn bank_error(&self) -> Option<&BankError> {
        match self {
            Self::Bank(e) => Some(e),
            _ => None,
        }
    }

    /// Whether the bank refused the login data or has blocked the access.
    /// Do not repeat the request automatically: each attempt with a wrong
    /// PIN counts, and after a few the bank blocks the access until the
    /// customer has it unblocked.
    pub fn is_credentials_error(&self) -> bool {
        matches!(self.kind(), Category::Credentials(_))
    }

    /// Whether only the customer can resolve this: a TAN is wanted, or an
    /// approval in the app did not come in time. Ask the customer before
    /// trying again.
    pub fn needs_user(&self) -> bool {
        self.kind() == Category::Tan
    }

    /// A temporary failure worth retrying later unchanged: network trouble,
    /// timeouts, server errors, rate limiting, a bank system that is
    /// briefly unavailable.
    pub fn is_retryable(&self) -> bool {
        self.kind() == Category::Temporary
    }
}

/// Which step of a transport exchange failed ([`FinTSError::Transport`]).
/// [`FinTSError::kind`] reports every transport error as
/// [`Category::Temporary`]; [`TransportKind::is_retryable`] tells a
/// certificate problem (`Tls`), which a retry does not fix, from a network
/// blip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransportKind {
    /// The host name did not resolve.
    Dns,
    /// The connection was refused, reset or unreachable before TLS.
    Connect,
    /// The TLS handshake failed (certificate, protocol, alert). A retry
    /// without a change of trust store or bank URL fails again.
    Tls,
    /// Another failure while sending the request or reading the answer.
    Io,
    /// The answer arrived but is not the base64 the transport expects.
    Decode,
    /// The transport cannot work as configured (no HTTP client; feature
    /// `http` off and no transport set).
    Config,
}

impl TransportKind {
    /// Stable lower-case name (`dns`, `connect`, `tls`, `io`, `decode`,
    /// `config`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Connect => "connect",
            Self::Tls => "tls",
            Self::Io => "io",
            Self::Decode => "decode",
            Self::Config => "config",
        }
    }

    /// Whether the same request may work later without a change.
    pub fn is_retryable(self) -> bool {
        !matches!(self, Self::Tls | Self::Config)
    }
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A rejection by the bank: the code line selected as the reason
/// (docs/return-codes-design.md section 3) and every code of the response,
/// in wire order (HIRMG first, then HIRMS), so nothing is lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankError(Box<BankErrorInner>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct BankErrorInner {
    code: ReturnCode,
    codes: Vec<ReturnCode>,
}

impl BankError {
    /// `code` is the selected class 9 code, `codes` all codes of the
    /// response.
    pub fn new(code: ReturnCode, codes: Vec<ReturnCode>) -> Self {
        Self(Box::new(BankErrorInner { code, codes }))
    }

    /// The code line reported as the reason.
    pub fn code(&self) -> &ReturnCode {
        &self.0.code
    }

    /// Every code of the response, success and warning codes included.
    pub fn codes(&self) -> &[ReturnCode] {
        &self.0.codes
    }

    /// What the caller can do, from the selected code.
    pub fn category(&self) -> Category {
        self.0.code.category().unwrap_or(Category::Rejected)
    }
}

impl std::fmt::Display for BankError {
    /// `Bank error 9210: Keine gueltige Kontoverbindung`
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bank error {}", self.0.code)
    }
}

/// Why an order version cannot address an account
/// ([`FinTSError::AccountNotAddressable`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AddressingReason {
    /// The version addresses accounts nationally (ktv), but the account has
    /// no account number and BLZ and none can be derived from its IBAN.
    NoNationalDetails,
    /// The version addresses accounts by IBAN and BIC (kti), but the account
    /// has none.
    NoIbanBic,
}

impl std::fmt::Display for AddressingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoNationalDetails => {
                "it needs a national account number and BLZ, which cannot be derived \
                 from the IBAN; use Account::with_national()"
            }
            Self::NoIbanBic => "it needs an IBAN and BIC, which the account lacks",
        })
    }
}

pub type Result<T> = std::result::Result<T, FinTSError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::return_code::{Code, Scope};

    fn bank(code: &str) -> FinTSError {
        let c = ReturnCode::new(Code::new(code).unwrap(), "t", Scope::Message);
        FinTSError::Bank(BankError::new(c.clone(), vec![c]))
    }

    /// Every error has a category; the predicates follow from it.
    #[test]
    fn classification() {
        use crate::return_code::Credentials;
        let http = |status| FinTSError::Http {
            status,
            message: "m".into(),
        };
        let cases: Vec<(FinTSError, Category)> = vec![
            (bank("9942"), Category::Credentials(Credentials::PinWrong)),
            (bank("9931"), Category::Credentials(Credentials::Locked)),
            (bank("9910"), Category::Credentials(Credentials::Unclear)),
            (bank("9391"), Category::SystemId),
            (bank("9941"), Category::Tan),
            (bank("9000"), Category::OutcomeUnknown),
            (bank("9311"), Category::Temporary),
            (bank("9210"), Category::Rejected),
            (FinTSError::DecoupledMaxPolls(3), Category::Tan),
            (
                FinTSError::transport(TransportKind::Io, "reset"),
                Category::Temporary,
            ),
            (FinTSError::Timeout, Category::Temporary),
            (http(503), Category::Temporary),
            (http(408), Category::Temporary),
            (http(429), Category::Temporary),
            (http(404), Category::Rejected),
            (FinTSError::DialogPaused, Category::ClientFault),
            (FinTSError::WrongResume, Category::ClientFault),
            (FinTSError::DialogEnded, Category::DialogEnded),
            (
                FinTSError::PausedDialogMismatch { field: "blz" },
                Category::ClientFault,
            ),
            (FinTSError::Internal("x".into()), Category::ClientFault),
            (
                FinTSError::InvalidResponse("x".into()),
                Category::ClientFault,
            ),
            (
                FinTSError::NoBpd {
                    code: crate::types::SegmentType::new("HKSAL"),
                },
                Category::ClientFault,
            ),
            (
                FinTSError::AccountNotAddressable {
                    code: crate::types::SegmentType::new("HKSAL"),
                    version: 4,
                    reason: AddressingReason::NoIbanBic,
                },
                Category::Unsupported,
            ),
        ];
        for (err, want) in &cases {
            assert_eq!(err.kind(), *want, "{err}");
            assert_eq!(
                err.is_credentials_error(),
                matches!(want, Category::Credentials(_)),
                "{err}"
            );
            assert_eq!(err.needs_user(), *want == Category::Tan, "{err}");
            assert_eq!(err.is_retryable(), *want == Category::Temporary, "{err}");
        }
        assert_eq!(bank("9210").to_string(), "Bank error 9210: t");
    }

    /// Categories and transport kinds go over a wire as stable snake_case
    /// strings; credentials carry their reason.
    #[test]
    fn category_and_transport_kind_serialize() {
        use crate::return_code::Credentials;
        let json = |c: Category| serde_json::to_string(&c).unwrap();
        assert_eq!(json(Category::Temporary), "\"temporary\"");
        assert_eq!(json(Category::DialogEnded), "\"dialog_ended\"");
        assert_eq!(
            json(Category::Credentials(Credentials::PinWrong)),
            "{\"credentials\":\"pin_wrong\"}"
        );
        for c in [
            Category::Tan,
            Category::OutcomeUnknown,
            Category::Credentials(Credentials::Locked),
        ] {
            assert_eq!(serde_json::from_str::<Category>(&json(c)).unwrap(), c);
        }
        for (k, s) in [
            (TransportKind::Dns, "dns"),
            (TransportKind::Connect, "connect"),
            (TransportKind::Tls, "tls"),
            (TransportKind::Io, "io"),
            (TransportKind::Decode, "decode"),
            (TransportKind::Config, "config"),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), format!("\"{s}\""));
            assert_eq!(k.as_str(), s);
        }
        assert!(TransportKind::Dns.is_retryable() && !TransportKind::Tls.is_retryable());
    }
}
