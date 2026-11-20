//! The typed bank and user parameter data (BPD, UPD).
//!
//! [`Bpd`] and [`Upd`] are built from the raw parameter segments a bank sends
//! (Formals C.3.2, D, E) and answer the questions the dialog and the jobs ask:
//! which versions of a job the bank offers, whether an order needs a TAN,
//! which TAN methods exist, the SEPA account parameters, which accounts the
//! user has and what the UPD says each one allows.
//!
//! The raw segments stay the source of truth (and what `SessionState`
//! stores); the model is rebuilt from them and is read-only. Every segment is
//! read through its codec definition. Typed parameter details exist for the
//! read jobs (HIPINS, HITANS, HISPAS, HISALS, HIKAZS, HICAZS, HIWPDS); every
//! other job parameter segment is read for its generic header (Formals D.6)
//! only. A segment the codec rejects, or one without even a D.6 header, is
//! kept in `unparsed` with its error (raw fallback).
//!
//! Design: `docs/bpd-upd-design.md`.

use std::collections::BTreeMap;
use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize, Serializer};
use tracing::warn;

use super::Account;
use crate::codec::defs::{AccountLimit2, AllowedOrder2, HiBpa3, HiUpa4, PinTanParameters1};
use crate::codec::{Code, CodecError, ErrorKind, Num, RawSegment, TypedSegment};
use crate::segments::Order;
use crate::segments::read::{self, ParamsHeader};
use crate::types::{
    AccountClass, AccountKey, Bic, Blz, Currency, Iban, SegmentType, SepaAccount, TanMethod,
};

// ── Segment sets and update rules ─────────────────────────────────────────

/// Is this a BPD segment? HIBPA and the general parameter segments (HIKOM,
/// HISHV, HIKPV), and every job parameter segment: `HI…S`, and `DI…S` for
/// the banks' proprietary (DK) jobs.
pub(crate) fn is_bpd_segment(code: &str) -> bool {
    matches!(code, "HIBPA" | "HIKOM" | "HISHV" | "HIKPV") || is_job_parameter_segment(code)
}

/// A job parameter segment (Formals D.6): `HI…S` or `DI…S`, six letters.
fn is_job_parameter_segment(code: &str) -> bool {
    (code.starts_with("HI") || code.starts_with("DI")) && code.len() == 6 && code.ends_with('S')
}

/// The order code a parameter segment describes: `HISALS` → `HKSAL`,
/// `DIPINS` → `DKPIN`.
fn order_code(parameter_code: &str) -> SegmentType {
    let mut code = String::with_capacity(5);
    code.push_str(&parameter_code[..1]);
    code.push('K');
    code.push_str(&parameter_code[2..5]);
    SegmentType::new(code)
}

/// The BPD segment set after a response (Formals C.3.2.2). `None` when the
/// response carries no BPD segment.
///
/// A response with HIBPA replaces the set: the BPD is always sent complete.
/// Parameter segments without HIBPA are merged into `current`, replacing the
/// segment of the same code and version, with a warning: the spec does not
/// foresee this case.
pub(crate) fn bpd_after(
    current: &[RawSegment],
    response: &[RawSegment],
) -> Option<Vec<RawSegment>> {
    let incoming: Vec<RawSegment> = response
        .iter()
        .filter(|s| is_bpd_segment(s.segment_type()))
        .cloned()
        .collect();
    if response.iter().any(|s| s.segment_type() == "HIBPA") {
        return Some(incoming);
    }
    if incoming.is_empty() {
        return None;
    }
    let names: Vec<String> = incoming
        .iter()
        .map(|s| format!("{}:{}", s.segment_type(), s.segment_version()))
        .collect();
    warn!(
        "[FinTS] parameter segments without HIBPA, merged into the BPD: {}",
        names.join(", ")
    );
    let mut merged = current.to_vec();
    for seg in incoming {
        let same = |s: &&mut RawSegment| {
            s.segment_type() == seg.segment_type() && s.segment_version() == seg.segment_version()
        };
        match merged.iter_mut().find(same) {
            Some(slot) => *slot = seg,
            None => merged.push(seg),
        }
    }
    Some(merged)
}

/// The UPD segment set after a response (Formals C.3.2.3): HIUPA and the
/// HIUPDs of the response when it carries HIUPA, else `None` (unchanged).
/// Dropping the UPD on code 3072 is the caller's part, as it depends on the
/// response codes.
pub(crate) fn upd_after(response: &[RawSegment]) -> Option<Vec<RawSegment>> {
    response
        .iter()
        .any(|s| s.segment_type() == "HIUPA")
        .then(|| {
            response
                .iter()
                .filter(|s| matches!(s.segment_type(), "HIUPA" | "HIUPD"))
                .cloned()
                .collect()
        })
}

// ── Errors ────────────────────────────────────────────────────────────────

/// The BPD or UPD could not be built: its header segment is missing or
/// unreadable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParamsError {
    /// No HIBPA (BPD) or HIUPA (UPD) among the segments.
    #[error("no {0} segment")]
    Missing(&'static str),
    /// The header segment is there, but its definition rejects it (or its
    /// version has none).
    #[error("{0} not readable: {1}")]
    Unreadable(&'static str, Box<CodecError>),
}

/// No version of a job is both offered by the bank and implemented by us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct VersionMismatch {
    /// The order code, e.g. `HKSAL`.
    pub code: SegmentType,
    /// The versions the BPD offers, ascending (empty: the job is not in the
    /// BPD).
    pub offered: Vec<u16>,
    /// The versions we implement, as given.
    pub implemented: Vec<u16>,
    /// For a data format (`code` `pain.001`): both lists; `None` for
    /// order versions.
    pub formats: Option<Box<FormatLists>>,
}

/// The data formats of a [`VersionMismatch`] on a format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct FormatLists {
    /// The descriptors the bank offers for the order.
    pub offered: Vec<String>,
    /// The versions this client builds.
    pub implemented: Vec<String>,
}

impl fmt::Display for VersionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |v: &[u16]| {
            if v.is_empty() {
                "none".to_string()
            } else {
                v.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
            }
        };
        if let Some(formats) = &self.formats {
            let offered = if formats.offered.is_empty() {
                "none".to_string()
            } else {
                formats.offered.join(", ")
            };
            return write!(
                f,
                "{}: no common format (bank offers {offered}, client builds {})",
                self.code,
                formats.implemented.join(", ")
            );
        }
        if self.offered.is_empty() {
            return write!(
                f,
                "{}: bank does not offer this order (client implements {})",
                self.code,
                list(&self.implemented)
            );
        }
        write!(
            f,
            "{}: no common version (bank offers {}, client implements {})",
            self.code,
            list(&self.offered),
            list(&self.implemented)
        )
    }
}

impl std::error::Error for VersionMismatch {}

/// A parameter segment that was not (fully) read, with the reason.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct UnparsedParam {
    pub segment: RawSegment,
    #[serde(serialize_with = "display")]
    pub error: CodecError,
}

fn display<T: fmt::Display, S: Serializer>(value: &T, s: S) -> Result<S::Ok, S::Error> {
    s.collect_str(value)
}

// ── BPD ───────────────────────────────────────────────────────────────────

/// The bank parameter data (Formals D).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Bpd {
    /// BPD-Version (HIBPA DE 2).
    pub version: u16,
    /// Länderkennzeichen of the bank (`280` for Germany).
    pub country: String,
    /// Kreditinstitutscode (the BLZ in Germany).
    pub bank_code: Option<Blz>,
    pub bank_name: String,
    /// The most kinds of order per message; `0` means no limit.
    pub max_order_kinds: u16,
    /// Supported dialog languages (1 German, 2 English, 3 French).
    pub languages: Vec<u16>,
    /// Supported HBCI/FinTS versions (`300` for FinTS 3.0).
    pub hbci_versions: Vec<u16>,
    /// Maximale Nachrichtengröße in KiB.
    pub max_message_size: Option<u32>,
    pub min_timeout: Option<u32>,
    pub max_timeout: Option<u32>,
    /// HIPINS: the PIN/TAN parameters. `None` when the BPD has no readable
    /// HIPINS.
    pub pin_tan: Option<PinTanParams>,
    /// HITANS, per version.
    pub tan: TanParams,
    jobs: BTreeMap<SegmentType, JobParams>,
    unparsed: Vec<UnparsedParam>,
}

/// The parameters of one job, per offered version.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct JobParams {
    /// The order code, e.g. `HKSAL` (from `HISALS`).
    pub code: SegmentType,
    /// One entry per parameter segment version.
    pub versions: BTreeMap<u16, JobVersion>,
}

/// One version of a job parameter segment.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct JobVersion {
    /// Maximale Anzahl Aufträge (D.6 DE 2).
    pub max_orders: u16,
    /// Anzahl Signaturen mindestens (D.6 DE 3).
    pub min_signatures: u8,
    /// Sicherheitsklasse (D.6 DE 4); absent in pre-FinTS versions (HISALS 5,
    /// HIKAZS 5, HIWPDS 5). For a segment read header-only it is a guess:
    /// a lone digit 0 to 4 after the signature count.
    pub security_class: Option<u8>,
    pub detail: JobDetail,
}

