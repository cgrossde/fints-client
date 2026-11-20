//! The state a client keeps between runs.
//!
//! Formals has the customer system keep what the bank handed out: the
//! system ID from synchronisation, and the bank and user parameter data
//! (BPD, UPD) with their version numbers. HKVVB names the versions the
//! client holds, and the bank sends parameter data again only when its own
//! version is newer. [`SessionState`] is all of that in one value the
//! caller stores; the library writes no files.
//!
//! Keep the system ID: banks tie strong customer authentication
//! exemptions to it, and a client that comes back with a new ID every run
//! gets asked for TANs more often. Store the state after each run and hand
//! it back the next time.
//!
//! The state holds neither PIN nor TAN, but it names the user and the
//! accounts. Keep it encrypted.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::codec::{RawSegment, wire};
use crate::error::{FinTSError, Result};
use crate::protocol::{BankParams, Bpd, Upd};
use crate::types::*;

/// The `format` tag of a stored [`SessionState`]. Bumped on any change to
/// its shape; no other version is read.
pub const SESSION_STATE_FORMAT: &str = "fints-client-session/2";

/// A format tag as a value: serialises as its string and deserialises only
/// from exactly that string, so the check holds in every serde format, not
/// only in `from_json`.
macro_rules! format_tag {
    ($(#[$meta:meta])* $name:ident, $tag:expr, $what:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name;

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.serialize_str($tag)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let found = String::deserialize(d)?;
                if found == $tag {
                    Ok(Self)
                } else {
                    Err(serde::de::Error::custom(format!(
                        concat!($what, " format {:?}; this version reads {:?}"),
                        found, $tag
                    )))
                }
            }
        }
    };
}
pub(crate) use format_tag;

format_tag!(
    /// The format tag of [`SessionState`] ([`SESSION_STATE_FORMAT`]).
    SessionFormat,
    SESSION_STATE_FORMAT,
    "session state"
);

/// Fail with [`FinTSError::StateFormat`] when `json` is an object whose
/// `format` is not `expected` (or missing). Anything else is left to the
/// full parse.
pub(crate) fn check_format(json: &str, expected: &'static str) -> Result<()> {
    #[derive(Deserialize)]
    struct Tag {
        format: Option<String>,
    }
    if let Ok(Tag { format }) = serde_json::from_str(json) {
        let found = format.unwrap_or_default();
        if found != expected {
            return Err(FinTSError::StateFormat { found, expected });
        }
    }
    Ok(())
}

/// What one user's dialogs with one bank leave behind for the next run.
///
/// Store the state that `Dialog::end` or `jobs::synchronise` returns after
/// every dialog and pass it to the next `jobs::open`. With it, the next
/// dialog skips the sync dialog, announces the cached BPD/UPD versions so
/// the bank sends nothing it has already sent, and keeps the system ID the
/// bank knows. A lost state gets a new system ID, and a bank that granted
/// an SCA exemption (3076) to the old one may ask for a login TAN again.
///
/// `Debug` hides user ID, system ID and the UPD content.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    /// Checked on every read; see [`SESSION_STATE_FORMAT`].
    format: SessionFormat,
    /// The bank code.
    pub blz: Blz,
    /// The login. When the bank announces a new one (return code 3072) it
    /// replaces the old one here.
    pub user_id: UserId,
    /// The customer system ID from HISYN; `0` until the bank gave one.
    pub system_id: SystemId,
    /// The BPD version HKVVB announces; `0` asks for the BPD.
    pub bpd_version: u16,
    /// The UPD version HKVVB announces; `0` asks for the UPD.
    pub upd_version: u16,
    /// The BPD segments, kept in their wire form, byte for byte.
    #[serde(with = "wire_segments")]
    bpd: Vec<RawSegment>,
    /// The UPD segments, kept in their wire form, byte for byte.
    #[serde(with = "wire_segments")]
    upd: Vec<RawSegment>,
    /// The security function (TAN method) the user prefers, such as `902`.
    pub tan_method: Option<SecurityFunction>,
    /// The TAN medium (device) to name for methods that need one.
    pub tan_medium: Option<TanMediumName>,
    /// The security functions of the bank's last return code 3920.
    pub allowed_security_functions: Vec<SecurityFunction>,
    /// The login used before the bank's last 3072, so the caller can update
    /// what it stores; `user_id` already holds the new one.
    pub previous_user_id: Option<UserId>,
    /// Order codes (e.g. `HKSAL`) for which the bank answered 9075 without
    /// HKTAN; the library sends HKTAN with them at once. Dropped by `reset`.
    pub tan_required_learned: Vec<SegmentType>,
    /// When the state was written.
    pub updated_at: Timestamp,
}

