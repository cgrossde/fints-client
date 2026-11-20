//! A dialog frozen between two calls (docs/session-state-design.md §3–§4).
//!
//! [`Dialog::freeze`] turns an open or paused dialog into a
//! [`PausedDialog`]: plain, versioned data the caller stores (encrypted: it
//! holds the user ID, system ID, UPD with IBANs and names, dialog ID, task
//! reference and challenge) and passes to [`Dialog::thaw`] later, in
//! another process if need be, with the bank client and the PIN. A paused
//! transfer also holds its pain.001 (payee name, IBAN, amount) and payee
//! check result. The
//! thawed dialog sends its next message with the same dialog ID and the
//! next message number, as if it had never been frozen.
//!
//! Never stored: PIN, TAN, transport, quirk chain, product ID. The library
//! sets no expiry of its own: [`PausedDialog::expires_at`] tells the caller
//! the bank's timeout, and a dialog the bank has dropped answers 9800
//! ([`FinTSError::DialogEnded`]), a TAN past its window 9951.

use super::*;
use crate::codec::RawSegment;
use crate::protocol::{Bpd, TanOverrides};
use crate::session::{format_tag, stars, wire_segments};

/// The `format` tag of a stored [`PausedDialog`]. Bumped on any change to
/// its shape; no other version is read.
pub const PAUSED_DIALOG_FORMAT: &str = "fints-client-paused/4";

format_tag!(
    /// The format tag of [`PausedDialog`] ([`PAUSED_DIALOG_FORMAT`]).
    PausedFormat,
    PAUSED_DIALOG_FORMAT,
    "paused dialog"
);

/// A dialog frozen by [`Dialog::freeze`]: everything to continue it except
/// the transport and the PIN. Self-contained: it carries the dialog's own
/// BPD and UPD (also a UPD valid for this dialog only, which no
/// [`SessionState`] keeps) and TAN selection.
///
/// `Debug` shows no user ID, system ID, dialog ID, UPD or challenge.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PausedDialog {
    format: PausedFormat,
    /// Checked against the client on thaw.
    blz: Blz,
    url: String,
    /// Checked against the credentials on thaw.
    user_id: UserId,
    system_id: SystemId,
    dialog_id: DialogId,
    /// The number the next message carries.
    message_number: u16,
    params: StoredParams,
    /// The bank profile's HKTAN overrides at open; profile changes apply
    /// to new dialogs only.
    tan_overrides: TanOverrides,
    /// The job waiting for a TAN; `None` for a dialog frozen while open.
    pending: Option<PendingJob>,
    /// Wall clock of the bank's last response.
    last_message_at: Timestamp,
}

/// The dialog's [`BankParams`] as data: the raw BPD and UPD as the bank
/// sent them, and the state the dialog added to them.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredParams {
    bpd_version: u16,
    upd_version: u16,
    #[serde(with = "wire_segments")]
    bpd: Vec<RawSegment>,
    #[serde(with = "wire_segments")]
    upd: Vec<RawSegment>,
    selected_security_function: SecurityFunction,
    preferred_security_function: Option<SecurityFunction>,
    selected_tan_medium: Option<TanMediumName>,
    allowed_security_functions: Vec<SecurityFunction>,
    changed_user_id: Option<UserId>,
    tan_required_learned: Vec<SegmentType>,
}

impl StoredParams {
    fn new(p: &BankParams) -> Self {
        Self {
            bpd_version: p.bpd_version,
            upd_version: p.upd_version,
            bpd: p.bpd_segments.clone(),
            upd: p.upd_segments.clone(),
            selected_security_function: p.selected_security_function.clone(),
            preferred_security_function: p.preferred_security_function.clone(),
            selected_tan_medium: p.selected_tan_medium.clone(),
            allowed_security_functions: p.allowed_security_functions.clone(),
            changed_user_id: p.changed_user_id.clone(),
            tan_required_learned: p.tan_required_learned.iter().cloned().collect(),
        }
    }

    fn to_params(&self) -> BankParams {
        let mut p = BankParams::new();
        p.set_bpd(self.bpd.clone());
        p.set_upd(self.upd.clone());
        p.bpd_version = self.bpd_version;
        p.upd_version = self.upd_version;
        p.selected_security_function = self.selected_security_function.clone();
        p.preferred_security_function = self.preferred_security_function.clone();
        p.selected_tan_medium = self.selected_tan_medium.clone();
        p.allowed_security_functions = self.allowed_security_functions.clone();
        p.changed_user_id = self.changed_user_id.clone();
        p.tan_required_learned = self.tan_required_learned.iter().cloned().collect();
        p
    }
}