/// The job-specific parameters of a [`JobVersion`].
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub enum JobDetail {
    /// No job-specific parameters (HISALS 5–7), or kept elsewhere: HIPINS in
    /// [`Bpd::pin_tan`], HITANS in [`Bpd::tan`].
    None,
    /// HISPAS 1–3.
    Sepa(SepaParams),
    /// HIKAZS 5–7.
    Statement(StatementParams),
    /// HICAZS 1.
    Camt(CamtParams),
    /// HISALS 8.
    Balance(BalanceParams),
    /// HIWPDS 5–6.
    Holdings(HoldingsParams),
    /// HIIPZS 1–2.
    Instant(InstantParams),
    /// HIIPSS 1.
    InstantStatus(InstantStatusParams),
    /// HIVPPS 1.
    Vop(VopParams),
    /// Read header-only: the version has no definition, or the definition
    /// rejected it (then it is also in [`Bpd::unparsed`]).
    Raw(RawSegment),
}

/// Parameter SEPA-Kontoverbindung anfordern (HISPAS 1–3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SepaParams {
    pub single_account_allowed: bool,
    /// Nationale Kontoverbindung erlaubt: orders may name the account by
    /// account number and bank code instead of IBAN.
    pub national_account_allowed: bool,
    pub structured_purpose_allowed: bool,
    /// Eingabe Anzahl Einträge erlaubt (from version 2).
    pub max_entries_allowed: Option<bool>,
    /// Anzahl reservierter Verwendungszweckstellen (version 3).
    pub reserved_purpose_places: Option<u16>,
    /// Supported SEPA formats (schema URNs).
    pub formats: Vec<String>,
}

/// Parameter Kontoumsätze (HIKAZS 5–7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct StatementParams {
    /// Speicherzeitraum in days.
    pub days_stored: u32,
    pub max_entries_allowed: bool,
    pub all_accounts_allowed: bool,
}

/// Parameter Kontoumsätze camt (HICAZS 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CamtParams {
    pub days_stored: u32,
    pub max_entries_allowed: bool,
    pub all_accounts_allowed: bool,
    /// Supported camt formats (schema URNs).
    pub formats: Vec<String>,
}

/// Parameter Saldenabfrage (HISALS 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct BalanceParams {
    pub max_entries_allowed: bool,
}

/// Parameter Depotaufstellung (HIWPDS 5–6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct HoldingsParams {
    pub max_entries_allowed: bool,
    pub currency_selectable: bool,
    pub quality_selectable: bool,
}

/// Parameter SEPA-Instant Payment Zahlung (HIIPZS 1–2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct InstantParams {
    /// Umwandlung nach SEPA-Überweisung zulässig erlaubt: the order may ask
    /// the bank to convert an undeliverable instant payment into a standard
    /// transfer. `false` in version 1, which has no such element.
    pub conversion_allowed: bool,
    /// Zulässige Purpose codes. Empty: no restriction announced.
    pub purpose_codes: Vec<String>,
    /// Supported pain.001 formats (schema URNs or file names). Empty: the
    /// order names none (HISPAS applies).
    pub formats: Vec<String>,
}

/// Parameter SEPA-Instant Payment Status (HIIPSS 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct InstantStatusParams {
    /// Mindestwartezeit before the first status request (seconds).
    pub min_wait: u32,
    /// Supported SEPA formats. Empty: HISPAS applies.
    pub formats: Vec<String>,
}

/// How the bank delivers the payee check result over several requests
/// (VoP 1.02, D, "Art der Lieferung Payment Status Report").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum VopDelivery {
    /// `S`: each answer carries only the newly available entries.
    Stepwise,
    /// `V`: each answer carries everything delivered so far.
    Complete,
    /// Another code: read like [`VopDelivery::Complete`] (the last answer
    /// is the result), so the rest of HIVPPS, the orders that require the
    /// check above all, still counts.
    Other(String),
}

/// Parameter Namensabgleich Prüfauftrag (HIVPPS 1; VoP 1.02, D).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct VopParams {
    /// Maximale Anzahl CreditTransferTransactionInformation; `0`: unlimited.
    pub max_transactions: u32,
    /// Aufklärungstext strukturiert: the explanation text may carry markup.
    pub explanation_structured: bool,
    pub delivery: VopDelivery,
    pub bulk_with_one_order_allowed: bool,
    pub max_entries_input_allowed: bool,
    /// Unterstützte Payment Status Report Datenformate (pain.002; the
    /// element's `;`-separated list split up).
    pub report_formats: Vec<String>,
    /// VOP-pflichtige Zahlungsverkehrsaufträge: the orders that must go with
    /// an HKVPP.
    pub required_for: Vec<SegmentType>,
}

impl VopParams {
    /// Whether the bank requires the payee check for `order` (a code such
    /// as `HKCCS`).
    pub fn requires(&self, order: &str) -> bool {
        self.required_for.iter().any(|c| c.as_str() == order)
    }
}

/// What the BPD says about instant payments: HIIPZS and HIIPSS, summarised
/// ([`Bpd::instant`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct InstantSupport {
    /// The HIIPZS versions the bank offers, ascending.
    pub versions: Vec<u16>,
    /// The parameters of the highest HIIPZS version.
    pub payment: InstantParams,
    /// HIIPSS, when the bank offers the status request.
    pub status: Option<InstantStatusParams>,
}

/// PIN/TAN-spezifische Informationen (HIPINS 1, PIN/TAN B.8.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PinTanParams {
    pub min_pin_length: Option<u8>,
    pub max_pin_length: Option<u8>,
    pub max_tan_length: Option<u8>,
    pub user_id_text: Option<String>,
    pub customer_id_text: Option<String>,
    /// TAN erforderlich, per order code. An order without an entry may not
    /// be secured with PIN/TAN.
    pub tan_required: BTreeMap<SegmentType, bool>,
}

/// The TAN parameters (HITANS), per version.
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct TanParams {
    pub versions: BTreeMap<u16, TanVersion>,
}

/// One HITANS version.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct TanVersion {
    /// Einschritt-Verfahren erlaubt. `None` for a version above 7, whose
    /// methods are read by the remainder parser.
    pub one_step_allowed: Option<bool>,
    pub methods: Vec<TanMethod>,
}

impl TanParams {
    /// The HKTAN versions the bank offers, ascending.
    pub fn hktan_versions(&self) -> impl Iterator<Item = u16> + '_ {
        self.versions.keys().copied()
    }

    /// All TAN methods, in ascending HITANS version.
    pub fn methods(&self) -> impl Iterator<Item = &TanMethod> {
        self.versions.values().flat_map(|v| &v.methods)
    }
}

impl JobParams {
    /// The offered versions, ascending.
    pub fn offered(&self) -> impl Iterator<Item = u16> + '_ {
        self.versions.keys().copied()
    }

    /// The highest offered version.
    pub fn highest(&self) -> Option<(u16, &JobVersion)> {
        self.versions.iter().next_back().map(|(v, j)| (*v, j))
    }
}

impl Bpd {
    /// Builds the BPD from its segments (other segments are ignored). Needs a
    /// readable HIBPA; every other problem goes to [`Bpd::unparsed`].
    pub fn from_segments(segments: &[RawSegment]) -> Result<Bpd, ParamsError> {
        let hibpa = segments
            .iter()
            .find(|s| s.segment_type() == "HIBPA")
            .ok_or(ParamsError::Missing("HIBPA"))?;
        let header: HiBpa3 = match read::typed(hibpa) {
            Ok((_, TypedSegment::HiBpa3(h))) => h,
            Ok(_) => unreachable!("HIBPA reads as HIBPA"),
            Err(e) => return Err(ParamsError::Unreadable("HIBPA", Box::new(e))),
        };
        let num32 = |n: Option<Num>| n.and_then(|n| u32::try_from(n.value()).ok());
        let mut bpd = Bpd {
            version: u16_of(header.bpd_version),
            country: header.bank.country.as_str().to_string(),
            bank_code: header.bank.bank_code.as_ref().map(|c| Blz::new(c.as_str())),
            bank_name: header.bank_name.as_str().to_string(),
            max_order_kinds: u16_of(header.max_order_kinds),
            languages: header
                .languages
                .languages
                .iter()
                .filter_map(|c| c.as_str().parse().ok())
                .collect(),
            hbci_versions: header
                .hbci_versions
                .versions
                .iter()
                .map(|n| u16_of(*n))
                .collect(),
            max_message_size: num32(header.max_message_size),
            min_timeout: num32(header.min_timeout),
            max_timeout: num32(header.max_timeout),
            pin_tan: None,
            tan: TanParams::default(),
            jobs: BTreeMap::new(),
            unparsed: Vec::new(),
        };
        for seg in segments
            .iter()
            .filter(|s| is_job_parameter_segment(s.segment_type()))
        {
            bpd.add_job_segment(seg);
        }
        Ok(bpd)
    }