/// `0` for a segment list that is empty, so that the bank sends it again.
fn version_if_present(segments: &[RawSegment], version: u16) -> u16 {
    if segments.is_empty() { 0 } else { version }
}

impl SessionState {
    /// The state before the first dialog: nothing known.
    pub fn new(blz: &Blz, user_id: &UserId) -> Self {
        Self::fresh(blz.clone(), user_id.clone(), SystemId::unassigned())
    }

    fn fresh(blz: Blz, user_id: UserId, system_id: SystemId) -> Self {
        Self {
            format: SessionFormat,
            blz,
            user_id,
            system_id,
            bpd_version: 0,
            upd_version: 0,
            bpd: Vec::new(),
            upd: Vec::new(),
            tan_method: None,
            tan_medium: None,
            allowed_security_functions: Vec::new(),
            previous_user_id: None,
            tan_required_learned: Vec::new(),
            updated_at: Timestamp::now(),
        }
    }

    /// Start from a known system ID. The host stores the ID itself, so a
    /// state format change keeps the bank's device entry: the new state has
    /// no BPD or UPD (one parameter dialog fetches them) but no new sync
    /// dialog either.
    pub fn with_system_id(self, system_id: &SystemId) -> Self {
        Self {
            system_id: system_id.clone(),
            ..self
        }
    }

    /// Ask for this security function whenever the bank offers it.
    pub fn with_tan_method(self, security_function: &SecurityFunction) -> Self {
        Self {
            tan_method: Some(security_function.clone()),
            ..self
        }
    }

    /// Name this device to methods that ask for a TAN medium.
    pub fn with_tan_medium(self, medium: &TanMediumName) -> Self {
        Self {
            tan_medium: Some(medium.clone()),
            ..self
        }
    }

    /// Whether the next dialog can go straight to business: the bank gave a
    /// system ID and the BPD (with the TAN methods) is cached.
    pub fn can_skip_sync(&self) -> bool {
        let has_bpd = !self.bpd.is_empty() && self.bpd_version != 0;
        has_bpd && self.system_id.is_assigned()
    }

    /// The TAN methods of the cached BPD.
    pub fn tan_methods(&self) -> Vec<TanMethod> {
        self.to_params().tan_methods
    }

    /// The cached BPD as a typed model, rebuilt from the stored segments.
    /// `None` when no readable BPD is stored.
    #[cfg(feature = "tooling")]
    pub fn bpd(&self) -> Option<Bpd> {
        Bpd::from_segments(&self.bpd).ok()
    }

    #[cfg(not(feature = "tooling"))]
    #[allow(dead_code)]
    pub(crate) fn bpd(&self) -> Option<Bpd> {
        Bpd::from_segments(&self.bpd).ok()
    }

    /// The cached UPD as a typed model, rebuilt from the stored segments.
    /// `None` when no UPD is stored, which includes banks that send it for
    /// one dialog only (UPD version 0): the next dialog asks for it again.
    #[cfg(feature = "tooling")]
    pub fn upd(&self) -> Option<Upd> {
        Upd::from_segments(&self.upd).ok()
    }

    #[cfg(not(feature = "tooling"))]
    pub(crate) fn upd(&self) -> Option<Upd> {
        Upd::from_segments(&self.upd).ok()
    }