/// The typed ticket of a paused job, rebuilt from a [`PausedDialog`] (one
/// variant per job), so a restarted caller can resume it.
#[derive(Debug)]
pub enum AnyResume {
    Open(Resume<Opened>),
    Balance(Resume<Option<AccountBalance>>),
    Transactions(Resume<Statement>),
    Holdings(Resume<Vec<SecurityHolding>>),
    DepotTransactions(Resume<Vec<DepotTransaction>>),
    Transfer(Resume<TransferReceipt>),
    InstantStatus(Resume<InstantStatus>),
}

/// Which job a paused step belongs to ([`AnyResume::kind`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResumeKind {
    Open,
    Balance,
    Transactions,
    Holdings,
    DepotTransactions,
    Transfer,
    InstantStatus,
}

impl AnyResume {
    /// The job this ticket resumes, to label or log it without matching
    /// every variant.
    pub fn kind(&self) -> ResumeKind {
        match self {
            Self::Open(_) => ResumeKind::Open,
            Self::Balance(_) => ResumeKind::Balance,
            Self::Transactions(_) => ResumeKind::Transactions,
            Self::Holdings(_) => ResumeKind::Holdings,
            Self::DepotTransactions(_) => ResumeKind::DepotTransactions,
            Self::Transfer(_) => ResumeKind::Transfer,
            Self::InstantStatus(_) => ResumeKind::InstantStatus,
        }
    }

    fn of(pending: &PendingJob) -> Self {
        fn ticket<T>(pending: &PendingJob) -> Resume<T> {
            Resume {
                ticket: pending.ticket.clone(),
                _value: PhantomData,
            }
        }
        match pending.job.job() {
            Job::Open => Self::Open(ticket(pending)),
            Job::Balance => Self::Balance(ticket(pending)),
            Job::Transactions => Self::Transactions(ticket(pending)),
            Job::Holdings => Self::Holdings(ticket(pending)),
            Job::DepotTransactions => Self::DepotTransactions(ticket(pending)),
            Job::Transfer => Self::Transfer(ticket(pending)),
            Job::InstantStatus => Self::InstantStatus(ticket(pending)),
        }
    }
}

impl PausedDialog {
    /// The ticket of the paused job; `None` for a dialog frozen while open.
    /// Take it before [`Dialog::thaw`], which consumes the value.
    pub fn pending(&self) -> Option<AnyResume> {
        self.pending.as_ref().map(AnyResume::of)
    }

    /// The TAN challenge the paused job waits on, to show without thawing;
    /// `None` while a transfer waits on its payee check.
    pub fn challenge(&self) -> Option<&TanChallenge> {
        self.pending.as_ref().and_then(PendingJob::challenge)
    }

    /// The payee check result a paused transfer waits to have confirmed,
    /// to show without thawing.
    pub fn vop(&self) -> Option<&VopConfirmation> {
        self.pending.as_ref().and_then(PendingJob::vop)
    }

    /// The bank the dialog belongs to.
    pub fn blz(&self) -> &Blz {
        &self.blz
    }

    /// Wall clock of the bank's last response.
    pub fn last_message_at(&self) -> Timestamp {
        self.last_message_at
    }

    /// When the bank's timeout ends the dialog, if it announced one:
    /// `last_message_at` plus the BPD's maximum timeout (HIBPA "Maximaler
    /// Timeout-Wert", seconds; absent or 0 = no limit). Only our own clock
    /// goes into it. The challenge's validity (HITAN "Gültigkeitsdatum und
    /// -uhrzeit für Challenge") is a bank-local wall-clock value without a
    /// zone; it stays readable on the challenge itself as a civil
    /// [`DateTime`]. `None`: no timeout announced. Informational:
    /// [`Dialog::thaw`] never refuses on time, and the bank may end a
    /// dialog earlier.
    pub fn expires_at(&self) -> Option<Timestamp> {
        let secs = Bpd::from_segments(&self.params.bpd)
            .ok()
            .and_then(|b| b.max_timeout)
            .filter(|s| *s > 0)?;
        self.last_message_at
            .checked_add(jiff::SignedDuration::from_secs(i64::from(secs)))
            .ok()
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|e| FinTSError::Serialize(format!("paused dialog: {e}")))
    }

    /// Deserialize from JSON produced by [`PausedDialog::to_json`]. Any
    /// other format tag fails with [`FinTSError::StateFormat`].
    pub fn from_json(json: &str) -> Result<Self> {
        crate::session::check_format(json, PAUSED_DIALOG_FORMAT)?;
        serde_json::from_str(json).map_err(|e| FinTSError::Parse(format!("paused dialog: {e}")))
    }
}