    fn add_job_segment(&mut self, seg: &RawSegment) {
        let version = seg.segment_version();
        let job = match read::typed(seg) {
            Ok((_, typed)) => match self.typed_job(seg, typed) {
                Some(job) => job,
                None => header_only(seg),
            },
            Err(error) => {
                let undefined = error.kind == ErrorKind::UnknownSegment;
                if undefined && seg.segment_type() == "HITANS" {
                    // A version above 7: the remainder parser reads the methods.
                    self.tan.versions.insert(
                        version,
                        TanVersion {
                            one_step_allowed: None,
                            methods: read::tan_methods(seg),
                        },
                    );
                }
                if !undefined {
                    read::report(&error);
                }
                let job =
                    read::params_header(seg).map(|h| job_version(h, JobDetail::Raw(seg.clone())));
                if !undefined || job.is_none() {
                    self.unparsed.push(UnparsedParam {
                        segment: seg.clone(),
                        error,
                    });
                }
                match job {
                    Some(job) => job,
                    None => return,
                }
            }
        };
        let code = order_code(seg.segment_type());
        self.jobs
            .entry(code.clone())
            .or_insert_with(|| JobParams {
                code,
                versions: BTreeMap::new(),
            })
            .versions
            .insert(version, job);
    }

    /// The job version of a segment its definition read. `None` for a typed
    /// segment this model has no mapping for (read header-only instead).
    fn typed_job(&mut self, seg: &RawSegment, typed: TypedSegment) -> Option<JobVersion> {
        use JobDetail as D;
        use TypedSegment as T;
        macro_rules! head {
            ($s:expr) => {
                ParamsHeader {
                    max_orders: u16_of($s.max_orders),
                    min_signatures: u8_of($s.min_signatures),
                    security_class: class(&$s.security_class),
                }
            };
        }
        macro_rules! head_pre_fints {
            ($s:expr) => {
                ParamsHeader {
                    max_orders: u16_of($s.max_orders),
                    min_signatures: u8_of($s.min_signatures),
                    security_class: None,
                }
            };
        }
        macro_rules! tans {
            ($s:expr) => {{
                self.tan.versions.insert(
                    seg.segment_version(),
                    TanVersion {
                        one_step_allowed: Some($s.parameters.one_step_allowed.value()),
                        methods: read::tan_methods(seg),
                    },
                );
                job_version(head!($s), D::None)
            }};
        }
        let statement = |p: &crate::codec::defs::TurnoverParameters2| {
            D::Statement(StatementParams {
                days_stored: u32_of(p.storage_period),
                max_entries_allowed: p.max_entries_allowed.value(),
                all_accounts_allowed: p.all_accounts_allowed.value(),
            })
        };
        let holdings = |p: &crate::codec::defs::DepotListParameters2| {
            D::Holdings(HoldingsParams {
                max_entries_allowed: p.max_entries_allowed.value(),
                currency_selectable: p.currency_selectable.value(),
                quality_selectable: p.quality_selectable.value(),
            })
        };
        let texts = |v: &[crate::codec::An]| v.iter().map(|f| f.as_str().to_string()).collect();
        // One data element holding a list: split, trimmed, empty parts dropped.
        let list = |a: Option<&crate::codec::An>, sep: char| -> Vec<String> {
            a.map(|a| {
                a.as_str()
                    .split(sep)
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
        };
        Some(match typed {
            T::HiPins1(s) => {
                self.pin_tan = Some(pin_tan(&s.parameters));
                job_version(head!(s), D::None)
            }
            T::HiTans1(s) => tans!(s),
            T::HiTans2(s) => tans!(s),
            T::HiTans3(s) => tans!(s),
            T::HiTans4(s) => tans!(s),
            T::HiTans5(s) => tans!(s),
            T::HiTans6(s) => tans!(s),
            T::HiTans7(s) => tans!(s),
            T::HiSpas1(s) => {
                let p = &s.parameters;
                let detail = D::Sepa(SepaParams {
                    single_account_allowed: p.single_account_allowed.value(),
                    national_account_allowed: p.national_account_allowed.value(),
                    structured_purpose_allowed: p.structured_purpose_allowed.value(),
                    max_entries_allowed: None,
                    reserved_purpose_places: None,
                    formats: texts(&p.formats),
                });
                job_version(head!(s), detail)
            }
            T::HiSpas2(s) => {
                let p = &s.parameters;
                let detail = D::Sepa(SepaParams {
                    single_account_allowed: p.single_account_allowed.value(),
                    national_account_allowed: p.national_account_allowed.value(),
                    structured_purpose_allowed: p.structured_purpose_allowed.value(),
                    max_entries_allowed: Some(p.max_entries_allowed.value()),
                    reserved_purpose_places: None,
                    formats: texts(&p.formats),
                });
                job_version(head!(s), detail)
            }
            T::HiSpas3(s) => {
                let p = &s.parameters;
                let detail = D::Sepa(SepaParams {
                    single_account_allowed: p.single_account_allowed.value(),
                    national_account_allowed: p.national_account_allowed.value(),
                    structured_purpose_allowed: p.structured_purpose_allowed.value(),
                    max_entries_allowed: Some(p.max_entries_allowed.value()),
                    reserved_purpose_places: Some(u16_of(p.reserved_purpose_places)),
                    formats: texts(&p.formats),
                });
                job_version(head!(s), detail)
            }
            T::HiSals5(s) => job_version(head_pre_fints!(s), D::None),
            T::HiSals6(s) => job_version(head!(s), D::None),
            T::HiSals7(s) => job_version(head!(s), D::None),
            T::HiSals8(s) => {
                let detail = D::Balance(BalanceParams {
                    max_entries_allowed: s.parameters.max_entries_allowed.value(),
                });
                job_version(head!(s), detail)
            }
            T::HiKazs5(s) => job_version(head_pre_fints!(s), statement(&s.parameters)),
            T::HiKazs6(s) => job_version(head!(s), statement(&s.parameters)),
            T::HiKazs7(s) => job_version(head!(s), statement(&s.parameters)),
            T::HiCazs1(s) => {
                let p = &s.parameters;
                let detail = D::Camt(CamtParams {
                    days_stored: u32_of(p.storage_period),
                    max_entries_allowed: p.entry_count_allowed.value(),
                    all_accounts_allowed: p.all_accounts_allowed.value(),
                    formats: texts(&p.formats),
                });
                job_version(head!(s), detail)
            }
            T::HiCcss1(s) => job_version(head!(s), D::None),
            T::HiVpas1(s) => job_version(head!(s), D::None),
            T::HiIpzs1(s) => {
                let p = &s.parameters;
                let detail = D::Instant(InstantParams {
                    conversion_allowed: false,
                    purpose_codes: list(p.purpose_codes.as_ref(), ','),
                    formats: texts(&p.formats),
                });
                job_version(head!(s), detail)
            }
            T::HiIpzs2(s) => {
                let p = &s.parameters;
                let detail = D::Instant(InstantParams {
                    conversion_allowed: p.conversion_allowed.value(),
                    purpose_codes: list(p.purpose_codes.as_ref(), ','),
                    formats: texts(&p.formats),
                });
                job_version(head!(s), detail)
            }
            T::HiIpss1(s) => {
                let p = &s.parameters;
                let detail = D::InstantStatus(InstantStatusParams {
                    min_wait: u32_of(p.min_wait),
                    formats: texts(&p.formats),
                });
                job_version(head!(s), detail)
            }
            T::HiVpps1(s) => {
                let p = &s.parameters;
                let delivery = match p.delivery.as_str() {
                    "S" => VopDelivery::Stepwise,
                    "V" => VopDelivery::Complete,
                    other => {
                        warn!("[FinTS] HIVPPS: unknown delivery {other:?}, read as complete");
                        VopDelivery::Other(other.to_string())
                    }
                };
                let detail = D::Vop(VopParams {
                    max_transactions: u32_of(p.max_transactions),
                    explanation_structured: p.explanation_structured.value(),
                    delivery,
                    bulk_with_one_order_allowed: p.bulk_with_one_order_allowed.value(),
                    max_entries_input_allowed: p.max_entries_input_allowed.value(),
                    report_formats: list(Some(&p.report_formats), ';'),
                    required_for: p
                        .required_for
                        .iter()
                        .map(|c| SegmentType::new(c.as_str()))
                        .collect(),
                });
                job_version(head!(s), detail)
            }
            T::HiWpds5(s) => job_version(head_pre_fints!(s), holdings(&s.parameters)),
            T::HiWpds6(s) => job_version(head!(s), holdings(&s.parameters)),
            _ => return None,
        })
    }

    /// The parameters of a job by its order code (`HKSAL`).
    pub fn job(&self, code: &str) -> Option<&JobParams> {
        self.jobs.get(&SegmentType::new(code))
    }

    /// All jobs the BPD lists, by order code.
    pub fn jobs(&self) -> impl Iterator<Item = &JobParams> {
        self.jobs.values()
    }

    /// Parameter segments that were not (fully) read.
    pub fn unparsed(&self) -> &[UnparsedParam] {
        &self.unparsed
    }

    /// The highest version of job `code` that the bank offers and we
    /// implement.
    pub fn highest_common(&self, code: &str, implemented: &[u16]) -> Result<u16, VersionMismatch> {
        let offered: Vec<u16> = self
            .job(code)
            .map(|j| j.offered().collect())
            .unwrap_or_default();
        offered
            .iter()
            .rev()
            .copied()
            .find(|v| implemented.contains(v))
            .ok_or_else(|| VersionMismatch {
                code: SegmentType::new(code),
                offered,
                implemented: implemented.to_vec(),
                formats: None,
            })
    }

    /// Whether order `code` needs a TAN (HIPINS). `None` when the BPD has no
    /// HIPINS or HIPINS does not list the order.
    pub fn tan_required(&self, code: &str) -> Option<bool> {
        self.pin_tan
            .as_ref()?
            .tan_required
            .get(&SegmentType::new(code))
            .copied()
    }

    /// All TAN methods (HITANS), in ascending version.
    pub fn tan_methods(&self) -> impl Iterator<Item = &TanMethod> {
        self.tan.methods()
    }

    /// The SEPA account parameters of the highest HISPAS version.
    pub fn sepa(&self) -> Option<&SepaParams> {
        self.job("HKSPA")?
            .versions
            .values()
            .rev()
            .find_map(|j| match &j.detail {
                JobDetail::Sepa(s) => Some(s),
                _ => None,
            })
    }

    /// The payee check (VoP) parameters of the highest HIVPPS version.
    /// `None` when the bank offers no HKVPP: its payments go without a
    /// check.
    pub fn vop(&self) -> Option<&VopParams> {
        self.job("HKVPP")?
            .versions
            .values()
            .rev()
            .find_map(|j| match &j.detail {
                JobDetail::Vop(p) => Some(p),
                _ => None,
            })
    }

    /// The pain.001 formats (schema URNs or file names) the bank accepts for
    /// `order`: the HIIPZS or HIIPSS formats of the highest version for the
    /// instant orders, else (and when the instant parameters name none) the
    /// HISPAS formats; the HISPAS formats for [`Order::Transfer`]. Empty for
    /// an order that carries no pain message, or without a readable
    /// HISPAS.
    pub fn sepa_formats(&self, order: Order) -> &[String] {
        let own: &[String] = match order {
            Order::InstantTransfer => self.instant_params().map_or(&[], |p| &p.formats),
            Order::InstantStatus => self
                .instant_status_params()
                .map_or(&[], |p| p.formats.as_slice()),
            Order::Transfer => &[],
            _ => return &[],
        };
        if own.is_empty() {
            self.sepa().map_or(&[], |s| s.formats.as_slice())
        } else {
            own
        }
    }

    /// Instant payments (HIIPZS, HIIPSS). `None` when the bank offers no
    /// HKIPZ.
    pub fn instant(&self) -> Option<InstantSupport> {
        let job = self.job("HKIPZ")?;
        let payment = self.instant_params()?.clone();
        Some(InstantSupport {
            versions: job.offered().collect(),
            payment,
            status: self.instant_status_params().cloned(),
        })
    }

    fn instant_params(&self) -> Option<&InstantParams> {
        self.job("HKIPZ")?
            .versions
            .values()
            .rev()
            .find_map(|j| match &j.detail {
                JobDetail::Instant(p) => Some(p),
                _ => None,
            })
    }

    fn instant_status_params(&self) -> Option<&InstantStatusParams> {
        self.job("HKIPS")?
            .versions
            .values()
            .rev()
            .find_map(|j| match &j.detail {
                JobDetail::InstantStatus(p) => Some(p),
                _ => None,
            })
    }

    /// The camt formats of the highest HICAZS version; empty without one.
    pub fn camt_formats(&self) -> &[String] {
        self.job("HKCAZ")
            .and_then(|j| {
                j.versions.values().rev().find_map(|j| match &j.detail {
                    JobDetail::Camt(c) => Some(c.formats.as_slice()),
                    _ => None,
                })
            })
            .unwrap_or_default()
    }
}

fn job_version(h: ParamsHeader, detail: JobDetail) -> JobVersion {
    JobVersion {
        max_orders: h.max_orders,
        min_signatures: h.min_signatures,
        security_class: h.security_class,
        detail,
    }
}

/// A typed segment without a mapping in this model: its generic header.
fn header_only(seg: &RawSegment) -> JobVersion {
    match read::params_header(seg) {
        Some(h) => job_version(h, JobDetail::Raw(seg.clone())),
        None => JobVersion {
            max_orders: 0,
            min_signatures: 0,
            security_class: None,
            detail: JobDetail::Raw(seg.clone()),
        },
    }
}

fn pin_tan(p: &PinTanParameters1) -> PinTanParams {
    let len = |n: Option<Num>| n.map(u8_of);
    let text =
        |a: Option<&crate::codec::An>| a.map(|a| a.as_str().to_string()).filter(|s| !s.is_empty());
    PinTanParams {
        min_pin_length: len(p.min_pin_length),
        max_pin_length: len(p.max_pin_length),
        max_tan_length: len(p.max_tan_length),
        user_id_text: text(p.user_id_text.as_ref()),
        customer_id_text: text(p.customer_id_text.as_ref()),
        tan_required: p
            .orders
            .iter()
            .map(|o| {
                (
                    SegmentType::new(o.segment_code.as_str()),
                    o.tan_required.value(),
                )
            })
            .collect(),
    }
}

fn u8_of(n: Num) -> u8 {
    u8::try_from(n.value()).unwrap_or(u8::MAX)
}

fn u16_of(n: Num) -> u16 {
    u16::try_from(n.value()).unwrap_or(u16::MAX)
}

fn u32_of(n: Num) -> u32 {
    u32::try_from(n.value()).unwrap_or(u32::MAX)
}

fn class(c: &Code) -> Option<u8> {
    c.as_str().parse().ok()
}

// ── UPD ───────────────────────────────────────────────────────────────────

/// The user parameter data (Formals E).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct Upd {
    /// UPD-Version (HIUPA DE 3). `0` in answer to our version 0: the UPD is
    /// valid for this dialog only (Formals C.3.2.3), see
    /// [`Upd::is_dialog_only`].
    pub version: u16,
    /// UPD-Verwendung (HIUPA DE 4).
    pub usage: UpdUsage,
    pub accounts: Vec<UpdAccount>,
    unparsed: Vec<UnparsedParam>,
}

/// UPD-Verwendung: what the UPD says about orders it does not list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum UpdUsage {
    /// `0`: orders not listed for an account are blocked.
    Closed,
    /// `1`: nothing can be said about unlisted orders (also used for an
    /// unknown code).
    Open,
}

