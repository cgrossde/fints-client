//! What one bank does differently from the FinTS specification and its own
//! parameter data, as a serialisable value.
//!
//! A profile holds only what the BPD and UPD cannot tell the client; job
//! versions, statement formats, KTI form and TAN methods come from the
//! bank's parameters. Evidence per entry: docs/banks.md.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::protocol::TanOverrides;

/// Which profile a [`BankProfile`] is: for logs, traces and callers that
/// show bank-specific help.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileId {
    /// No bank-specific behaviour.
    #[default]
    Generic,
    Dkb,
    Comdirect,
    Consorsbank,
    Ing,
}

impl ProfileId {
    /// The profile's name as serialised (`"dkb"`, `"generic"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Dkb => "dkb",
            Self::Comdirect => "comdirect",
            Self::Consorsbank => "consorsbank",
            Self::Ing => "ing",
        }
    }
}

impl std::fmt::Display for ProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How [`jobs::open`](crate::jobs::open) sets the session up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitMode {
    /// Synchronisation dialog when the session state has no system ID and
    /// parameters, then the business dialog with its TAN if the bank asks.
    #[default]
    Standard,
    /// No synchronisation dialog: the business dialog opens PIN-only with
    /// the system ID of the session state (or a client-generated one) and
    /// this security function. The bank sends its parameters in that
    /// dialog and asks for no TAN.
    Direct { security_function: String },
}

/// A bank's rule for completing the FinTS user ID the customer typed.
///
/// A user ID of only ASCII digits whose length is in
/// `min_digits..=max_digits` gets `suffix` appended; any other user ID is
/// sent as given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserIdRule {
    pub min_digits: u8,
    pub max_digits: u8,
    pub suffix: String,
}

impl UserIdRule {
    /// The user ID as it is sent: `user_id` with `suffix` appended when it
    /// is only ASCII digits and its length is in range, else unchanged.
    pub fn apply<'a>(&self, user_id: &'a str) -> Cow<'a, str> {
        let n = user_id.len();
        let digits = !user_id.is_empty() && user_id.bytes().all(|b| b.is_ascii_digit());
        if digits && (self.min_digits as usize..=self.max_digits as usize).contains(&n) {
            Cow::Owned(format!("{user_id}{}", self.suffix))
        } else {
            Cow::Borrowed(user_id)
        }
    }
}

/// Which statement reference, if any, the bank keeps unique and stable
/// for each booking: the source of [`Transaction::booking_id`].
///
/// [`Transaction::booking_id`]: crate::Transaction::booking_id
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BookingIdSource {
    /// No reference is known to be unique and stable: `booking_id` stays
    /// `None`. The MT940 `:61:` bank reference is never one (often
    /// `NONREF` or repeated).
    #[default]
    None,
    /// The entry-level camt `Ntry/AcctSvcrRef` of a booked entry. Pending
    /// entries, MT940 statements and entries without the element get no ID.
    CamtEntryAcctSvcrRef,
}

/// What one bank does differently. [`BankProfile::generic`] for banks
/// without an entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BankProfile {
    pub id: ProfileId,
    /// How the business dialog is opened. [`InitMode::Standard`] for most
    /// banks.
    pub init: InitMode,
    /// Orders for which the profile fixes whether HKTAN is sent, beating
    /// HIPINS. Empty for most banks.
    pub tan_overrides: TanOverrides,
    /// How the library completes the user ID before it is sent. `None`:
    /// sent as given.
    pub user_id: Option<UserIdRule>,
    /// Which reference becomes [`Transaction::booking_id`]. `None` for
    /// banks whose references are not known to be unique and stable.
    ///
    /// [`Transaction::booking_id`]: crate::Transaction::booking_id
    pub booking_id: BookingIdSource,
}

impl BankProfile {
    /// No bank-specific behaviour.
    pub fn generic() -> Self {
        Self::default()
    }
}