impl std::fmt::Debug for PausedDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PausedDialog")
            .field("blz", &self.blz)
            .field("user_id", &stars(self.user_id.as_str()))
            .field("system_id", &stars(self.system_id.as_str()))
            .field("dialog_id", &stars(self.dialog_id.as_str()))
            .field("message_number", &self.message_number)
            .field("bpd_version", &self.params.bpd_version)
            .field("upd_version", &self.params.upd_version)
            .field("bpd_segments", &self.params.bpd.len())
            .field("upd_segments", &self.params.upd.len())
            .field(
                "phase",
                &if self.pending.is_some() {
                    "Paused"
                } else {
                    "Open"
                },
            )
            .field("pending_job", &self.pending.as_ref().map(|p| p.job.job()))
            .field("last_message_at", &self.last_message_at)
            .finish()
    }
}

impl Dialog {
    /// Freeze the dialog into plain data to store, for an open dialog
    /// between two jobs and for one paused on a TAN. Sends nothing;
    /// consumes the dialog, so no live copy can send the same message
    /// number. An ended dialog fails with [`FinTSError::DialogEnded`]; one
    /// whose transfer request was dropped mid-flight with
    /// [`FinTSError::PaymentOutcomeUnknown`].
    pub fn freeze(mut self) -> Result<PausedDialog> {
        self.check_in_flight()?;
        let pending = match self.phase {
            Phase::Ended => return Err(FinTSError::DialogEnded),
            Phase::Open => None,
            Phase::Paused(p) => Some(*p),
        };
        let inner = &self.inner;
        Ok(PausedDialog {
            format: PausedFormat,
            blz: inner.blz().clone(),
            url: inner.url().to_string(),
            user_id: inner.user_id().clone(),
            system_id: inner.system_id().clone(),
            dialog_id: inner.dialog_id().clone(),
            message_number: inner.message_number(),
            params: StoredParams::new(inner.bank_params()),
            tan_overrides: inner.tan_overrides().clone(),
            pending,
            last_message_at: self.last_message_at,
        })
    }