/// One account of the UPD (HIUPD 4 or 6; other versions by the remainder
/// parser, which reads no customer ID, limit or allowed orders).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct UpdAccount {
    pub account_number: Option<String>,
    pub sub_account: Option<String>,
    pub blz: Option<Blz>,
    pub iban: Option<Iban>,
    /// Kunden-ID; empty for an account read by the remainder parser.
    pub customer_id: String,
    /// Kontoart (HIUPD 6), see [`UpdAccount::class`].
    pub account_type: Option<u8>,
    pub currency: Option<Currency>,
    /// Name des Kontoinhabers 1 and 2, those given.
    pub owner: Vec<String>,
    pub product_name: Option<String>,
    pub limit: Option<AccountLimit>,
    /// Erlaubte Geschäftsvorfälle, by order code.
    pub allowed: BTreeMap<SegmentType, AllowedJob>,
    /// A BIC the remainder parser found in the segment; HIUPD 4 and 6
    /// carry none.
    pub bic: Option<Bic>,
}

/// One entry of Erlaubte Geschäftsvorfälle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct AllowedJob {
    /// Anzahl benötigter Signaturen.
    pub min_signatures: u8,
    pub limit: Option<AccountLimit>,
}

/// Kontolimit, or the limit of one allowed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountLimit {
    pub kind: LimitKind,
    pub amount: Option<Decimal>,
    pub currency: Option<Currency>,
    /// Limit-Tage, for [`LimitKind::Period`].
    pub days: Option<u16>,
}

/// Limitart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum LimitKind {
    /// `E`: per order.
    SingleOrder,
    /// `T`: per day.
    Day,
    /// `W`: per week.
    Week,
    /// `M`: per month.
    Month,
    /// `Z`: per period of [`AccountLimit::days`].
    Period,
    /// A code the spec does not define.
    Other(String),
}

/// What the UPD says about an order for an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum Allowance {
    /// The account lists the order.
    Yes,
    /// The account does not list it, and the UPD usage is closed.
    No,
    /// The account does not list it, and the UPD usage is open.
    Unknown,
}