    /// What the bank offers per account (`accounts`, or the cached UPD's
    /// accounts when empty), from the cached BPD and UPD and the learned
    /// TAN codes; sends nothing. Without a stored UPD (none yet, or a bank
    /// that sends it for one dialog only, see `SessionState::upd`) pass
    /// the accounts; every UPD allowance is then `Unknown`, and
    /// [`Dialog::capabilities`](crate::jobs::Dialog::capabilities) knows the
    /// dialog's UPD. TAN overrides of the client config are not applied here.
    pub fn capabilities(&self, accounts: &[crate::Account]) -> crate::jobs::Capabilities {
        crate::jobs::Capabilities::build(
            &self.to_params(),
            &crate::protocol::TanOverrides::default(),
            accounts,
        )
    }

    /// Accounts listed in the cached UPD (none when the bank sends the UPD
    /// for one dialog only, see `SessionState::upd`), as
    /// [`Opened::accounts`](crate::jobs::Opened::accounts) lists them;
    /// `bank_bic` fills in the BIC the UPD does not give.
    pub fn accounts(&self, bank_bic: &Bic) -> Vec<crate::jobs::AccountInfo> {
        self.upd()
            .map(|u| crate::jobs::AccountInfo::from_upd(&u, bank_bic))
            .unwrap_or_default()
    }

    /// Forget everything the bank sent, for instance when it no longer
    /// accepts the system ID (9391); the next run synchronises again. Bank,
    /// login and the user's TAN choice stay.
    pub fn reset(&mut self) {
        let mut fresh = Self::fresh(
            self.blz.clone(),
            self.user_id.clone(),
            SystemId::unassigned(),
        );
        fresh.tan_method = self.tan_method.take();
        fresh.tan_medium = self.tan_medium.take();
        *self = fresh;
    }

    /// The state as JSON, for the caller to store.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|e| FinTSError::Serialize(format!("session state: {e}")))
    }

    /// Deserialize from JSON produced by [`SessionState::to_json`]. Any
    /// other format tag, older or newer, fails with
    /// [`FinTSError::StateFormat`]; there is no migration.
    pub fn from_json(json: &str) -> Result<Self> {
        check_format(json, SESSION_STATE_FORMAT)?;
        serde_json::from_str(json).map_err(|e| FinTSError::Parse(format!("session state: {e}")))
    }

    /// The parameters a dialog starts with.
    pub(crate) fn to_params(&self) -> BankParams {
        let mut params = BankParams::new();
        params.set_bpd(self.bpd.clone());
        params.set_upd(self.upd.clone());
        // Announce the stored version only together with the stored data;
        // without the data, version 0 makes the bank send it again.
        params.bpd_version = version_if_present(&self.bpd, self.bpd_version);
        params.upd_version = version_if_present(&self.upd, self.upd_version);
        params.allowed_security_functions = self.allowed_security_functions.clone();
        params.tan_required_learned = self.tan_required_learned.iter().cloned().collect();
        params.preferred_security_function = self.tan_method.clone();
        params.selected_tan_medium = self.tan_medium.clone();
        params.select_security_function();
        params
    }

    /// The state a finished dialog leaves.
    pub(crate) fn capture(
        blz: &Blz,
        user_id: &UserId,
        system_id: &SystemId,
        params: &BankParams,
    ) -> Self {
        let mut state = Self::fresh(blz.clone(), user_id.clone(), system_id.clone());

        if let Some(renamed) = params.changed_user_id.as_ref().filter(|id| *id != user_id) {
            state.previous_user_id = Some(std::mem::replace(&mut state.user_id, renamed.clone()));
        }

        state.bpd = params.bpd_segments.clone();
        state.bpd_version = params.bpd_version;
        // UPD version 0 means "valid in this dialog only" (Formals C.3.2.3);
        // keeping nothing makes the next dialog ask again.
        if !params.upd().is_some_and(Upd::is_dialog_only) {
            state.upd = params.upd_segments.clone();
            state.upd_version = params.upd_version;
        }

        let chosen = &params.selected_security_function;
        if *chosen != SecurityFunction::pin_only() {
            state.tan_method = Some(chosen.clone());
        }
        state.tan_medium = params.selected_tan_medium.clone();
        state.allowed_security_functions = params.allowed_security_functions.clone();
        state.tan_required_learned = params.tan_required_learned.iter().cloned().collect();
        state
    }
}