    /// Continue a frozen dialog through `client` with the PIN of `creds`.
    /// Fails with [`FinTSError::PausedDialogMismatch`] when the client's
    /// BLZ or URL, or the user ID, differ from the frozen dialog's; sends
    /// nothing. The thawed dialog is paused on the same job (resume it with
    /// the ticket of [`PausedDialog::pending`]) or open. It never refuses
    /// on time: a dialog the bank has dropped answers with
    /// [`FinTSError::DialogEnded`].
    pub fn thaw(paused: PausedDialog, client: &BankClient, creds: &Credentials) -> Result<Dialog> {
        let config = client.config();
        let field = if paused.blz != config.blz {
            Some("blz")
        } else if paused.url != config.url.as_str() {
            Some("url")
        } else if paused.user_id != client.effective_user_id(&creds.user_id) {
            Some("user_id")
        } else {
            None
        };
        if let Some(field) = field {
            return Err(FinTSError::PausedDialogMismatch { field });
        }
        let inner = client
            .new_dialog(creds)?
            .with_system_id(&paused.system_id)
            .with_params(&paused.params.to_params())
            .with_tan_overrides(paused.tan_overrides)
            .resume(paused.dialog_id, paused.message_number);
        Ok(Dialog {
            inner,
            bic: client.config().bic.clone(),
            booking_id: client.config().profile.booking_id,
            phase: match paused.pending {
                Some(p) => Phase::Paused(Box::new(p)),
                None => Phase::Open,
            },
            last_message_at: paused.last_message_at,
            in_flight: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::wire;

    const BPD: &str = "HIBPA:6:3:4+78+280:12345678+Test Bank+1+1+300+500+10+300'\
        HITANS:53:5:4+1+1+1+N:N:0:942:2:MTAN2:mobileTAN::mobile TAN:6:1:SMS:3:1:J:1:0:N:0:2:N:J:00:1:1'\
        HIPINS:54:1:4+1+1+1+5:20:6:Benutzer ID::HKKAZ:N:HKSAL:N:HKTAN:N'";
    const UPD: &str = "HIUPA:57:4:4+test1+0+0'\
        HIUPD:58:6:4+1::280:12345678+DE111234567800000001+test1++EUR+Fullname++Girokonto++HKSAL:1'";

    #[test]
    fn any_resume_kind() {
        let p = sample(None);
        assert_eq!(p.pending().unwrap().kind(), ResumeKind::Balance);
    }

    fn sample(valid_until: Option<DateTime>) -> PausedDialog {
        let mut p = BankParams::new();
        p.set_bpd(wire::tokenize(BPD.as_bytes()).unwrap());
        p.set_upd(wire::tokenize(UPD.as_bytes()).unwrap());
        p.allowed_security_functions = vec![SecurityFunction::new("942")];
        p.select_security_function();
        PausedDialog {
            format: PausedFormat,
            blz: Blz::new("12345678"),
            url: "https://bank.example/fints".into(),
            user_id: UserId::new("test1"),
            system_id: SystemId::new("SYS;001"),
            dialog_id: DialogId::new("DLG4711"),
            message_number: 4,
            params: StoredParams::new(&p),
            tan_overrides: TanOverrides::default(),
            pending: Some(PendingJob {
                ticket: Ticket::Task(TaskReference::new("TASK0001")),
                wait: Wait::Tan(TanChallenge {
                    challenge: ChallengeText::new("Ueberweisung an Beispiel 12,00"),
                    challenge_markup: false,
                    kind: TanKind::Typed { media: None },
                    valid_until,
                    not_before: None,
                    payee_check: None,
                    rejected: None,
                }),
                job: JobInput::Balance {
                    account: Account::new("DE02120300000000202051", "BYLADEM1001").unwrap(),
                },
                notices: Vec::new(),
            }),
            last_message_at: Timestamp::constant(1_768_471_200, 0),
        }
    }

    /// The dialog-only UPD (version 0) survives, unlike in `SessionState`.
    #[test]
    fn round_trips_through_json_with_a_dialog_only_upd() {
        let paused = sample(None);
        let json = paused.to_json().unwrap();
        assert!(json.contains("\"format\":\"fints-client-paused/4\""));
        let back = PausedDialog::from_json(&json).unwrap();
        assert_eq!(back.to_json().unwrap(), json);
        let p = back.params.to_params();
        assert!(p.upd().unwrap().is_dialog_only());
        assert_eq!(p.upd().unwrap().accounts().len(), 1);
        assert_eq!(p.selected_security_function, SecurityFunction::new("942"));
        assert!(matches!(back.pending(), Some(AnyResume::Balance(_))));
    }

    #[test]
    fn other_formats_and_unknown_fields_are_rejected() {
        let json = sample(None).to_json().unwrap();
        for other in [
            "fints-client-paused/3",
            "fints-client-paused/5",
            "fints-client-session/1",
        ] {
            match PausedDialog::from_json(&json.replace(PAUSED_DIALOG_FORMAT, other)) {
                Err(FinTSError::StateFormat { found, expected }) => {
                    assert_eq!((found.as_str(), expected), (other, PAUSED_DIALOG_FORMAT));
                }
                r => panic!("{other}: {r:?}"),
            }
        }
        let bad = json.replace(PAUSED_DIALOG_FORMAT, "fints-client-paused/5");
        assert!(serde_json::from_str::<PausedDialog>(&bad).is_err());
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["extra"] = serde_json::json!(1);
        assert!(matches!(
            PausedDialog::from_json(&v.to_string()),
            Err(FinTSError::Parse(_))
        ));
        v.as_object_mut().unwrap().remove("extra");
        v.as_object_mut().unwrap().remove("format");
        assert!(matches!(
            PausedDialog::from_json(&v.to_string()),
            Err(FinTSError::StateFormat { .. })
        ));
    }

    #[test]
    fn debug_hides_identifiers_upd_and_challenge() {
        let out = format!("{:?}", sample(None));
        for secret in [
            "test1", "SYS;001", "DLG4711", "DE11", "DE02", "Fullname", "Beispiel", "TASK",
        ] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert!(out.contains("12345678") && out.contains("Paused"), "{out}");
    }

    /// HIBPA maximum timeout 300 s from our own clock; the challenge
    /// validity does not enter.
    #[test]
    fn expires_at_from_the_banks_limits() {
        let paused = sample(None);
        assert_eq!(
            paused.expires_at(),
            Some(Timestamp::constant(1_768_471_200 + 300, 0))
        );
        let valid_until = Date::new(2026, 1, 15).unwrap().at(10, 2, 0, 0);
        assert_eq!(
            sample(Some(valid_until)).expires_at(),
            paused.expires_at(),
            "challenge validity is not part of it"
        );

        let mut none = sample(None);
        let bpd = BPD.replace("+10+300'", "+10+0'");
        none.params.bpd = wire::tokenize(bpd.as_bytes()).unwrap();
        assert_eq!(none.expires_at(), None, "0 = no limit");
        none.params.bpd = wire::tokenize(BPD.replace("+500+10+300'", "'").as_bytes()).unwrap();
        assert_eq!(none.expires_at(), None, "absent = no limit");
    }
}