impl Upd {
    /// Builds the UPD from its segments (other segments are ignored). Needs
    /// a readable HIUPA; an unreadable HIUPD goes to [`Upd::unparsed`].
    pub fn from_segments(segments: &[RawSegment]) -> Result<Upd, ParamsError> {
        let hiupa = segments
            .iter()
            .find(|s| s.segment_type() == "HIUPA")
            .ok_or(ParamsError::Missing("HIUPA"))?;
        let header: HiUpa4 = match read::typed(hiupa) {
            Ok((_, TypedSegment::HiUpa4(h))) => h,
            Ok(_) => unreachable!("HIUPA reads as HIUPA"),
            Err(e) => return Err(ParamsError::Unreadable("HIUPA", Box::new(e))),
        };
        let usage = match header.upd_usage.as_str() {
            "0" => UpdUsage::Closed,
            "1" => UpdUsage::Open,
            other => {
                warn!("[FinTS] unknown UPD usage {other:?}, taken as open");
                UpdUsage::Open
            }
        };
        let mut upd = Upd {
            version: u16_of(header.upd_version),
            usage,
            accounts: Vec::new(),
            unparsed: Vec::new(),
        };
        for seg in segments.iter().filter(|s| s.segment_type() == "HIUPD") {
            match read::typed(seg) {
                Ok((_, TypedSegment::HiUpd4(s))) => upd.accounts.push(UpdAccount {
                    account_number: some(s.account.account_number.as_str()),
                    sub_account: s.account.subaccount.as_ref().and_then(|a| some(a.as_str())),
                    blz: s
                        .account
                        .bank
                        .bank_code
                        .as_ref()
                        .and_then(|b| some(b.as_str()))
                        .map(Blz::new),
                    iban: None,
                    customer_id: s.customer_id.as_str().to_string(),
                    account_type: None,
                    currency: s.currency.as_ref().map(|c| Currency::new(c.as_str())),
                    owner: owners(&s.holder_name_1, s.holder_name_2.as_ref()),
                    product_name: s.product_name.as_ref().and_then(|p| some(p.as_str())),
                    limit: s.limit.as_ref().map(account_limit),
                    allowed: allowed(&s.allowed_orders),
                    bic: None,
                }),
                Ok((_, TypedSegment::HiUpd6(s))) => {
                    let k = s.account.as_ref();
                    upd.accounts.push(UpdAccount {
                        account_number: k.and_then(|k| some(k.account_number.as_str())),
                        sub_account: k
                            .and_then(|k| k.subaccount.as_ref())
                            .and_then(|a| some(a.as_str())),
                        blz: k
                            .and_then(|k| k.bank.bank_code.as_ref())
                            .and_then(|b| some(b.as_str()))
                            .map(Blz::new),
                        iban: s
                            .iban
                            .as_ref()
                            .and_then(|i| some(i.as_str()))
                            .map(Iban::new),
                        customer_id: s.customer_id.as_str().to_string(),
                        account_type: s.account_type.and_then(|t| u8::try_from(t.value()).ok()),
                        currency: s.currency.as_ref().map(|c| Currency::new(c.as_str())),
                        owner: owners(&s.holder_name_1, s.holder_name_2.as_ref()),
                        product_name: s.product_name.as_ref().and_then(|p| some(p.as_str())),
                        limit: s.limit.as_ref().map(account_limit),
                        allowed: allowed(&s.allowed_orders),
                        bic: None,
                    });
                }
                Ok(_) => {}
                Err(e) if e.kind == ErrorKind::UnknownSegment => {
                    // HIUPD 5 (no published table): the remainder parser.
                    match crate::segments::response::parse_hiupd(seg) {
                        Some(sepa) => upd.accounts.push(UpdAccount::from_remainder(sepa)),
                        None => upd.unparsed.push(UnparsedParam {
                            segment: seg.clone(),
                            error: e,
                        }),
                    }
                }
                Err(e) => {
                    read::report(&e);
                    upd.unparsed.push(UnparsedParam {
                        segment: seg.clone(),
                        error: e,
                    });
                }
            }
        }
        Ok(upd)
    }

    /// Version 0: the bank sent the UPD for this dialog only; it must not
    /// be stored, so the next dialog asks again.
    pub fn is_dialog_only(&self) -> bool {
        self.version == 0
    }

    /// The accounts.
    pub fn accounts(&self) -> &[UpdAccount] {
        &self.accounts
    }

    /// HIUPD segments that were not read.
    pub fn unparsed(&self) -> &[UnparsedParam] {
        &self.unparsed
    }

    /// What the UPD says about order `code` (`HKSAL`) for `account`. For
    /// information only: the bank's answer decides.
    pub fn allows(&self, account: &UpdAccount, code: &str) -> Allowance {
        if account.allowed.contains_key(&SegmentType::new(code)) {
            Allowance::Yes
        } else if self.usage == UpdUsage::Closed {
            Allowance::No
        } else {
            Allowance::Unknown
        }
    }
}

impl UpdAccount {
    fn from_remainder(sepa: SepaAccount) -> UpdAccount {
        UpdAccount {
            account_number: some(&sepa.account_number),
            sub_account: some(&sepa.sub_account),
            blz: some(sepa.blz.as_str()).map(Blz::new),
            iban: some(sepa.iban.as_str()).map(Iban::new),
            customer_id: String::new(),
            account_type: sepa.account_type,
            currency: sepa.currency,
            owner: sepa.owner.into_iter().collect(),
            product_name: sepa.product_name,
            limit: None,
            allowed: BTreeMap::new(),
            bic: some(sepa.bic.as_str()).map(Bic::new),
        }
    }

    /// The account class from the Kontoart, if given and in range.
    pub fn class(&self) -> Option<AccountClass> {
        AccountClass::from_account_type(self.account_type?)
    }

    /// The account's identity: its KTV (own BLZ, number, subaccount), or
    /// IBAN and subaccount when the entry has no number or bank code.
    /// `None` when it has neither.
    pub fn key(&self) -> Option<AccountKey> {
        let sub = self.sub_account.as_deref();
        match (&self.blz, &self.account_number) {
            (Some(blz), Some(number)) => AccountKey::national(blz.as_str(), number, sub),
            _ => None,
        }
        .or_else(|| AccountKey::iban(self.iban.as_ref()?.as_str(), sub))
    }

    /// Whether `account` names this entry exactly: the same subaccount,
    /// and the same number (and BLZ, when the account has one) or, for an
    /// entry keyed by IBAN, the same IBAN.
    fn is(&self, account: &Account) -> bool {
        match self.key() {
            Some(AccountKey::National {
                blz,
                number,
                subaccount,
            }) => {
                let same_blz = account.blz().is_none_or(|b| b == blz.as_str());
                let mine = account
                    .account_number()
                    .and_then(|n| AccountKey::national(blz.as_str(), n, account.subaccount()));
                same_blz
                    && mine
                        == Some(AccountKey::National {
                            blz,
                            number,
                            subaccount,
                        })
            }
            Some(key @ AccountKey::Iban { .. }) => {
                AccountKey::iban(account.iban(), account.subaccount()) == Some(key)
            }
            None => false,
        }
    }

    /// Whether `account` shares the IBAN or the number with this entry,
    /// whatever the subaccount.
    fn resembles(&self, account: &Account) -> bool {
        let number = |n: &str| n.trim_start_matches('0').to_string();
        let by_iban = !account.iban().is_empty()
            && self
                .iban
                .as_ref()
                .is_some_and(|i| i.as_str() == account.iban());
        let by_number = account.account_number().is_some_and(|n| {
            self.account_number
                .as_deref()
                .is_some_and(|m| number(m) == number(n))
        });
        by_iban || by_number
    }
}