/// One star per character, as `mask` does for identifiers.
pub(crate) fn stars(s: &str) -> String {
    "*".repeat(s.chars().count())
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState")
            .field("blz", &self.blz)
            .field("user_id", &stars(self.user_id.as_str()))
            .field("system_id", &stars(self.system_id.as_str()))
            .field("bpd_version", &self.bpd_version)
            .field("upd_version", &self.upd_version)
            .field("bpd_segments", &self.bpd.len())
            .field("upd_segments", &self.upd.len())
            .field("tan_method", &self.tan_method)
            .field("tan_medium", &self.tan_medium.as_ref().map(|_| "<set>"))
            .field(
                "allowed_security_functions",
                &self.allowed_security_functions,
            )
            .field(
                "previous_user_id",
                &self.previous_user_id.as_ref().map(|u| stars(u.as_str())),
            )
            .field("tan_required_learned", &self.tan_required_learned)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Serialize segments as their FinTS wire form (ISO-8859-1 mapped 1:1 to
/// chars), which is compact, diffable, and exactly what the bank sent.
pub(crate) mod wire_segments {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        segments: &[RawSegment],
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let out = wire::serialize(segments).map_err(serde::ser::Error::custom)?;
        s.serialize_str(&out.iter().map(|&b| b as char).collect::<String>())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Vec<RawSegment>, D::Error> {
        let text = String::deserialize(d)?;
        let bytes = text
            .chars()
            .map(|c| {
                u8::try_from(u32::from(c))
                    .map_err(|_| serde::de::Error::custom("non-Latin-1 char in segment data"))
            })
            .collect::<std::result::Result<Vec<u8>, _>>()?;
        wire::tokenize(&bytes).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic parameter data in the style of `testbank/`: one decoupled
    /// method 902, HKSAL and HKKAZ without TAN.
    const BPD: &str = "HIBPA:5:3:4+12+280:12345678+Testbank+3+1+300'\
        HISALS:6:7:4+1+1+0'\
        HITANS:7:7:4+1+1+1+J:N:0:902:2:TEST902:Decoupled:1.0:Test-App:::Freigabe:0:N:1:N:0:0:N:N:00:0:N:0:0:0:0:N:J'\
        HIPINS:8:1:4+1+1+0+5:20:6:Benutzerkennung:Kunden-ID:HKKAZ:N:HKSAL:N'";
    const UPD: &str = "HIUPA:9:4:4+kunde7+4+0'\
        HIUPD:10:6:4+4711::280:12345678+DE00123456780000004711+kunde7++EUR+Erika Muster++Girokonto++HKSAL:1'";

    const BANK: &str = "12345678";
    const USER: &str = "kunde7";
    const SYSTEM: &str = "SYS-0042";

    fn segments(text: &str) -> Vec<RawSegment> {
        wire::tokenize(text.as_bytes()).unwrap()
    }

    /// Parameters as a dialog leaves them: BPD, UPD, 902 allowed and chosen.
    fn dialog_params() -> BankParams {
        let mut params = BankParams::new();
        params.set_bpd(segments(BPD));
        params.set_upd(segments(UPD));
        params.allowed_security_functions = vec![SecurityFunction::new("902")];
        params.select_security_function();
        params
    }

    fn captured(user: &str, params: &BankParams) -> SessionState {
        SessionState::capture(
            &Blz::new(BANK),
            &UserId::new(user),
            &SystemId::new(SYSTEM),
            params,
        )
    }

    fn sample() -> SessionState {
        captured(USER, &dialog_params())
    }

    fn method_902() -> Option<SecurityFunction> {
        Some(SecurityFunction::new("902"))
    }

    #[test]
    fn state_survives_json_and_gives_the_same_parameters() {
        let state = sample();
        assert_eq!(state.bpd_version, 12);
        assert_eq!(state.upd_version, 4);
        assert_eq!(state.tan_method, method_902());
        assert!(state.can_skip_sync());

        let json = state.to_json().unwrap();
        assert!(
            json.contains("HIBPA:5:3:4+12"),
            "wire form expected: {json}"
        );

        let back = SessionState::from_json(&json).unwrap();
        assert_eq!(back.system_id.as_str(), SYSTEM);
        let params = back.to_params();
        assert_eq!(params.bpd_version, 12);
        assert_eq!(params.upd_version, 4);
        assert_eq!(params.selected_security_function.as_str(), "902");
        assert_eq!(params.tan_methods.len(), 1);
        assert_eq!(params.upd().unwrap().accounts().len(), 1);
        assert_eq!(params.bpd().unwrap().tan_required("HKKAZ"), Some(false));
    }

    #[test]
    fn new_state_asks_for_everything() {
        let state = SessionState::new(&Blz::new(BANK), &UserId::new(USER));
        assert!(!state.can_skip_sync());
        assert!(state.tan_methods().is_empty());
        let params = state.to_params();
        assert_eq!(params.bpd_version, 0);
        assert_eq!(params.upd_version, 0);
    }

    /// Formals C.3.2.3: a UPD of version 0 holds for the dialog that
    /// received it. The dialog sees the accounts; the state forgets them.
    #[test]
    fn upd_version_zero_is_not_kept() {
        let mut params = dialog_params();
        params.set_upd(segments(&UPD.replace("+kunde7+4+0'", "+kunde7+0+0'")));
        let upd = params.upd().unwrap();
        assert!(upd.is_dialog_only());
        assert_eq!(upd.accounts().len(), 1);

        let state = captured(USER, &params);
        assert_eq!(state.upd_version, 0);
        assert!(state.upd().is_none());
        assert!(state.accounts(&Bic::new("TESTDEFFXXX")).is_empty());
        assert_eq!(state.to_params().upd_version, 0);
        // The BPD is not affected.
        assert_eq!(state.bpd().unwrap().version, 12);
    }

    #[test]
    fn new_user_id_from_the_bank_replaces_the_old_one() {
        let mut params = dialog_params();
        params.changed_user_id = Some(UserId::new("kunde8"));
        let state = captured(USER, &params);
        assert_eq!(state.user_id.as_str(), "kunde8");
        assert_eq!(state.previous_user_id, Some(UserId::new(USER)));

        // Reporting the ID that is already in use changes nothing.
        params.changed_user_id = Some(UserId::new(USER));
        let state = captured(USER, &params);
        assert_eq!(state.user_id.as_str(), USER);
        assert_eq!(state.previous_user_id, None);
    }

    #[test]
    fn learned_tan_requirements_are_kept_until_reset() {
        let mut params = dialog_params();
        let hksal = SegmentType::new("HKSAL");
        params.tan_required_learned.insert(hksal.clone());
        let mut state = captured(USER, &params);

        let back = SessionState::from_json(&state.to_json().unwrap()).unwrap();
        assert_eq!(back.tan_required_learned, std::slice::from_ref(&hksal));
        assert!(back.to_params().tan_required_learned.contains(&hksal));

        state.reset();
        assert!(state.tan_required_learned.is_empty());
    }

    #[test]
    fn reset_forgets_the_bank_data_but_not_the_tan_choice() {
        let device = TanMediumName::new("Test-Phone");
        let mut state = sample().with_tan_medium(&device);
        state.reset();
        assert_eq!(state.system_id, SystemId::unassigned());
        assert!(state.bpd().is_none());
        assert!(!state.can_skip_sync());
        assert_eq!(state.tan_method, method_902());
        assert_eq!(state.tan_medium, Some(device));
        assert_eq!(state.user_id.as_str(), USER);
    }

    #[test]
    fn builders_set_what_they_name() {
        let state = SessionState::new(&Blz::new(BANK), &UserId::new(USER))
            .with_system_id(&SystemId::new(SYSTEM))
            .with_tan_method(&SecurityFunction::new("902"));
        assert!(state.system_id.is_assigned());
        assert_eq!(state.tan_method, method_902());
        // A system ID alone is not enough: the BPD is still missing.
        assert!(!state.can_skip_sync());
    }

    /// Bytes 0x80 to 0x9F are control characters in ISO 8859-1
    /// (codec::wire); the stored BPD still gives back the exact bytes.
    #[test]
    fn stored_bpd_keeps_every_byte() {
        let mut bytes = BPD.as_bytes().to_vec();
        let at = bytes.windows(8).position(|w| w == b"Testbank").unwrap();
        bytes[at] = 0x85;
        let mut params = BankParams::new();
        params.set_bpd(wire::tokenize(&bytes).unwrap());
        let state = captured(USER, &params);
        let back = SessionState::from_json(&state.to_json().unwrap()).unwrap();
        assert_eq!(wire::serialize(&back.bpd).unwrap(), bytes);
    }

    #[test]
    fn json_carries_the_format_tag_and_round_trips() {
        let json = sample().to_json().unwrap();
        assert!(json.contains("\"format\":\"fints-client-session/2\""));
        let back = SessionState::from_json(&json).unwrap();
        assert_eq!(back.to_json().unwrap(), json);
    }

    #[test]
    fn other_formats_are_rejected_with_state_format() {
        let json = sample().to_json().unwrap();
        for other in [
            "fints-client-session/1",
            "fints-client-session/3",
            "fints-client-trace/3",
        ] {
            let bad = json.replace(SESSION_STATE_FORMAT, other);
            match SessionState::from_json(&bad) {
                Err(FinTSError::StateFormat { found, expected }) => {
                    assert_eq!((found.as_str(), expected), (other, SESSION_STATE_FORMAT));
                }
                r => panic!("{other}: {r:?}"),
            }
        }
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["format"] = serde_json::json!(1);
        assert!(SessionState::from_json(&v.to_string()).is_err());
        v.as_object_mut().unwrap().remove("format");
        assert!(matches!(
            SessionState::from_json(&v.to_string()),
            Err(FinTSError::StateFormat { .. })
        ));
        let err = FinTSError::StateFormat {
            found: String::new(),
            expected: SESSION_STATE_FORMAT,
        };
        assert_eq!(err.kind(), crate::return_code::Category::ClientFault);
    }

    #[test]
    fn tag_check_holds_outside_from_json() {
        let bad = sample()
            .to_json()
            .unwrap()
            .replace(SESSION_STATE_FORMAT, "fints-client-session/3");
        assert!(serde_json::from_str::<SessionState>(&bad).is_err());
    }

    #[test]
    fn unknown_and_missing_fields_are_rejected() {
        let json = sample().to_json().unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["extra"] = serde_json::json!(1);
        assert!(matches!(
            SessionState::from_json(&v.to_string()),
            Err(FinTSError::Parse(_))
        ));
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("tan_required_learned");
        assert!(matches!(
            SessionState::from_json(&v.to_string()),
            Err(FinTSError::Parse(_))
        ));
    }

    #[test]
    fn debug_hides_user_id_system_id_and_upd() {
        let mut state = sample();
        state.previous_user_id = Some(UserId::new("altkunde3"));
        let out = format!("{state:?}");
        for secret in [USER, SYSTEM, "altkunde3", "DE00", "Erika", "HIUPD"] {
            assert!(!out.contains(secret), "{secret} in {out}");
        }
        assert!(out.contains(BANK) && out.contains("upd_segments"));
    }
}