impl Upd {
    /// The entry `account` names: the one it names exactly (the
    /// [`UpdAccount::key`], subaccount included), else the only one that
    /// shares its IBAN or number (an account built by hand from an IBAN
    /// has no subaccount). `None` when several share them and none is
    /// named exactly.
    pub fn find(&self, account: &Account) -> Option<&UpdAccount> {
        if let Some(exact) = self.accounts.iter().find(|a| a.is(account)) {
            return Some(exact);
        }
        let mut alike = self.accounts.iter().filter(|a| a.resembles(account));
        match (alike.next(), alike.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    }
}

fn some(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn owners(first: &crate::codec::An, second: Option<&crate::codec::An>) -> Vec<String> {
    std::iter::once(first)
        .chain(second)
        .filter_map(|n| some(n.as_str()))
        .collect()
}

fn limit_kind(code: &str) -> LimitKind {
    match code {
        "E" => LimitKind::SingleOrder,
        "T" => LimitKind::Day,
        "W" => LimitKind::Week,
        "M" => LimitKind::Month,
        "Z" => LimitKind::Period,
        other => LimitKind::Other(other.to_string()),
    }
}

fn account_limit(l: &AccountLimit2) -> AccountLimit {
    AccountLimit {
        kind: limit_kind(l.kind.as_str()),
        amount: l.amount.as_ref().map(|b| b.value.value()),
        currency: l
            .amount
            .as_ref()
            .map(|b| Currency::new(b.currency.as_str())),
        days: l.days.map(u16_of),
    }
}

fn allowed(orders: &[AllowedOrder2]) -> BTreeMap<SegmentType, AllowedJob> {
    orders
        .iter()
        .map(|o| {
            let limit = o.limit_kind.as_ref().map(|k| AccountLimit {
                kind: limit_kind(k.as_str()),
                amount: o.limit_amount.as_ref().map(|b| b.value.value()),
                currency: o
                    .limit_amount
                    .as_ref()
                    .map(|b| Currency::new(b.currency.as_str())),
                days: o.limit_days.map(u16_of),
            });
            (
                SegmentType::new(o.segment_code.as_str()),
                AllowedJob {
                    min_signatures: u8_of(o.signatures_required),
                    limit,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(wire: &[&str]) -> Vec<RawSegment> {
        wire.iter()
            .map(|s| {
                crate::codec::wire::tokenize(s.as_bytes())
                    .unwrap()
                    .remove(0)
            })
            .collect()
    }

    const HIBPA: &str = "HIBPA:5:3:4+12+280:12345678+Testbank+3+1:2+300+100+10+120'";

    fn bpd(rest: &[&str]) -> Bpd {
        let mut all = vec![HIBPA];
        all.extend_from_slice(rest);
        Bpd::from_segments(&segs(&all)).unwrap()
    }

    #[test]
    fn hibpa_fields() {
        let b = bpd(&[]);
        assert_eq!(b.version, 12);
        assert_eq!(b.country, "280");
        assert_eq!(b.bank_code, Some(Blz::new("12345678")));
        assert_eq!(b.bank_name, "Testbank");
        assert_eq!(b.max_order_kinds, 3);
        assert_eq!(b.languages, [1, 2]);
        assert_eq!(b.hbci_versions, [300]);
        assert_eq!(
            (b.max_message_size, b.min_timeout, b.max_timeout),
            (Some(100), Some(10), Some(120))
        );
        assert!(b.pin_tan.is_none() && b.jobs().next().is_none());
    }

    #[test]
    fn missing_or_unreadable_header_is_an_error() {
        assert_eq!(
            Bpd::from_segments(&segs(&["HISALS:6:6:4+1+1+0'"])).unwrap_err(),
            ParamsError::Missing("HIBPA")
        );
        assert!(matches!(
            Bpd::from_segments(&segs(&["HIBPA:5:3:4+x+280:1+T+3+1+300'"])).unwrap_err(),
            ParamsError::Unreadable("HIBPA", _)
        ));
        assert_eq!(
            Upd::from_segments(&segs(&[])).unwrap_err(),
            ParamsError::Missing("HIUPA")
        );
        assert!(matches!(
            Upd::from_segments(&segs(&["HIUPA:5:4:4+USER+7+0x'"])).unwrap_err(),
            ParamsError::Unreadable("HIUPA", _)
        ));
    }

    #[test]
    fn several_versions_of_one_job() {
        let b = bpd(&[
            "HISALS:6:5:4+1+1'",
            "HISALS:7:6:4+1+2+3'",
            "HISALS:8:8:4+5+1+1+J'",
        ]);
        let sal = b.job("HKSAL").unwrap();
        assert_eq!(sal.offered().collect::<Vec<_>>(), [5, 6, 8]);
        let v5 = &sal.versions[&5];
        assert_eq!(
            (v5.max_orders, v5.min_signatures, v5.security_class),
            (1, 1, None)
        );
        assert!(matches!(v5.detail, JobDetail::None));
        let v6 = &sal.versions[&6];
        assert_eq!((v6.min_signatures, v6.security_class), (2, Some(3)));
        let (v, v8) = sal.highest().unwrap();
        assert_eq!((v, v8.max_orders), (8, 5));
        assert!(matches!(
            v8.detail,
            JobDetail::Balance(BalanceParams {
                max_entries_allowed: true
            })
        ));
        assert!(b.unparsed().is_empty());
    }

    #[test]
    fn typed_details() {
        let b = bpd(&[
            "HIKAZS:6:6:4+1+1+0+90:J:N'",
            "HIKAZS:7:5:4+1+1+60:N:J'",
            "HICAZS:8:1:4+1+1+0+450:N:N:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02'",
            "HIWPDS:9:6:4+1+1+0+J:N:J'",
        ]);
        let kaz = b.job("HKKAZ").unwrap();
        let JobDetail::Statement(s) = &kaz.versions[&6].detail else {
            panic!()
        };
        assert_eq!(
            (s.days_stored, s.max_entries_allowed, s.all_accounts_allowed),
            (90, true, false)
        );
        assert_eq!(kaz.versions[&5].security_class, None);
        let JobDetail::Statement(s) = &kaz.versions[&5].detail else {
            panic!()
        };
        assert_eq!((s.days_stored, s.all_accounts_allowed), (60, true));
        assert_eq!(
            b.camt_formats(),
            ["urn:iso:std:iso:20022:tech:xsd:camt.052.001.02"]
        );
        let JobDetail::Holdings(h) = &b.job("HKWPD").unwrap().versions[&6].detail else {
            panic!()
        };
        assert_eq!(
            (
                h.max_entries_allowed,
                h.currency_selectable,
                h.quality_selectable
            ),
            (true, false, true)
        );
    }

    #[test]
    fn job_without_definition_is_header_only() {
        let b = bpd(&["HICCMS:6:1:4+1+1+0+J:N'", "HIPROS:7:3:4+999+2'"]);
        let ccs = &b.job("HKCCM").unwrap().versions[&1];
        assert_eq!(
            (ccs.max_orders, ccs.min_signatures, ccs.security_class),
            (1, 1, Some(0))
        );
        assert!(matches!(&ccs.detail, JobDetail::Raw(s) if s.segment_type() == "HICCMS"));
        let pro = &b.job("HKPRO").unwrap().versions[&3];
        assert_eq!(
            (pro.max_orders, pro.min_signatures, pro.security_class),
            (999, 2, None)
        );
        assert!(b.unparsed().is_empty());
        // DI…S is a DK job.
        let b = bpd(&["DIKKUS:6:2:4+1+1+0'"]);
        assert!(b.job("DKKKU").is_some());
    }

    #[test]
    fn rejected_segment_is_header_only_and_unparsed() {
        // HIKAZS 6 needs the parameter group; `X` is no jn.
        let b = bpd(&["HIKAZS:6:6:4+1+1+0+90:X:N'"]);
        let v6 = &b.job("HKKAZ").unwrap().versions[&6];
        assert!(matches!(v6.detail, JobDetail::Raw(_)));
        assert_eq!(v6.security_class, Some(0));
        assert_eq!(b.unparsed().len(), 1);
        assert_eq!(b.unparsed()[0].segment.segment_type(), "HIKAZS");
        assert!(b.unparsed()[0].error.field.contains("max_entries_allowed"));
    }

    #[test]
    fn segment_without_header_is_unparsed_and_not_offered() {
        let b = bpd(&["HIXYZS:6:1:4+*+*'"]);
        assert!(b.job("HKXYZ").is_none());
        assert_eq!(b.unparsed().len(), 1);
        assert_eq!(b.unparsed()[0].error.kind, ErrorKind::UnknownSegment);
    }

    #[test]
    fn general_parameter_segments_are_not_jobs() {
        let b = bpd(&[
            "HIKOM:6:4:4+280:12345678+1+3:https?://x'",
            "HISHV:7:3:4+N+PIN:1'",
        ]);
        assert!(b.jobs().next().is_none());
        assert!(b.unparsed().is_empty());
    }

    #[test]
    fn pin_tan_and_tan_required() {
        let b = bpd(&[
            "HIPINS:6:1:4+1+1+0+5:20:6:Kundennummer:Kunden-ID:HKSAL:N:HKCCS:J'",
            "HISALS:7:6:4+1+1+0'",
        ]);
        let p = b.pin_tan.as_ref().unwrap();
        assert_eq!(
            (p.min_pin_length, p.max_pin_length, p.max_tan_length),
            (Some(5), Some(20), Some(6))
        );
        assert_eq!(p.user_id_text.as_deref(), Some("Kundennummer"));
        assert_eq!(b.tan_required("HKSAL"), Some(false));
        // An order HIPINS lists although the BPD has no parameters for it.
        assert_eq!(b.tan_required("HKCCS"), Some(true));
        assert!(b.job("HKCCS").is_none());
        assert_eq!(b.tan_required("HKKAZ"), None);
        assert!(b.job("HKPIN").is_some());
        assert_eq!(bpd(&[]).tan_required("HKSAL"), None);
    }

    #[test]
    fn tan_methods_per_version() {
        let hitans7 = "HITANS:6:7:4+1+1+0+N:N:1:900:2:MS1.0.0:photoTAN::Generator:9:1:TAN:999:J:1:N:0:0:N:J:00:0:J:1::::::901:2:MS2.0.0:Decoupled::App:::TAN:999:J:1:N:0:0:N:J:00:0:J:1:60:5:5:N:J'";
        let b = bpd(&[hitans7]);
        assert_eq!(b.tan.hktan_versions().collect::<Vec<_>>(), [7]);
        assert_eq!(b.tan.versions[&7].one_step_allowed, Some(false));
        let sf: Vec<&str> = b
            .tan_methods()
            .map(|m| m.security_function.as_str())
            .collect();
        assert_eq!(sf, ["900", "901"]);
        assert!(b.tan_methods().nth(1).unwrap().is_decoupled);
        assert!(b.job("HKTAN").is_some());
    }

    #[test]
    fn sepa_is_the_highest_hispas() {
        let b = bpd(&[
            "HISPAS:6:1:4+1+1+0+J:J:J:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.03'",
            "HISPAS:7:3:4+1+1+0+J:N:J:J:27'",
        ]);
        let s = b.sepa().unwrap();
        assert!(!s.national_account_allowed);
        assert_eq!(s.max_entries_allowed, Some(true));
        assert_eq!(s.reserved_purpose_places, Some(27));
        assert!(s.formats.is_empty());
        assert!(
            bpd(&["HISPAS:6:1:4+1+1+0+J:J:N'"])
                .sepa()
                .unwrap()
                .national_account_allowed
        );
        assert!(bpd(&[]).sepa().is_none());
    }

    #[test]
    fn highest_common_version() {
        let b = bpd(&[
            "HISALS:6:5:4+1+1'",
            "HISALS:7:6:4+1+1+0'",
            "HISALS:8:7:4+1+1+0'",
        ]);
        assert_eq!(b.highest_common("HKSAL", &[5, 6, 8]), Ok(6));
        assert_eq!(b.highest_common("HKSAL", &[8, 7]), Ok(7));
        let e = b.highest_common("HKSAL", &[8]).unwrap_err();
        assert_eq!(
            (e.offered.as_slice(), e.implemented.as_slice()),
            (&[5, 6, 7][..], &[8][..])
        );
        assert_eq!(
            e.to_string(),
            "HKSAL: no common version (bank offers 5, 6, 7, client implements 8)"
        );
        let e = b.highest_common("HKKAZ", &[6, 7]).unwrap_err();
        assert_eq!(
            e.to_string(),
            "HKKAZ: bank does not offer this order (client implements 6, 7)"
        );
    }

    #[test]
    fn bpd_replaced_with_hibpa_merged_without() {
        let current = segs(&[HIBPA, "HISALS:6:5:4+1+1'", "HISALS:7:6:4+1+1+0'"]);
        // HIBPA: the response's set replaces everything.
        let response = segs(&[
            "HIRMG:2:2+0010::ok'",
            "HIBPA:3:3:4+13+280:12345678+Testbank+3+1+300'",
            "HIKAZS:4:6:4+1+1+0+90:J:N'",
        ]);
        let next = bpd_after(&current, &response).unwrap();
        let codes: Vec<_> = next.iter().map(|s| s.segment_type()).collect();
        assert_eq!(codes, ["HIBPA", "HIKAZS"]);
        assert_eq!(Bpd::from_segments(&next).unwrap().version, 13);
        // No HIBPA: merged by code and version.
        let response = segs(&["HISALS:3:6:4+9+1+0'", "HISALS:4:7:4+1+1+0'"]);
        let next = bpd_after(&current, &response).unwrap();
        let b = Bpd::from_segments(&next).unwrap();
        assert_eq!(b.version, 12);
        let sal = b.job("HKSAL").unwrap();
        assert_eq!(sal.offered().collect::<Vec<_>>(), [5, 6, 7]);
        assert_eq!(sal.versions[&6].max_orders, 9);
        assert_eq!(next.len(), 4);
        // Nothing parameter-like: unchanged.
        assert!(bpd_after(&current, &segs(&["HIRMG:2:2+0010::ok'"])).is_none());
    }

    const HIUPD6: &str = "HIUPD:6:6:4+1234567890::280:12345678+DE00123456781234567890+CUST0001+1+EUR+Erika Beispiel+Max Beispiel+Girokonto+T:2500,:EUR+HKSAL:1+HKKAZ:1+HKCCS:1:E:1000,:EUR'";
    const HIUPD4: &str =
        "HIUPD:7:4:4+7654321::280:12345678+CUST0001+EUR+Erika Beispiel++Sparkonto++HKSAL:2'";

    #[test]
    fn upd_accounts_and_usage() {
        let u = Upd::from_segments(&segs(&["HIUPA:5:4:4+USER0001+7+0'", HIUPD6, HIUPD4])).unwrap();
        assert_eq!((u.version, u.usage), (7, UpdUsage::Closed));
        assert!(!u.is_dialog_only());
        assert_eq!(u.accounts().len(), 2);
        let a = &u.accounts()[0];
        assert_eq!(a.account_number.as_deref(), Some("1234567890"));
        assert_eq!(a.iban, Some(Iban::new("DE00123456781234567890")));
        assert_eq!(a.customer_id, "CUST0001");
        assert_eq!(a.class(), Some(AccountClass::Checking));
        assert_eq!(a.owner, ["Erika Beispiel", "Max Beispiel"]);
        assert_eq!(a.limit.as_ref().unwrap().kind, LimitKind::Day);
        assert_eq!(a.limit.as_ref().unwrap().amount, Some(Decimal::from(2500)));
        let ccs = &a.allowed[&SegmentType::new("HKCCS")];
        assert_eq!(ccs.limit.as_ref().unwrap().kind, LimitKind::SingleOrder);
        assert_eq!(u.allows(a, "HKSAL"), Allowance::Yes);
        assert_eq!(u.allows(a, "HKWPD"), Allowance::No);
        assert_eq!(a.key().unwrap().to_string(), "12345678/1234567890");
        // HIUPD 4: no IBAN, no account type.
        let b = &u.accounts()[1];
        assert_eq!((b.iban.as_ref(), b.account_type), (None, None));
        assert_eq!(b.allowed[&SegmentType::new("HKSAL")].min_signatures, 2);
        assert_eq!(b.product_name.as_deref(), Some("Sparkonto"));

        let u = Upd::from_segments(&segs(&["HIUPA:5:4:4+USER0001+7+1'", HIUPD4])).unwrap();
        assert_eq!(u.usage, UpdUsage::Open);
        assert_eq!(u.allows(&u.accounts()[0], "HKWPD"), Allowance::Unknown);
    }

    #[test]
    fn upd_version_0_is_dialog_only() {
        let u = Upd::from_segments(&segs(&["HIUPA:5:4:4+USER0001+0+0'"])).unwrap();
        assert!(u.is_dialog_only());
        assert!(u.accounts().is_empty());
    }

    #[test]
    fn unreadable_hiupd_is_unparsed() {
        let u = Upd::from_segments(&segs(&[
            "HIUPA:5:4:4+USER0001+7+0'",
            "HIUPD:6:6:4+1::280:12345678++C+abc+EUR+Name'",
            HIUPD4,
        ]))
        .unwrap();
        assert_eq!(u.accounts().len(), 1);
        assert_eq!(u.unparsed().len(), 1);
        assert_eq!(u.unparsed()[0].error.field, "account_type");
    }

    #[test]
    fn upd_replaced_only_with_hiupa() {
        assert!(upd_after(&segs(&["HIRMG:2:2+0010::ok'", HIUPD6])).is_none());
        let next = upd_after(&segs(&[
            "HIRMG:2:2+0010::ok'",
            "HIUPA:5:4:4+U+8+0'",
            HIUPD6,
        ]))
        .unwrap();
        let codes: Vec<_> = next.iter().map(|s| s.segment_type()).collect();
        assert_eq!(codes, ["HIUPA", "HIUPD"]);
    }

    #[test]
    fn model_serialises_for_inspection() {
        let b = bpd(&["HIKAZS:6:6:4+1+1+0+90:X:N'"]);
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["jobs"]["HKKAZ"]["versions"]["6"]["max_orders"], 1);
        assert!(
            json["unparsed"][0]["error"]
                .as_str()
                .unwrap()
                .contains("max_entries_allowed")
        );
    }

    /// The live Consorsbank BPD and UPD (committed trace, anonymised).
    #[test]
    #[cfg(repo_checkout)]
    fn consorsbank_trace() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testbank/fixtures/replay/consorsbank-76030080.trace.json"
        );
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let answer = |marker: &str| -> Vec<RawSegment> {
            let exchange = json["exchanges"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| {
                    e["response"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|s| s.as_str().unwrap().starts_with(marker))
                })
                .unwrap();
            let wire: Vec<&str> = exchange["response"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .filter(|s| s.starts_with("HI") || s.starts_with("DI"))
                .collect();
            segs(&wire)
        };

        let b = Bpd::from_segments(&answer("HIBPA:")).unwrap();
        let p = b.pin_tan.as_ref().unwrap();
        let flags: Vec<(&str, bool)> = p
            .tan_required
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect();
        assert_eq!(
            flags,
            [
                ("HKCCS", true),
                ("HKIPZ", true),
                ("HKKAZ", false),
                ("HKPRO", false),
                ("HKSAL", false),
                ("HKSPA", false),
                ("HKTAN", false),
                ("HKWPD", false),
                ("XKADA", false),
                ("XKADS", false),
            ]
        );
        let offered = |code| b.job(code).unwrap().offered().collect::<Vec<_>>();
        assert_eq!(offered("HKSAL"), [3, 4, 5, 6]);
        assert_eq!(offered("HKKAZ"), [3, 4, 5, 6, 7]);
        assert_eq!(offered("HKWPD"), [2, 3, 4, 5, 6]);
        assert_eq!(offered("HKPRO"), [3, 4]);
        assert_eq!(b.tan.hktan_versions().collect::<Vec<_>>(), [7]);
        let sf: Vec<&str> = b
            .tan_methods()
            .map(|m| m.security_function.as_str())
            .collect();
        assert_eq!(sf, ["900", "901"]);
        let s = b.sepa().unwrap();
        assert!(
            s.single_account_allowed && s.national_account_allowed && s.structured_purpose_allowed
        );
        // Typed where a definition exists; header-only below.
        assert!(matches!(
            b.job("HKKAZ").unwrap().versions[&7].detail,
            JobDetail::Statement(_)
        ));
        assert!(matches!(
            b.job("HKKAZ").unwrap().versions[&4].detail,
            JobDetail::Raw(_)
        ));
        assert!(matches!(
            b.job("HKWPD").unwrap().versions[&6].detail,
            JobDetail::Holdings(_)
        ));
        assert!(matches!(
            b.job("HKCCS").unwrap().versions[&1].detail,
            JobDetail::None
        ));
        assert!(matches!(
            b.job("HKIPZ").unwrap().versions[&1].detail,
            JobDetail::Instant(_)
        ));
        // The bank's own parameter segments (DIPINS, XIADAS, XIADSS) read
        // by their D.6 header: nothing is unparsed.
        assert!(b.unparsed().is_empty());
        assert!(b.job("DKPIN").is_some());

        let u = Upd::from_segments(&answer("HIUPA:")).unwrap();
        assert!(u.is_dialog_only());
        assert_eq!(u.usage, UpdUsage::Closed);
        assert!(u.unparsed().is_empty());
        assert_eq!(u.accounts().len(), 7);
        fn jobs(a: &UpdAccount) -> Vec<&str> {
            a.allowed.keys().map(|k| k.as_str()).collect()
        }
        let giro = ["HKCCS", "HKIPZ", "HKKAZ", "HKPRO", "HKSAL", "HKSPA"];
        // One of the seven is the depot: the entry without an IBAN.
        assert_eq!(u.accounts().iter().filter(|a| a.iban.is_none()).count(), 1);
        for a in u.accounts() {
            if a.iban.is_none() {
                assert_eq!(jobs(a), ["HKPRO", "HKWPD"]);
                assert_eq!(u.allows(a, "HKSAL"), Allowance::No);
            } else {
                assert_eq!(jobs(a), giro);
                assert_eq!(u.allows(a, "HKWPD"), Allowance::No);
            }
            assert_eq!(a.currency, Some(Currency::new("EUR")));
        }
    }

    const URN_09: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09";
    const URN_03: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.001.03";
    const URN_09_WIRE: &str = "urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09";
    const URN_03_WIRE: &str = "urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.03";
    const HISPAS: &str =
        "HISPAS:8:1:4+1+1+0+J:J:J:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.03'";

    #[test]
    fn vop_reads_the_parameters_of_the_highest_hivpps() {
        let b = bpd(&[
            "HIVPPS:9:1:4+1+1+0+1000:N:V:N:N:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.002.001.10;sepade.pain.002.001.10.xsd:HKCCS:HKIPZ'",
        ]);
        assert!(b.unparsed().is_empty(), "{:?}", b.unparsed());
        let v = b.vop().expect("vop");
        assert_eq!(v.max_transactions, 1000);
        assert!(!v.explanation_structured);
        assert_eq!(v.delivery, VopDelivery::Complete);
        assert!(!v.bulk_with_one_order_allowed && !v.max_entries_input_allowed);
        assert_eq!(
            v.report_formats,
            [
                "urn:iso:std:iso:20022:tech:xsd:pain.002.001.10",
                "sepade.pain.002.001.10.xsd"
            ]
        );
        assert!(v.requires("HKCCS") && v.requires("HKIPZ") && !v.requires("HKKAZ"));
        let c = bpd(&["HIVPPS:9:1:4+1+1+0+20:J:S:N:N:x.xsd:HKIPZ'"]);
        let v = c.vop().unwrap();
        assert_eq!(v.delivery, VopDelivery::Stepwise);
        assert!(v.explanation_structured);
        assert!(!v.requires("HKCCS"));
    }

    #[test]
    fn vop_is_none_without_hivpps_and_kept_with_an_unknown_delivery() {
        assert!(bpd(&[HISPAS]).vop().is_none());
        let b = bpd(&["HIVPPS:9:1:4+1+1+0+20:J:X:N:N:x.xsd:HKIPZ'"]);
        let v = b.vop().expect("an unknown delivery keeps the parameters");
        assert_eq!(v.delivery, VopDelivery::Other("X".into()));
        assert!(v.requires("HKIPZ"));
    }

    #[test]
    fn instant_summarises_hiipzs_and_hiipss() {
        let ipz1 = format!("HIIPZS:3:1:4+1+1+0+BENE, SALA,:{URN_09_WIRE}'");
        let ipz2 = format!("HIIPZS:4:2:4+1+1+0+J::{URN_09_WIRE}:{URN_03_WIRE}'");
        let ips = "HIIPSS:5:1:4+1+1+0+5'";
        let b = bpd(&[&ipz1, &ipz2, ips]);
        assert!(b.unparsed().is_empty(), "{:?}", b.unparsed());
        let i = b.instant().expect("instant");
        assert_eq!(i.versions, [1, 2]);
        assert!(i.payment.conversion_allowed);
        assert!(i.payment.purpose_codes.is_empty());
        assert_eq!(i.payment.formats, [URN_09, URN_03]);
        let s = i.status.expect("status");
        assert_eq!((s.min_wait, s.formats.len()), (5, 0));

        let b = bpd(&[&ipz1]);
        let i = b.instant().unwrap();
        assert!(!i.payment.conversion_allowed);
        assert_eq!(i.payment.purpose_codes, ["BENE", "SALA"]);
        assert!(i.status.is_none());
        assert!(bpd(&[HISPAS]).instant().is_none());
    }

    #[test]
    fn sepa_formats_per_order() {
        let ipz = format!("HIIPZS:3:1:4+1+1+0+:{URN_09_WIRE}'");
        let b = bpd(&[HISPAS, &ipz, "HICCSS:4:1:4+1+1+0'"]);
        assert_eq!(b.sepa_formats(Order::Transfer), [URN_03]);
        assert_eq!(b.sepa_formats(Order::InstantTransfer), [URN_09]);
        // HIIPSS names no format here: HISPAS applies.
        let b = bpd(&[HISPAS, &ipz, "HIIPSS:5:1:4+1+1+0+5'"]);
        assert_eq!(b.sepa_formats(Order::InstantStatus), [URN_03]);
        let ips = format!("HIIPSS:5:1:4+1+1+0+5:{URN_09_WIRE}'");
        let b = bpd(&[HISPAS, &ips]);
        assert_eq!(b.sepa_formats(Order::InstantStatus), [URN_09]);
        // No HIIPZS formats: HISPAS applies; no HISPAS: nothing.
        let b = bpd(&[HISPAS, "HIIPZS:3:1:4+1+1+0+'"]);
        assert_eq!(b.sepa_formats(Order::InstantTransfer), [URN_03]);
        assert!(bpd(&[]).sepa_formats(Order::Transfer).is_empty());
        // Orders without a pain.001 message.
        assert!(b.sepa_formats(Order::Balance).is_empty());
        assert!(b.sepa_formats(Order::VopCheck).is_empty());
    }

    #[test]
    fn payment_parameter_segments_without_own_parameters_read_typed() {
        let b = bpd(&["HICCSS:3:1:4+999+1+0'", "HIVPAS:4:1:4+1+1+0'"]);
        assert!(b.unparsed().is_empty());
        for code in ["HKCCS", "HKVPA"] {
            let v = &b.job(code).unwrap().versions[&1];
            assert!(matches!(v.detail, JobDetail::None), "{code}");
        }
        assert_eq!(b.job("HKCCS").unwrap().versions[&1].max_orders, 999);
    }
}
