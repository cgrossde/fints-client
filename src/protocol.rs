//! FinTS 3.0 Protocol State Machine (spec-aligned).
//!
//! Implements the dialog lifecycle as defined in FinTS 3.0 Formals (2017-10-06)
//! and PIN/TAN Security (2020-07-10).
//!
//! ## Dialog types (spec Chapter C)
//!
//! - **Synchronization dialog**: HKIDN + HKVVB + HKSYN → get system_id → HKEND.
//!   Marked by presence of HKSYN. Business segments are forbidden (banks return 9110).
//! - **Normal dialog**: HKIDN + HKVVB [+ HKTAN:4] → [TAN confirmation] → business ops → HKEND.
//!
//! ## State machine (spec Chapter C, Section 6)
//!
//! Internal: the typestate `Dialog<S>` (`New`, `Synced`, `Open`,
//! `TanPending`) keeps the dialog's authentication state in the type system.
//! [`crate::jobs::Dialog`] wraps it for callers and adds the pause/resume
//! phases, so the library never waits for a TAN or an app confirmation.
//!
//! ```text
//!   Dialog<New>
//!       ├── sync()           → (Dialog<Synced>, Response)      [sync dialog]
//!       ├── init()           → InitResult                      [normal dialog]
//!       │     ├── Opened(Dialog<Open>)                            0010/0020
//!       │     └── TanRequired(Dialog<TanPending>, TanChallenge)   0030/3955
//!       └── init_no_tan()    → (Dialog<Open>, Response)        [PIN-only login]
//!
//!   Dialog<Synced>
//!       └── end()            → (BankParams, SystemId)
//!
//!   Dialog<Open>
//!       └── send_order()     → Response                        [one order]
//! ```

use std::collections::BTreeSet;
use std::marker::PhantomData;
use tracing::{debug, info, warn};

use jiff::civil::Date;

use crate::error::{BankError, FinTSError, Result};
use crate::message;
use crate::parser::RawSegment;
pub use crate::segments::Order;
use crate::segments::read::{extract_allowed_security_functions, find_touchdown};

use crate::quirks::QuirkChain;
use crate::return_code::{
    Category, Class, ClientWarning, Kind, Notice, NoticeDialog, ReturnCode, Scope,
    looks_like_utf8_as_latin1,
};
use crate::transport::SharedTransport;
use crate::types::*;

mod params;
mod tan;
pub use params::{
    AccountLimit, Allowance, AllowedJob, BalanceParams, Bpd, CamtParams, FormatLists,
    HoldingsParams, InstantParams, InstantStatusParams, InstantSupport, JobDetail, JobParams,
    JobVersion, LimitKind, ParamsError, PinTanParams, SepaParams, StatementParams, TanParams,
    TanVersion, UnparsedParam, Upd, UpdAccount, UpdUsage, VersionMismatch, VopDelivery, VopParams,
};
use params::{bpd_after, upd_after};
pub use tan::{
    Retry, TanDecision, TanOverride, TanOverrides, TanReason, on_sca_required, order_tan,
};

// ═══════════════════════════════════════════════════════════════════════════════
// Typestate markers (per spec dialog states)
// ═══════════════════════════════════════════════════════════════════════════════

/// Dialog not yet started. Can transition to `Synced` or `Open`/`TanPending`.
#[derive(Debug)]
pub(crate) struct New;
/// Synchronization dialog: system_id obtained, BPD/UPD cached. No business ops allowed.
#[derive(Debug)]
pub(crate) struct Synced;
/// Dialog is open and authenticated. Business segments can be sent.
#[derive(Debug)]
pub(crate) struct Open;
/// A TAN challenge is pending (either from init or from a business segment).
#[derive(Debug)]
pub(crate) struct TanPending;

// ═══════════════════════════════════════════════════════════════════════════════
// Typed business segments — replaces raw Vec<DEG> at all internal boundaries
// ═══════════════════════════════════════════════════════════════════════════════

/// A typed FinTS business segment. Every field is validated at construction.
/// This is the ONLY way to construct segments within the crate — raw DEG
/// builders are never called directly from protocol or job code.
#[derive(Debug, Clone)]
pub(crate) enum Segment {
    /// HKIDN: Identifikation (dialog init)
    Identify {
        blz: Blz,
        user_id: UserId,
        system_id: SystemId,
    },
    /// HKVVB: Verarbeitungsvorbereitung
    ProcessPrep {
        bpd_version: u16,
        upd_version: u16,
        product_id: ProductId,
    },
    /// HKSYN: Synchronisierung (get system_id)
    Sync,
    /// HKSAL: Saldenabfrage (balance request)
    Balance { account: Account },
    /// HKKAZ: Kontoumsätze (transaction request)
    Transactions {
        account: Account,
        start_date: Date,
        end_date: Date,
        touchdown: Option<TouchdownPoint>,
    },
    /// HKCAZ: Kontoumsätze im camt-Format (CAMT.052 transaction request)
    CamtTransactions {
        account: Account,
        start_date: Date,
        end_date: Date,
        touchdown: Option<TouchdownPoint>,
    },
    /// HKWPD: Wertpapierdepotaufstellung (securities holdings request)
    Holdings {
        account: Account,
        currency: Option<Currency>,
        touchdown: Option<TouchdownPoint>,
    },
    /// HKWDU: Depotumsätze (depot transactions request)
    DepotTransactions {
        account: Account,
        start_date: Option<Date>,
        end_date: Option<Date>,
        touchdown: Option<TouchdownPoint>,
    },
    /// HKTAN process 4: initiate TAN for a referenced segment
    TanProcess4 {
        reference_seg: SegmentRef,
        tan_medium: Option<TanMediumName>,
    },
    /// HKTAN process S: poll decoupled TAN status
    TanPollDecoupled {
        task_reference: TaskReference,
        tan_medium: Option<TanMediumName>,
    },
    /// HKTAN process 2: submit TAN response
    TanProcess2 {
        task_reference: TaskReference,
        tan_medium: Option<TanMediumName>,
    },
    /// HKEND: dialog end
    End { dialog_id: DialogId },
    /// HKCCS: SEPA credit transfer, the pain.001 bytes as built.
    CreditTransfer {
        account: Account,
        descriptor: String,
        pain: Vec<u8>,
    },
    /// HKIPZ: SEPA instant credit transfer at `version` (chosen before
    /// the pain message was built); `convert` is v2's conversion flag.
    InstantTransfer {
        account: Account,
        version: u16,
        descriptor: String,
        pain: Vec<u8>,
        convert: Option<bool>,
    },
    /// HKIPS: status of an instant payment; `descriptors` are the pain
    /// messages the client reads in the answer.
    InstantStatus {
        account: Account,
        descriptors: Vec<String>,
        order_id: String,
    },
    /// HKVPP: payee check; a poll carries the polling ID and touchdown.
    VopCheck {
        descriptors: Vec<String>,
        polling_id: Option<Vec<u8>>,
        touchdown: Option<String>,
    },
    /// HKVPA: release after the payee check.
    VopExecute { vop_id: Vec<u8> },
}

impl Segment {
    /// Encode with the given segment number, choosing segment versions from
    /// the bank's BPD.
    pub(crate) fn encode(&self, params: &BankParams, number: u16) -> Result<RawSegment> {
        use crate::segments::{encode, request};
        fn td(t: &Option<TouchdownPoint>) -> Option<&str> {
            t.as_ref().map(TouchdownPoint::as_str)
        }
        let national = params.national_account_allowed();
        let with_flag = |a: &Account| a.clone().with_kti_national(national);
        match self {
            Segment::Identify {
                blz,
                user_id,
                system_id,
            } => encode(
                &request::hkidn(blz.as_str(), user_id.as_str(), system_id.as_str())?,
                number,
            ),
            Segment::ProcessPrep {
                bpd_version,
                upd_version,
                product_id,
            } => encode(
                &request::hkvvb(*bpd_version, *upd_version, product_id.as_str())?,
                number,
            ),
            Segment::Sync => encode(&request::hksyn(), number),
            Segment::End { dialog_id } => encode(&request::hkend(dialog_id.as_str())?, number),
            Segment::TanProcess4 {
                reference_seg,
                tan_medium,
            } => request::hktan(
                params.hktan_version()?,
                request::TanRequest::Start {
                    segment: reference_seg.as_str(),
                },
                tan_medium.as_ref().map(|t| t.as_str()),
                number,
            ),
            Segment::TanPollDecoupled {
                task_reference,
                tan_medium,
            } => request::hktan(
                params.hktan_version()?,
                request::TanRequest::DecoupledStatus {
                    task_reference: task_reference.as_str(),
                },
                tan_medium.as_ref().map(|t| t.as_str()),
                number,
            ),
            Segment::TanProcess2 {
                task_reference,
                tan_medium,
            } => request::hktan(
                params.hktan_version()?,
                request::TanRequest::Submit {
                    task_reference: task_reference.as_str(),
                },
                tan_medium.as_ref().map(|t| t.as_str()),
                number,
            ),
            Segment::CreditTransfer {
                account,
                descriptor,
                pain,
            } => {
                let version = params.order_version(Order::Transfer)?;
                request::hkccs(number, version, &with_flag(account), descriptor, pain)
            }
            Segment::InstantTransfer {
                account,
                version,
                descriptor,
                pain,
                convert,
            } => request::hkipz(
                number,
                *version,
                &with_flag(account),
                descriptor,
                pain,
                *convert,
            ),
            Segment::InstantStatus {
                account,
                descriptors,
                order_id,
            } => {
                let version = params.order_version(Order::InstantStatus)?;
                request::hkips(number, version, &with_flag(account), descriptors, order_id)
            }
            Segment::VopCheck {
                descriptors,
                polling_id,
                touchdown,
            } => {
                let version = params.order_version(Order::VopCheck)?;
                request::hkvpp(
                    number,
                    version,
                    descriptors,
                    polling_id.as_deref(),
                    touchdown.as_deref(),
                )
            }
            Segment::VopExecute { vop_id } => {
                let version = params.order_version(Order::VopExecute)?;
                request::hkvpa(number, version, vop_id)
            }
            Segment::Balance { account } => {
                let version = params.order_version(Order::Balance)?;
                request::hksal(number, version, &with_flag(account), None)
            }
            Segment::Transactions {
                account,
                start_date,
                end_date,
                touchdown,
            } => {
                let version = params.order_version(Order::Transactions)?;
                request::hkkaz(
                    number,
                    version,
                    &with_flag(account),
                    *start_date,
                    *end_date,
                    td(touchdown),
                )
            }
            Segment::CamtTransactions {
                account,
                start_date,
                end_date,
                touchdown,
            } => {
                let version = params.order_version(Order::Camt)?;
                request::hkcaz(
                    number,
                    version,
                    &with_flag(account),
                    &params.camt052_format(),
                    *start_date,
                    *end_date,
                    td(touchdown),
                )
            }
            Segment::Holdings {
                account,
                currency,
                touchdown,
            } => {
                let version = params.order_version(Order::Holdings)?;
                request::hkwpd(
                    number,
                    version,
                    account,
                    currency.as_ref().map(|c| c.as_str()),
                    td(touchdown),
                )
            }
            Segment::DepotTransactions {
                account,
                start_date,
                end_date,
                touchdown,
            } => {
                let version = params.order_version(Order::DepotTransactions)?;
                request::hkwdu(
                    number,
                    version,
                    account,
                    *start_date,
                    *end_date,
                    td(touchdown),
                )
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Result of dialog init — response-driven transition
// ═══════════════════════════════════════════════════════════════════════════════

/// Result of `Dialog::init()`. Per the spec, the bank either accepts the dialog
/// (0010/0020) or requires SCA (0030/3955).
pub(crate) enum InitResult {
    /// Dialog opened, no TAN needed. Ready for business segments.
    Opened(Dialog<Open>),
    /// Bank requires TAN on init (SCA). Must confirm before business ops.
    TanRequired(Dialog<TanPending>, TanChallenge),
}

// ═══════════════════════════════════════════════════════════════════════════════
// Parsed bank response
// ═══════════════════════════════════════════════════════════════════════════════

/// Parsed response from the bank after sending a message.
#[derive(Debug)]
pub struct Response {
    /// All segments from the response (including inner HNVSD segments).
    pub segments: Vec<RawSegment>,
    /// Return codes for the whole message (HIRMG), in wire order.
    pub global_codes: Vec<ReturnCode>,
    /// Return codes for segments of the sent message (HIRMS), in wire order.
    pub segment_codes: Vec<ReturnCode>,
}

impl Response {
    pub fn find_segments(&self, seg_type: &str) -> Vec<&RawSegment> {
        self.segments
            .iter()
            .filter(|s| s.segment_type() == seg_type)
            .collect()
    }

    pub fn find_segment(&self, seg_type: &str) -> Option<&RawSegment> {
        self.segments.iter().find(|s| s.segment_type() == seg_type)
    }

    pub fn all_codes(&self) -> impl Iterator<Item = &ReturnCode> {
        self.global_codes.iter().chain(self.segment_codes.iter())
    }

    /// The answer to a typed TAN rejects the TAN and leaves the order for
    /// another one (docs/return-codes-design.md section 1): the class 9
    /// codes are 9941 and, at most, the 9050 summary and a 9360 that still
    /// leaves attempts. Gives the first 9941 and 9360's P1. `None` when
    /// the bank ended the dialog (9800), deleted the order (9210), locked
    /// the access (9931, 9360 without attempts), the TAN timed out or was
    /// used (9951, 9943), or the codes are unreadable.
    pub(crate) fn tan_rejection(&self) -> Option<(ReturnCode, Option<u32>)> {
        if !matches!(self.check_errors(), Err(FinTSError::Bank(_))) {
            return None;
        }
        let mut rejected = None;
        let mut attempts_left = None;
        for c in self.all_codes().filter(|c| c.is_error()) {
            match c.kind() {
                Kind::TanInvalid => {
                    rejected.get_or_insert_with(|| c.clone());
                }
                Kind::PartialErrors => {}
                Kind::LockAfterAttempts {
                    attempts_left: Some(n),
                } if n > 0 => attempts_left = Some(n),
                _ => return None,
            }
        }
        rejected.map(|c| (c, attempts_left))
    }

    /// True if any code has this kind.
    pub(crate) fn has(&self, kind: Kind) -> bool {
        self.all_codes().any(|c| c.kind() == kind)
    }

    /// Fills [`Scope::Segment`]'s `order` of every HIRMS code from the
    /// sent message `request` (its segment numbers). Codes referring to a
    /// number not in the request keep `None`.
    fn name_orders(&mut self, request: &[RawSegment]) {
        for code in &mut self.segment_codes {
            if let Scope::Segment { number, order } = &mut code.scope {
                *order = request
                    .iter()
                    .find(|s| s.segment_number() == *number)
                    .map(|s| SegmentType::new(s.segment_type()));
            }
        }
    }

    /// All segments read with the codec's declarative definitions. A
    /// segment without a definition, or one its definition rejects, comes
    /// back as `ParsedSegment::Raw` with the error that says why
    /// (docs/architecture.md 2.1, "parse what you can, keep the rest").
    pub fn decode_segments(&self) -> Vec<crate::codec::ParsedSegment> {
        crate::codec::parse_segments(self.segments.iter().cloned())
    }

    /// The codec errors of all segments that stayed raw for a reason other
    /// than a missing definition.
    pub fn segment_errors(&self) -> Vec<crate::codec::CodecError> {
        self.decode_segments()
            .into_iter()
            .filter_map(|p| match p {
                crate::codec::ParsedSegment::Raw { error, .. }
                    if error.kind != crate::codec::ErrorKind::UnknownSegment =>
                {
                    Some(error)
                }
                _ => None,
            })
            .collect()
    }

    /// 0030 = order received, TAN required.
    pub fn needs_tan(&self) -> bool {
        self.has(Kind::TanRequired) || self.has(Kind::DecoupledInitiated)
    }

    /// 3955 = decoupled TAN (pushTAN initiated).
    pub fn is_decoupled(&self) -> bool {
        self.has(Kind::DecoupledInitiated)
    }

    /// A status request got no confirmation yet: 3956, or 3957 ("wait for
    /// the push message") without 0020 in the same answer.
    pub fn is_decoupled_pending(&self) -> bool {
        self.has(Kind::DecoupledPending)
            || (self.has(Kind::DecoupledAwaitPush) && !self.has(Kind::OrderExecuted))
    }

    /// 3010 = not available; after a data request banks send it for "no
    /// entries".
    pub fn has_no_entries(&self) -> bool {
        self.has(Kind::NotAvailable)
    }

    /// 3076 = no strong authentication required (SCA exemption).
    pub fn has_sca_exemption(&self) -> bool {
        self.has(Kind::ScaExemption)
    }

    /// 3040 = more data available (touchdown/pagination).
    /// Returns the touchdown point if found.
    pub fn touchdown(&self) -> Option<TouchdownPoint> {
        find_touchdown(&self.segment_codes).or_else(|| find_touchdown(&self.global_codes))
    }

    /// Extract HITAN challenge from response. No HITAN is `Ok(None)`; a
    /// HITAN its definition rejects is [`FinTSError::SegmentUnreadable`].
    pub(crate) fn get_tan_challenge(&self) -> Result<Option<TanChallenge>> {
        let Some(raw) = self.find_segment("HITAN") else {
            return Ok(None);
        };
        let Some(hitan) =
            crate::segments::read::tan_challenge(raw).map_err(FinTSError::SegmentUnreadable)?
        else {
            return Ok(None);
        };
        if hitan.task_reference.is_empty() && hitan.challenge.is_empty() {
            return Ok(None);
        }
        Ok(Some(TanChallenge {
            challenge: ChallengeText::new(hitan.challenge),
            challenge_hhduc: hitan.challenge_hhduc.map(HhdUcData),
            task_reference: TaskReference::new(hitan.task_reference),
            decoupled: self.is_decoupled(),
            challenge_markup: false,
            valid_until: hitan.valid_until,
        }))
    }

    /// The TAN challenge of a business order the bank wants confirmed
    /// (0030/3955 with HITAN), unless it granted an exemption (3076).
    pub(crate) fn order_tan_challenge(&self) -> Result<Option<TanChallenge>> {
        if self.needs_tan() && !self.has_sca_exemption() {
            self.get_tan_challenge()
        } else {
            Ok(None)
        }
    }

    /// Check for fatal errors. Returns `Ok(())` if no errors.
    ///
    /// The reported error is [`Response::select_error`]'s; a credentials
    /// or system ID error wins even over unreadable HIRMG/HIRMS, which
    /// otherwise give `InvalidResponse`.
    pub fn check_errors(&self) -> Result<()> {
        let selected = self.select_error();
        // Credentials and system ID decide what the caller must do, even
        // next to response codes the codec cannot read.
        if let Some(e) = selected
            .as_ref()
            .filter(|e| matches!(e.category(), Category::Credentials(_) | Category::SystemId))
        {
            return Err(FinTSError::Bank(e.clone()));
        }
        // HIRMG/HIRMS the codec rejects: the bank's verdict is unknown, so
        // do not treat the response as a success.
        if let Some(error) = self
            .segments
            .iter()
            .filter(|s| matches!(s.segment_type(), "HIRMG" | "HIRMS"))
            .find_map(|s| crate::segments::read::response_codes(s).err())
        {
            return Err(FinTSError::InvalidResponse(format!(
                "response codes unreadable: {error}"
            )));
        }
        match selected {
            Some(e) => Err(FinTSError::Bank(e)),
            None => Ok(()),
        }
    }

    /// The error to report when the response has class 9 codes
    /// (docs/return-codes-design.md section 3), with all codes of the
    /// response. Specific causes rank above the signature error 9340,
    /// which ranks above the generic codes:
    /// 1. a credentials code that names the cause (9942, 9910, 9930,
    ///    9931), then a system ID code (9391), wherever they appear;
    /// 2. else the first specific segment-scope error;
    /// 3. else the first specific message-scope error (e.g. 9210 "BLZ does
    ///    not match the user" next to 9340);
    /// 4. else 9340 (signature not accepted), wherever it appears;
    /// 5. else the first error, segment scope first.
    ///
    /// Specific means neither generic (9010, 9050, 9800: summaries or
    /// consequences, not the reason) nor 9340.
    pub fn select_error(&self) -> Option<BankError> {
        fn errors(codes: &[ReturnCode]) -> impl Iterator<Item = &ReturnCode> {
            codes.iter().filter(|c| c.is_error())
        }
        let all = || self.all_codes().filter(|c| c.is_error());
        let specific = |c: &&ReturnCode| {
            !matches!(
                c.kind(),
                Kind::GeneralError
                    | Kind::PartialErrors
                    | Kind::DialogAborted
                    | Kind::SignatureInvalid
            )
        };
        let named_credentials = |c: &&ReturnCode| {
            matches!(c.category(), Some(Category::Credentials(_)))
                && c.kind() != Kind::SignatureInvalid
        };
        let selected = all()
            .find(named_credentials)
            .or_else(|| all().find(|c| c.category() == Some(Category::SystemId)))
            .or_else(|| errors(&self.segment_codes).find(specific))
            .or_else(|| errors(&self.global_codes).find(specific))
            .or_else(|| all().find(|c| c.kind() == Kind::SignatureInvalid))
            .or_else(|| {
                errors(&self.segment_codes)
                    .chain(errors(&self.global_codes))
                    .next()
            })?;
        Some(BankError::new(
            selected.clone(),
            self.all_codes().cloned().collect(),
        ))
    }

    /// The codes of this response the caller should see: not class 0 or 9
    /// and not consumed by the protocol (0030, 3040, 3076, 3920, 3955,
    /// 3956, 3091, 3945, 3045; 3090, 3093, 3094 on HKVPP/HKVPA): the
    /// transfer job acts on the VoP and instant codes itself.
    pub fn notices(&self, dialog: NoticeDialog) -> Vec<Notice> {
        self.all_codes()
            .filter(|c| matches!(c.class(), Class::Note | Class::Warning | Class::Undefined))
            .filter(|c| !Notice::is_consumed(c))
            .cloned()
            .map(|c| Notice::new(dialog, c))
            .chain(self.client_warning(dialog))
            .collect()
    }

    /// Heuristic check, once per response, of the free text of HIRMG and
    /// HIRMS (code texts and parameters): UTF-8 sequences in an ISO 8859-1
    /// answer. Statement bodies are not checked. The text stays as decoded.
    fn client_warning(&self, dialog: NoticeDialog) -> Option<Notice> {
        let garbled = self.all_codes().any(|c| {
            looks_like_utf8_as_latin1(&c.text)
                || c.parameters.iter().any(|p| looks_like_utf8_as_latin1(p))
        });
        garbled.then(|| Notice::client(dialog, ClientWarning::Utf8TextInLatin1))
    }

    /// Log the warnings no job turns into a notice of its own.
    pub(crate) fn log_informational_codes(&self) {
        for code in self.all_codes() {
            match code.kind() {
                Kind::VopNotRequired | Kind::VopApprovalNotPossible => {
                    debug!(
                        "[FinTS] {} (Verification of Payee): {}",
                        code.code, code.text
                    );
                }
                Kind::Other if code.class() == Class::Warning => {
                    debug!("[FinTS] Warning {}: {}", code.code, code.text);
                }
                _ => {}
            }
        }
    }

    /// Extract allowed TAN security functions from code 3920.
    pub fn allowed_security_functions(&self) -> Vec<SecurityFunction> {
        extract_allowed_security_functions(&self.segment_codes)
            .into_iter()
            .chain(extract_allowed_security_functions(&self.global_codes))
            .collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TAN challenge
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TanChallenge {
    pub challenge: ChallengeText,
    pub challenge_hhduc: Option<HhdUcData>,
    pub task_reference: TaskReference,
    pub decoupled: bool,
    /// HITANS "Challenge strukturiert" of the TAN method in use: the
    /// challenge text may carry markup. The response alone does not know
    /// the method; [`BankParams::challenge_structured`] sets it.
    pub challenge_markup: bool,
    /// HITAN "Gültigkeitsdatum und -uhrzeit für Challenge" (bank local
    /// time), when the bank sends one.
    pub valid_until: Option<jiff::civil::DateTime>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Bank parameters
// ═══════════════════════════════════════════════════════════════════════════════

/// The bank's parameter data as one dialog knows it.
///
/// The sources are `bpd_segments` (the bank parameters: HIBPA, HIKOM, the
/// TAN segments HITANS and HIPINS, and every `HI…S` order parameter
/// segment) and `upd_segments` (the user parameters: HIUPA and its HIUPD),
/// both byte for byte as received. All other fields are read from them. An
/// answer with a new HIBPA or HIUPA starts a new set; segments are never
/// merged across answers.
#[derive(Debug, Clone)]
pub struct BankParams {
    pub bpd_version: u16,
    pub upd_version: u16,
    pub bpd_segments: Vec<RawSegment>,
    pub upd_segments: Vec<RawSegment>,
    /// The TAN methods of `bpd`, kept as a list for the selection logic.
    pub tan_methods: Vec<TanMethod>,
    pub selected_security_function: SecurityFunction,
    pub selected_tan_medium: Option<TanMediumName>,
    pub allowed_security_functions: Vec<SecurityFunction>,
    pub preferred_security_function: Option<SecurityFunction>,
    /// Set when the bank reported code 3072 (changed access data): the new
    /// user ID to use from the next dialog on.
    pub changed_user_id: Option<UserId>,
    /// Order codes the bank turned out to want HKTAN for although HIPINS did
    /// not say so. Empty until the 9075 retry learns them.
    pub tan_required_learned: BTreeSet<SegmentType>,
    bpd: Option<Bpd>,
    upd: Option<Upd>,
}

impl BankParams {
    pub fn new() -> Self {
        Self {
            bpd_version: 0,
            upd_version: 0,
            bpd_segments: Vec::new(),
            upd_segments: Vec::new(),
            tan_methods: Vec::new(),
            selected_security_function: SecurityFunction::pin_only(),
            selected_tan_medium: None,
            allowed_security_functions: Vec::new(),
            preferred_security_function: None,
            changed_user_id: None,
            tan_required_learned: BTreeSet::new(),
            bpd: None,
            upd: None,
        }
    }

    /// The typed BPD, built from `bpd_segments`; `None` until a readable
    /// HIBPA has arrived.
    pub fn bpd(&self) -> Option<&Bpd> {
        self.bpd.as_ref()
    }

    /// The typed UPD, built from `upd_segments`. A dialog-only UPD
    /// (version 0) is held for the dialog but not stored by
    /// [`SessionState`](crate::SessionState).
    pub fn upd(&self) -> Option<&Upd> {
        self.upd.as_ref()
    }

    /// Ingest BPD/UPD/HISYN/3920/3072 from a response.
    pub fn ingest_response(&mut self, response: &Response, system_id: &mut SystemId) {
        let segs = &response.segments;

        if let Some(bpd) = bpd_after(&self.bpd_segments, segs) {
            self.set_bpd(bpd);
        }
        if let Some(upd) = upd_after(segs) {
            self.set_upd(upd);
        }

        if let Some(sid) = segs
            .iter()
            .find(|s| s.segment_type() == "HISYN")
            .and_then(crate::segments::read::system_id)
            && !sid.is_empty()
        {
            info!("[FinTS] System ID assigned");
            *system_id = SystemId::new(sid);
        }

        let allowed = response.allowed_security_functions();
        if !allowed.is_empty() {
            self.allowed_security_functions = allowed;
        }

        // 3072: access data changed. The new user/customer ID is passed on to the
        // caller; we also drop the UPD so the next dialog reloads it.
        for code in response.all_codes() {
            if let Kind::UserDataChanged { user_id, .. } = code.kind() {
                warn!(
                    "[FinTS] 3072 changed access data (new user ID {})",
                    if user_id.is_some() {
                        "given"
                    } else {
                        "not given"
                    }
                );
                if let Some(uid) = user_id {
                    self.changed_user_id = Some(UserId::new(uid));
                }
                if !segs.iter().any(|s| s.segment_type() == "HIUPA") {
                    self.invalidate_upd();
                }
            }
        }
    }

    /// Replace the BPD and rebuild the typed model, the version and the TAN
    /// methods from it. A HIBPA the codec cannot read leaves no model and
    /// version 0, so the next dialog fetches the BPD again.
    pub(crate) fn set_bpd(&mut self, segments: Vec<RawSegment>) {
        match Bpd::from_segments(&segments) {
            Ok(bpd) => {
                self.bpd_version = bpd.version;
                // Only methods in a HITANS version we can build are offered.
                self.tan_methods = bpd
                    .tan_methods()
                    .filter(|m| crate::segments::request::HKTAN_VERSIONS.contains(&m.hktan_version))
                    .cloned()
                    .collect();
                self.bpd = Some(bpd);
            }
            Err(e) => {
                if !segments.is_empty() {
                    warn!("[FinTS] BPD not usable: {e}");
                }
                if matches!(e, ParamsError::Unreadable(..)) {
                    self.bpd_version = 0;
                }
                self.tan_methods.clear();
                self.bpd = None;
            }
        }
        info!(
            "[FinTS] BPD v{}: {} segments, {} TAN methods",
            self.bpd_version,
            segments.len(),
            self.tan_methods.len(),
        );
        self.bpd_segments = segments;
    }

    /// Replace the UPD and rebuild the typed model, the version and the
    /// accounts from it.
    pub(crate) fn set_upd(&mut self, segments: Vec<RawSegment>) {
        match Upd::from_segments(&segments) {
            Ok(upd) => {
                self.upd_version = upd.version;
                self.upd = Some(upd);
            }
            Err(e) => {
                if !segments.is_empty() {
                    warn!("[FinTS] UPD not usable: {e}");
                }
                if matches!(e, ParamsError::Unreadable(..)) {
                    self.upd_version = 0;
                }
                self.upd = None;
            }
        }
        info!(
            "[FinTS] UPD v{}: {} accounts",
            self.upd_version,
            self.upd.as_ref().map_or(0, |u| u.accounts().len())
        );
        self.upd_segments = segments;
    }

    /// Drop the user parameters; HKVVB then announces UPD version 0 and the
    /// bank sends them anew.
    pub fn invalidate_upd(&mut self) {
        self.upd = None;
        self.upd_segments = Vec::new();
        self.upd_version = 0;
    }

    /// The TAN methods of every HITANS version the bank sent, including
    /// versions the client cannot build (`tan_methods` holds only the
    /// buildable ones once a BPD is set). Empty when no HITANS is known.
    fn hitans_methods(&self) -> Vec<&TanMethod> {
        match &self.bpd {
            Some(bpd) => bpd.tan_methods().collect(),
            None => self.tan_methods.iter().collect(),
        }
    }

    /// HKTAN version to send: the highest version in
    /// `HKTAN_VERSIONS` among the
    /// HITANS versions that list the selected security function (Formals
    /// "Segmentversion": the HITANS version of a method is its HKTAN version).
    /// There is no fallback to another method's version; when none is
    /// buildable the result is [`FinTSError::VersionMismatch`] naming the
    /// HITANS versions that list the method and the ones we implement.
    ///
    /// No HITANS known (first dialog, before any BPD): 7, the highest we
    /// implement. Evidence: the committed Consorsbank trace (live 2026-11-16)
    /// sends `HKTAN:5:7+4+HKIDN` at dialog init and the bank accepts it.
    pub fn hktan_version(&self) -> Result<u16> {
        let implemented = crate::segments::request::HKTAN_VERSIONS;
        let methods = self.hitans_methods();
        if methods.is_empty() {
            return Ok(implemented.iter().copied().max().unwrap_or(7));
        }
        let mut offered: Vec<u16> = methods
            .iter()
            .filter(|m| m.security_function == self.selected_security_function)
            .map(|m| m.hktan_version)
            .collect();
        offered.sort_unstable();
        offered.dedup();
        offered
            .iter()
            .rev()
            .copied()
            .find(|v| implemented.contains(v))
            .ok_or_else(|| {
                FinTSError::VersionMismatch(VersionMismatch {
                    code: SegmentType::new("HKTAN"),
                    offered,
                    implemented: implemented.to_vec(),
                    formats: None,
                })
            })
    }

    /// The HKTAN version for the dialog-initialisation message, or `None`
    /// when it carries no HKTAN. The init message allows HKTAN 6 and 7 only
    /// (PIN/TAN spec B.4, "Dialoginitialisierung": "ein Segment HKTAN ab
    /// Segmentversion #6"), so a selected method that appears only in HITANS
    /// below 6 gets no HKTAN at init; its orders still use its own version.
    /// A pin-only selection (999) with HITANS known has no method to signal
    /// and also gets none.
    pub fn init_hktan_version(&self) -> Result<Option<u16>> {
        if self.selected_security_function == SecurityFunction::pin_only()
            && !self.hitans_methods().is_empty()
        {
            return Ok(None);
        }
        let version = self.hktan_version()?;
        Ok((version >= 6).then_some(version))
    }

    /// Whether the bank allows national account details next to the IBAN
    /// (HISPAS, parameter "Nationale Kontoverbindung erlaubt"). `false`
    /// when the BPD has no HISPAS.
    pub fn national_account_allowed(&self) -> bool {
        self.bpd
            .as_ref()
            .and_then(Bpd::sepa)
            .is_some_and(|s| s.national_account_allowed)
    }

    /// The version of `order` to send: the highest version the bank offers
    /// in its BPD that this client implements ([`Order::implemented`],
    /// Formals C.10). Pure on the parameters, so it also answers "can this
    /// bank do X" without a dialog.
    ///
    /// - No BPD known → [`FinTSError::NoBpd`]: orders run only after
    ///   synchronisation, and no version is guessed.
    /// - The bank offers no version we implement, or not the order at all →
    ///   [`FinTSError::VersionMismatch`] naming both sides, instead of sending
    ///   something the bank will reject.
    pub fn order_version(&self, order: Order) -> Result<u16> {
        let code = order.code();
        let Some(bpd) = &self.bpd else {
            return Err(FinTSError::NoBpd {
                code: SegmentType::new(code),
            });
        };
        let chosen = bpd.highest_common(code, order.implemented());
        debug!(
            "[FinTS] Version negotiation {}: we implement {:?} → {:?}",
            code,
            order.implemented(),
            chosen
        );
        chosen.map_err(FinTSError::VersionMismatch)
    }

    /// The bank's storage period in days ("Speicherzeitraum" of HIKAZS or
    /// HICAZS, Messages C.2.1.1.1 / C.2.3.1.1.1) for the version we would
    /// send of `order` (`Order::Transactions` or `Order::Camt`). `None`
    /// without a BPD, for another order, or when the bank sends 0.
    pub fn days_stored(&self, order: Order) -> Option<u32> {
        let version = self.order_version(order).ok()?;
        let job = self.bpd.as_ref()?.job(order.code())?;
        match &job.versions.get(&version)?.detail {
            JobDetail::Statement(p) => Some(p.days_stored),
            JobDetail::Camt(p) => Some(p.days_stored),
            _ => None,
        }
        .filter(|d| *d > 0)
    }

    /// The camt.052 format to request in HKCAZ: the highest `camt.052.001.NN`
    /// the bank lists in HICAZS, compared by the number NN. Among equal
    /// versions the descriptor without `.xsd` wins. The bank's own string
    /// is sent. camt.052.001.02 when it lists none.
    /// All camt.052 versions parse with [`crate::camt::parse_camt052`].
    pub fn camt052_format(&self) -> String {
        self.bpd
            .iter()
            .flat_map(|b| b.camt_formats())
            .filter_map(|f| Some((crate::camt::camt052_version(f.as_bytes())?, f)))
            .max_by(|(va, a), (vb, b)| {
                va.cmp(vb)
                    .then_with(|| b.ends_with(".xsd").cmp(&a.ends_with(".xsd")))
            })
            .map(|(_, f)| f)
            .cloned()
            .unwrap_or_else(|| crate::segments::request::CAMT052_DEFAULT.to_string())
    }

    /// Select the best security function from 3920 allowed list.
    pub fn select_security_function(&mut self) {
        let allowed = &self.allowed_security_functions;
        if allowed.is_empty() {
            // No 3920 list (yet): honor a cached preference the BPD knows about.
            if let Some(pref) = &self.preferred_security_function
                && self
                    .tan_methods
                    .iter()
                    .any(|m| &m.security_function == pref)
            {
                self.selected_security_function = pref.clone();
            }
            return;
        }

        // A method that no buildable HITANS version lists is not offered:
        // a stored preference for it is ignored like an unknown method, and
        // the automatic choice skips it. Without HITANS everything is open.
        let pin_only = SecurityFunction::pin_only();
        let methods = &self.tan_methods;
        let available = |sf: &SecurityFunction| {
            methods.is_empty()
                || *sf == pin_only
                || methods.iter().any(|m| &m.security_function == sf)
        };
        if let Some(ref pref) = self.preferred_security_function
            && allowed.contains(pref)
            && available(pref)
        {
            self.selected_security_function = pref.clone();
            return;
        }

        let chosen = allowed
            .iter()
            .filter(|sf| *sf != &pin_only && available(sf))
            .max_by_key(|sf| {
                methods
                    .iter()
                    .find(|m| &m.security_function == *sf)
                    .map(|m| if m.is_decoupled { 2i32 } else { 1 })
                    .unwrap_or(0)
            });

        if let Some(sf) = chosen {
            info!("[FinTS] Selected security function: {}", sf);
            self.selected_security_function = sf.clone();
        }
    }

    pub fn is_decoupled(&self) -> bool {
        self.tan_methods
            .iter()
            .find(|m| m.security_function == self.selected_security_function)
            .map(|m| m.is_decoupled)
            .unwrap_or(false)
    }

    /// HITANS "Challenge strukturiert" of the TAN method in use.
    pub fn challenge_structured(&self) -> bool {
        self.tan_methods
            .iter()
            .find(|m| m.security_function == self.selected_security_function)
            .map(|m| m.challenge_structured)
            .unwrap_or(false)
    }

    /// Whether the chosen method wants a TAN medium named in HKTAN. Asks
    /// the first HITANS entry of the method, as the order lists them.
    pub fn needs_tan_medium(&self) -> bool {
        let chosen = &self.selected_security_function;
        let first = self
            .tan_methods
            .iter()
            .find(|m| m.security_function == *chosen);
        first.is_some_and(|m| m.needs_tan_medium)
    }

    /// The chosen method's HITANS entry. When several HITANS versions list
    /// it, the entry of the highest version, which is the HKTAN we send.
    pub fn selected_tan_method(&self) -> Option<&TanMethod> {
        let chosen = &self.selected_security_function;
        let mut best: Option<&TanMethod> = None;
        for method in self
            .tan_methods
            .iter()
            .filter(|m| m.security_function == *chosen)
        {
            if best.is_none_or(|b| method.hktan_version >= b.hktan_version) {
                best = Some(method);
            }
        }
        best
    }

    /// How often and how long to ask for the status of a decoupled
    /// approval, from the chosen method's HITANS 7 entry.
    pub fn decoupled_params(&self) -> DecoupledParams {
        let Some(method) = self.selected_tan_method() else {
            return DecoupledParams::default();
        };
        // HITANS gives seconds; 0 and the -1 we store for "not sent" both
        // mean no requirement.
        let wait = |seconds: i32| {
            (seconds > 0).then(|| std::time::Duration::from_secs(seconds.unsigned_abs().into()))
        };
        // A maximum of 0 means no limit, the same as no maximum.
        let max_polls =
            (method.decoupled_max_polls > 0).then(|| method.decoupled_max_polls.unsigned_abs());
        DecoupledParams {
            wait_before_first_poll: wait(method.wait_before_first_poll),
            wait_before_next_poll: wait(method.wait_before_next_poll),
            max_polls,
            auto_polling_allowed: method.decoupled_auto_polling,
        }
    }
}

impl Default for BankParams {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Decoupled TAN polling
// ═══════════════════════════════════════════════════════════════════════════════

/// The bank's rules for status requests (HKTAN process S) while a
/// decoupled approval is pending, from HITANS 7. `None` everywhere the
/// bank set no rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DecoupledParams {
    /// Wait at least this long before the first status request.
    pub wait_before_first_poll: Option<std::time::Duration>,
    /// Wait at least this long between two status requests.
    pub wait_before_next_poll: Option<std::time::Duration>,
    /// Send no more status requests than this.
    pub max_polls: Option<u32>,
    /// Whether the client may ask without the user confirming first.
    pub auto_polling_allowed: Option<bool>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Dialog<S> — the typestate dialog
// ═══════════════════════════════════════════════════════════════════════════════

pub(crate) struct Dialog<S: std::fmt::Debug> {
    transport: SharedTransport,
    url: String,
    quirks: QuirkChain,
    blz: Blz,
    user_id: UserId,
    pin: Pin,
    system_id: SystemId,
    product_id: ProductId,
    dialog_id: DialogId,
    message_number: u16,
    pub params: BankParams,
    /// The bank profile's HKTAN overrides, beating HIPINS.
    tan_overrides: TanOverrides,
    /// Notices of the messages since the last [`Dialog::take_notices`].
    notices: Vec<Notice>,
    /// The dialog the notices are tagged with.
    notice_dialog: NoticeDialog,
    /// Pinned local time ([`crate::BankClient::with_now`]), else the wall clock.
    now: Option<jiff::civil::DateTime>,
    _state: PhantomData<S>,
}

impl<S: std::fmt::Debug> std::fmt::Debug for Dialog<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialog")
            .field("blz", &self.blz)
            .field("user_id", &self.user_id)
            .field("system_id", &self.system_id)
            .field("dialog_id", &self.dialog_id)
            .field("message_number", &self.message_number)
            .field("state", &std::any::type_name::<S>())
            .finish()
    }
}

// Note: Pin intentionally Debug-redacted (shows "Pin(****)")

// ── Shared internals (all states) ───────────────────────────────────────────

impl<S: std::fmt::Debug> Dialog<S> {
    pub fn system_id(&self) -> &SystemId {
        &self.system_id
    }
    /// The notices of every message exchanged since the last call (or the
    /// dialog's start), including touchdown pages and TAN resubmits.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }
    pub fn bank_params(&self) -> &BankParams {
        &self.params
    }
    pub(crate) fn tan_overrides(&self) -> &TanOverrides {
        &self.tan_overrides
    }

    /// The dialog's local time: pinned, else the wall clock.
    pub(crate) fn now(&self) -> jiff::civil::DateTime {
        self.now
            .unwrap_or_else(|| jiff::ZonedDateTime::now().datetime())
    }

    /// Build the standard HKIDN segment for this dialog.
    fn identify_segment(&self) -> Segment {
        Segment::Identify {
            blz: self.blz.clone(),
            user_id: self.user_id.clone(),
            system_id: self.system_id.clone(),
        }
    }

    /// Build the standard HKVVB segment for this dialog.
    fn process_prep_segment(&self) -> Segment {
        Segment::ProcessPrep {
            bpd_version: self.params.bpd_version,
            upd_version: self.params.upd_version,
            product_id: self.product_id.clone(),
        }
    }

    /// The next message of this dialog around `segments`, signed with the
    /// PIN and, when given, the TAN (HNSHA).
    fn message(&self, segments: &[Segment], tan: Option<&str>) -> Result<Vec<u8>> {
        let signer = message::SecurityContext {
            dialog_id: self.dialog_id.as_str(),
            message_number: self.message_number,
            blz: self.blz.as_str(),
            user_id: self.user_id.as_str(),
            system_id: self.system_id.as_str(),
            pin: self.pin.as_str(),
            tan,
            security_function: self.params.selected_security_function.as_str(),
        };
        message::build_from_segments(&signer, segments, &self.params, self.now())
    }

    /// Sends `segments` signed with the PIN only.
    async fn send_segments(&mut self, segments: &[Segment]) -> Result<Response> {
        self.send(segments, None).await
    }

    /// Sends `segments` with `tan` in the signature closing segment.
    async fn send_segments_with_tan(
        &mut self,
        segments: &[Segment],
        tan: &str,
    ) -> Result<Response> {
        self.send(segments, Some(tan)).await
    }

    async fn send(&mut self, segments: &[Segment], tan: Option<&str>) -> Result<Response> {
        let request = self.message(segments, tan)?;
        if tan.is_none() {
            // Masked by segment (PIN, TAN, IDs, accounts); cut by characters,
            // since a byte index can fall inside an umlaut.
            let masked: String = crate::mask::mask_for_log(&request)
                .chars()
                .take(500)
                .collect();
            info!("[FinTS] Outgoing ({} bytes): {masked}", request.len());
        }
        let answer = self.exchange(&request).await?;
        let response = parse_answer(&answer, self.message_number - 1, &request)?;
        response.log_informational_codes();
        self.notices.extend(response.notices(self.notice_dialog));
        Ok(response)
    }

    async fn send_end(&mut self) -> Result<()> {
        if !self.dialog_id.is_assigned() {
            return Ok(());
        }
        debug!("Ending dialog {}", self.dialog_id);
        let end = [Segment::End {
            dialog_id: self.dialog_id.clone(),
        }];
        let msg_bytes = self.message(&end, None)?;
        let _ = self.exchange(&msg_bytes).await;
        self.dialog_id = DialogId::unassigned();
        Ok(())
    }

    /// One request and its answer, both through the quirk chain. The next
    /// message of the dialog gets the next number.
    async fn exchange(&mut self, request: &[u8]) -> Result<Vec<u8>> {
        let number = self.message_number;
        self.message_number = number + 1;
        let Self {
            transport,
            url,
            quirks,
            dialog_id,
            ..
        } = &*self;
        let ctx = crate::quirks::QuirkContext {
            hbci_version: "300",
            dialog_id: dialog_id.as_str(),
            message_number: number,
        };
        crate::transport::exchange(transport.as_ref(), url, quirks, &ctx, request).await
    }

    /// Sends a dialog initialisation (or another message that may carry
    /// BPD and UPD) and takes over what the answer hands out: the dialog
    /// ID from its header, and parameters, system ID and allowed methods.
    async fn send_and_adopt(&mut self, segments: &[Segment]) -> Result<Response> {
        let response = self.send_segments(segments).await?;
        let header = response.find_segment("HNHBK");
        let assigned = header
            .and_then(crate::segments::read::message_header)
            .map(|(id, _)| DialogId::new(id))
            .filter(DialogId::is_assigned);
        if let Some(id) = assigned {
            self.dialog_id = id;
        }
        self.params.ingest_response(&response, &mut self.system_id);
        Ok(response)
    }

    pub(crate) fn blz(&self) -> &Blz {
        &self.blz
    }

    pub(crate) fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn dialog_id(&self) -> &DialogId {
        &self.dialog_id
    }

    /// The number the next message will carry.
    pub(crate) fn message_number(&self) -> u16 {
        self.message_number
    }

    /// Take BPD, UPD, system ID, 3920 and 3072 from a response.
    pub(crate) fn ingest(&mut self, response: &Response) {
        self.params.ingest_response(response, &mut self.system_id);
    }

    /// HKTAN process 2 for `task_reference`, after the `extra` segments
    /// (HKVPA for a payment, else none), with `tan` in HNSHA (PIN/TAN
    /// B.4.2). Works for the login and for any order. Returns the bank's
    /// answer unchecked.
    pub(crate) async fn send_tan(
        &mut self,
        extra: &[Segment],
        task_reference: &TaskReference,
        tan: &str,
    ) -> Result<Response> {
        let mut segments = extra.to_vec();
        segments.push(Segment::TanProcess2 {
            task_reference: task_reference.clone(),
            tan_medium: self.params.selected_tan_medium.clone(),
        });
        self.send_segments_with_tan(&segments, tan).await
    }

    /// One decoupled status request for `task_reference` (HKTAN process S,
    /// or 2 before HKTAN 7), after the `extra` segments (HKVPA on a
    /// payment's first poll, else none). Returns the bank's answer
    /// unchecked.
    pub(crate) async fn send_tan_status(
        &mut self,
        extra: &[Segment],
        task_reference: &TaskReference,
    ) -> Result<Response> {
        let mut segments = extra.to_vec();
        segments.push(Segment::TanPollDecoupled {
            task_reference: task_reference.clone(),
            tan_medium: self.params.selected_tan_medium.clone(),
        });
        self.send_segments(&segments).await
    }

    /// HKEND, best effort; nothing when the dialog has no ID (never
    /// opened, or forgotten).
    pub(crate) async fn end_quietly(&mut self) {
        self.send_end().await.ok();
    }

    /// Forget the dialog ID: the bank ended the dialog, or its message
    /// number is unknown, so no HKEND is sent for it.
    pub(crate) fn forget_dialog(&mut self) {
        self.dialog_id = DialogId::unassigned();
    }

    pub(crate) fn transition<T: std::fmt::Debug>(self) -> Dialog<T> {
        Dialog {
            transport: self.transport,
            url: self.url,
            quirks: self.quirks,
            blz: self.blz,
            user_id: self.user_id,
            pin: self.pin,
            system_id: self.system_id,
            product_id: self.product_id,
            dialog_id: self.dialog_id,
            message_number: self.message_number,
            params: self.params,
            tan_overrides: self.tan_overrides,
            notices: self.notices,
            notice_dialog: self.notice_dialog,
            now: self.now,
            _state: PhantomData,
        }
    }
}

// ── Dialog<New> ─────────────────────────────────────────────────────────────

impl Dialog<New> {
    pub fn new(
        url: &str,
        blz: &Blz,
        user_id: &UserId,
        pin: &Pin,
        product_id: &ProductId,
    ) -> Result<Self> {
        Ok(Self {
            transport: crate::transport::default_transport()?,
            url: url.to_string(),
            quirks: QuirkChain::default(),
            blz: blz.clone(),
            user_id: user_id.clone(),
            pin: pin.clone(),
            system_id: SystemId::unassigned(),
            product_id: product_id.clone(),
            dialog_id: DialogId::unassigned(),
            message_number: 1,
            params: BankParams::new(),
            tan_overrides: TanOverrides::default(),
            notices: Vec::new(),
            notice_dialog: NoticeDialog::Job,
            now: None,
            _state: PhantomData,
        })
    }

    /// Pin the local time of this dialog's signatures and orders.
    pub fn with_now(mut self, now: Option<jiff::civil::DateTime>) -> Self {
        self.now = now;
        self
    }

    pub fn with_system_id(mut self, system_id: &SystemId) -> Self {
        self.system_id = system_id.clone();
        self
    }

    pub fn with_params(mut self, params: &BankParams) -> Self {
        self.params = params.clone();
        self
    }

    pub fn with_security_function(mut self, function: SecurityFunction) -> Self {
        self.params.selected_security_function = function;
        self
    }

    /// Use a custom quirk chain instead of the default one
    /// (e.g. `QuirkChain::default().with(MyBankFix)`, or `QuirkChain::none()`).
    /// Continue a dialog the bank opened earlier (a thawed paused dialog):
    /// its ID, and the number its next message carries.
    pub(crate) fn resume(mut self, dialog_id: DialogId, message_number: u16) -> Dialog<Open> {
        self.dialog_id = dialog_id;
        self.message_number = message_number;
        self.transition()
    }

    pub(crate) fn with_quirks(mut self, quirks: crate::quirks::QuirkChain) -> Self {
        self.quirks = quirks;
        self
    }

    /// Send through `transport` instead of the default (`HttpTransport` with feature `http`)
    /// (custom timeout, recording, in-process banks).
    pub fn with_transport(mut self, transport: SharedTransport) -> Self {
        self.transport = transport;
        self
    }

    /// The bank profile's per-order HKTAN overrides (beat HIPINS).
    pub fn with_tan_overrides(mut self, overrides: TanOverrides) -> Self {
        self.tan_overrides = overrides;
        self
    }

    /// Synchronization dialog (spec: Initialisierung mit Synchronisierung).
    ///
    /// Sends HKIDN + HKVVB + HKSYN (mode=0 for new system_id).
    /// The bank responds with BPD, UPD, HITANS, HIPINS, and HISYN(system_id).
    /// This dialog is ONLY for synchronization — no business segments allowed.
    pub async fn sync(mut self) -> Result<(Dialog<Synced>, Response)> {
        info!("[FinTS] Sync dialog: BLZ={}", self.blz);
        self.notice_dialog = NoticeDialog::Sync;

        let segments = [
            self.identify_segment(),
            self.process_prep_segment(),
            Segment::Sync,
        ];

        let response = self.send_and_adopt(&segments).await?;
        self.params.select_security_function();

        // Check for fatal errors (sync dialog should not require TAN)
        if !response.needs_tan() {
            response.check_errors()?;
        }

        // Log all BPD parameter segments for diagnostics
        let bpd_summary: Vec<String> = self
            .params
            .bpd_segments
            .iter()
            .map(|s| format!("{}:v{}", s.segment_type(), s.segment_version()))
            .collect();
        info!(
            "[FinTS] Sync complete: BPD v{}, {} TAN methods",
            self.params.bpd_version,
            self.params.tan_methods.len()
        );
        info!(
            "[FinTS] BPD segments ({}): {}",
            bpd_summary.len(),
            bpd_summary.join(", ")
        );
        if let Some(bpd) = self.params.bpd() {
            for u in bpd.unparsed() {
                warn!("[FinTS] BPD segment not fully read: {u:?}");
            }
        }

        Ok((self.transition(), response))
    }

    /// Normal dialog initialization (spec: Dialoginitialisierung).
    ///
    /// Sends HKIDN + HKVVB + HKTAN(process=4, ref=HKIDN).
    /// Response-driven: returns `InitResult::Opened` or `InitResult::TanRequired`
    /// based on the bank's response codes.
    pub async fn init(mut self) -> Result<InitResult> {
        let medium = self.params.selected_tan_medium.clone();
        info!(
            "[FinTS] Init dialog: BLZ={} security_fn={}",
            self.blz, self.params.selected_security_function
        );

        let mut segments = vec![self.identify_segment(), self.process_prep_segment()];
        if self.params.init_hktan_version()?.is_some() {
            segments.push(Segment::TanProcess4 {
                reference_seg: SegmentRef::new("HKIDN"),
                tan_medium: medium,
            });
        } else {
            info!("[FinTS] Init dialog: no HKTAN (selected method below HKTAN 6, or PIN only)");
        }

        let response = self.send_and_adopt(&segments).await?;

        let allowed = response.allowed_security_functions();
        if !allowed.is_empty() {
            self.params.allowed_security_functions = allowed;
            self.params.select_security_function();
        }

        for c in response.all_codes() {
            info!("[FinTS] Init: {} - {}", c.code, c.text);
        }

        // Response-driven transition per spec:
        if response.needs_tan()
            && let Some(challenge) = response.get_tan_challenge()?
        {
            // 0030/3955: TAN required on init → TanPending
            // Decoupled only on the bank's word (3955, PIN/TAN TAN-Prozess
            // 4): a decoupled method may still ask for a typed TAN (0030).
            let challenge = TanChallenge {
                challenge_markup: self.params.challenge_structured(),
                ..challenge
            };
            info!(
                "[FinTS] Init requires TAN: decoupled={}",
                challenge.decoupled
            );
            return Ok(InitResult::TanRequired(self.transition(), challenge));
        }

        // 0010/0020 or 3076: dialog opened, no TAN needed → Open
        response.check_errors()?;
        info!("[FinTS] Init opened without TAN");
        Ok(InitResult::Opened(self.transition()))
    }

    /// Learn the bank's parameters without a synchronization dialog, for a
    /// client that has a system ID but no BPD (e.g. migrated from another
    /// store). HKSYN would assign a new system ID, so the bank would see a
    /// new device and might withhold an SCA exemption (3076).
    ///
    /// Sends HKIDN + HKVVB with the stored system ID, signed PIN-only (999)
    /// and without HKTAN (HKTAN with 999 is an invalid combination). Whether
    /// the bank opens a dialog or aborts it, it answers with its BPD, the UPD
    /// and the allowed security functions (3920); the dialog is ended right
    /// away. Wrong PIN, locked access and a rejected system ID are errors.
    /// Returns the parameters with the best security function selected.
    pub async fn probe_parameters(mut self) -> Result<(BankParams, SystemId)> {
        info!(
            "[FinTS] Parameter probe: BLZ={} with stored system ID",
            self.blz
        );
        self.params.selected_security_function = SecurityFunction::pin_only();
        let segments = [self.identify_segment(), self.process_prep_segment()];
        let response = self.send_and_adopt(&segments).await?;
        for c in response.all_codes() {
            info!("[FinTS] Probe: {} - {}", c.code, c.text);
        }
        let fatal = response.all_codes().find(|c| {
            matches!(
                c.kind(),
                Kind::PinWrong
                    | Kind::SignatureInvalid
                    | Kind::AccountLocked
                    | Kind::SystemIdInvalid
            )
        });
        if fatal.is_some() {
            response.check_errors()?;
        }
        self.send_end().await.ok();
        self.params.select_security_function();
        if self.params.bpd().is_none() {
            return Err(FinTSError::Dialog(
                "parameter probe: the bank sent no BPD".into(),
            ));
        }
        Ok((self.params, self.system_id))
    }

    /// Initialize WITHOUT HKTAN — PIN-only. Used when bank doesn't require SCA.
    pub async fn init_no_tan(mut self) -> Result<(Dialog<Open>, Response)> {
        info!("[FinTS] Init (no HKTAN)");
        let segments = [self.identify_segment(), self.process_prep_segment()];

        let response = self.send_and_adopt(&segments).await?;
        response.check_errors()?;
        Ok((self.transition(), response))
    }
}

// ── Dialog<Synced> ──────────────────────────────────────────────────────────

impl Dialog<Synced> {
    /// End the sync dialog. Returns bank params and system_id for use in normal dialogs.
    pub async fn end(mut self) -> Result<(BankParams, SystemId)> {
        self.send_end().await.ok();
        Ok((self.params, self.system_id))
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Account — the account an order is for
// ═══════════════════════════════════════════════════════════════════════════════

/// The account an order is for, as KTI (international: IBAN and BIC) and,
/// where known, KTV (national: number, subaccount, bank code).
///
/// The constructors refuse an account the bank could not address: no IBAN
/// or no BIC for [`Account::new`], no number or no valid bank code for
/// [`Account::national`]. Orders on `Dialog<Open>` take `&Account`, so
/// they never see an empty identifier.
///
/// ```
/// use fints_client::protocol::Account;
///
/// let account = Account::new("DE14 1234 5678 0000 0047 11", "TESTDEFFXXX").unwrap();
/// assert_eq!(account.iban(), "DE14123456780000004711");
/// assert_eq!(account.account_number(), Some("4711"));
///
/// assert!(Account::new("DE14123456780000004711", " ").is_err());
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    iban: Iban,
    bic: Bic,
    /// KTV account number, for the segment versions that address accounts
    /// nationally.
    number: Option<String>,
    /// KTV "Unterkontomerkmal".
    subaccount: Option<String>,
    /// KTV bank code.
    blz: Option<Blz>,
    /// Send number, subaccount and bank code in KTI too (`IBAN:BIC:number::280:BLZ`).
    /// Set from the BPD (HISPAS) when the bank allows national account details.
    kti_national: bool,
}

/// `None` for a missing or blank subaccount.
fn subaccount_value(subaccount: Option<&str>) -> Option<String> {
    subaccount.filter(|s| !s.is_empty()).map(String::from)
}

impl Account {
    /// An account by IBAN and BIC; fails when either is blank.
    ///
    /// Blanks are removed from the IBAN and letters upper-cased. A German
    /// IBAN contains the bank code (positions 5 to 12) and the account
    /// number (13 to 22), which become the KTV that HKSAL 5 and 6, HKKAZ 5
    /// and 6 and HKWPD need.
    pub fn new(iban: &str, bic: &str) -> Result<Self> {
        let iban: String = iban
            .split_whitespace()
            .flat_map(str::chars)
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let bic = bic.trim();
        if iban.is_empty() {
            return Err(FinTSError::Dialog("the account has no IBAN".into()));
        }
        if bic.is_empty() {
            return Err(FinTSError::Dialog(
                "the account has no BIC; set it in the account settings".into(),
            ));
        }
        let national = german_iban_parts(&iban);
        Ok(Self {
            number: national.as_ref().map(|(_, number)| number.clone()),
            blz: national.map(|(blz, _)| Blz::new(blz)),
            iban: Iban::new(iban),
            bic: Bic::new(bic),
            subaccount: None,
            kti_national: false,
        })
    }

    /// An account by its KTV alone, the way the UPD lists most depots.
    /// HKWPD addresses such an account; orders whose segment version needs
    /// an IBAN fail for it. National account details exist only in Germany
    /// (country code 280), so the bank code must have 8 digits.
    pub fn national(number: &str, subaccount: Option<&str>, blz: &Blz, bic: &str) -> Result<Self> {
        let number = number.trim();
        let code = blz.as_str();
        if number.is_empty() || code.is_empty() {
            return Err(FinTSError::Dialog(
                "a national account needs an account number and a bank code".into(),
            ));
        }
        if code.len() != 8 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(FinTSError::Dialog(format!(
                "bank code {code:?} is not 8 digits"
            )));
        }
        let account = Self {
            iban: Iban::new(""),
            bic: Bic::new(bic.trim()),
            number: None,
            subaccount: None,
            blz: None,
            kti_national: false,
        };
        Ok(account.with_national(number, subaccount, blz))
    }

    /// The same account with this subaccount, for an account the UPD
    /// lists by IBAN and subaccount.
    pub(crate) fn with_subaccount(self, subaccount: Option<&str>) -> Self {
        Self {
            subaccount: subaccount_value(subaccount),
            ..self
        }
    }

    /// The same account with this KTV, which replaces anything taken from
    /// the IBAN; used with the details the UPD gives.
    pub fn with_national(self, number: &str, subaccount: Option<&str>, blz: &Blz) -> Self {
        Self {
            number: Some(number.to_owned()),
            subaccount: subaccount_value(subaccount),
            blz: Some(blz.clone()),
            ..self
        }
    }

    /// Whether KTI carries the national parts too (see [`Account::kti_national`] field).
    pub(crate) fn with_kti_national(self, kti_national: bool) -> Self {
        Self {
            kti_national,
            ..self
        }
    }

    pub(crate) fn kti_national(&self) -> bool {
        self.kti_national
    }

    /// The IBAN without blanks; empty for an account known nationally only.
    pub fn iban(&self) -> &str {
        self.iban.as_str()
    }

    pub fn bic(&self) -> &str {
        self.bic.as_str()
    }

    /// The KTV account number, if known.
    pub fn account_number(&self) -> Option<&str> {
        self.number.as_deref()
    }

    /// The KTV subaccount, if any.
    pub fn subaccount(&self) -> Option<&str> {
        self.subaccount.as_deref()
    }

    /// The KTV bank code, if known.
    pub fn blz(&self) -> Option<&str> {
        self.blz.as_ref().map(Blz::as_str)
    }
}

/// Bank code and account number of a German IBAN (`DE`, two check digits,
/// eight digits bank code, ten digits account number). The account number
/// loses its leading zeros, keeping a single `0` for an all-zero number.
fn german_iban_parts(iban: &str) -> Option<(String, String)> {
    let digits = iban.strip_prefix("DE")?;
    if digits.len() != 20 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (bank_code, account) = (&digits[2..10], &digits[10..]);
    let significant = account.trim_start_matches('0');
    let number = if significant.is_empty() {
        "0"
    } else {
        significant
    };
    Some((bank_code.to_owned(), number.to_owned()))
}

// ── Dialog<Open> ─────────────────────────────────────────────────────────────

impl Dialog<Open> {
    /// Send a business order, with the 9075 retry and learning.
    pub(crate) async fn send_order(&mut self, segment: Segment, code: &str) -> Result<Response> {
        let (mut response, decision) = self
            .send_order_decided(&[], segment.clone(), code, None)
            .await?;
        match on_sca_required(&decision, response.all_codes().map(ReturnCode::kind)) {
            Retry::No => {}
            Retry::SameDialog => {
                info!(
                    "[FinTS] {}: 9075 without HKTAN, sending it again with HKTAN",
                    code
                );
                let (retry, _) = self
                    .send_order_decided(&[], segment.clone(), code, Some(true))
                    .await?;
                if !retry.all_codes().any(|c| c.is_error()) {
                    info!("[FinTS] {}: 9075 without HKTAN, learned", code);
                    self.params
                        .tan_required_learned
                        .insert(SegmentType::new(code));
                }
                response = retry;
            }
            Retry::DialogGone => {
                info!("[FinTS] {}: 9075 with 9800 without HKTAN, learned", code);
                self.params
                    .tan_required_learned
                    .insert(SegmentType::new(code));
                return Ok(response);
            }
        }
        Ok(response)
    }

    /// A payment order (HKCCS, HKIPZ) after the `extra` segments (HKVPP in
    /// message 1, HKVPA in the execution message), with HKTAN process 4 as
    /// `order_tan` decides (always, unless a profile override says
    /// otherwise). No 9075 retry: a payment is never sent twice.
    pub(crate) async fn send_payment(
        &mut self,
        extra: &[Segment],
        order: Segment,
        code: &str,
    ) -> Result<Response> {
        Ok(self.send_order_decided(extra, order, code, None).await?.0)
    }

    /// HKVPP alone: a payee check poll (VoP C.10.7.1.1). No order, no
    /// HKTAN.
    pub(crate) async fn send_vop_poll(&mut self, poll: Segment) -> Result<Response> {
        self.send_segments(&[poll]).await
    }

    /// Send an order after the `extra` segments, with HKTAN as `order_tan`
    /// decides, or as `force` says (the 9075 retry). Returns the decision
    /// that was used.
    async fn send_order_decided(
        &mut self,
        extra: &[Segment],
        segment: Segment,
        code: &str,
        force: Option<bool>,
    ) -> Result<(Response, TanDecision)> {
        let mut segments = extra.to_vec();
        segments.push(segment);
        let mut decision = order_tan(
            &SegmentType::new(code),
            self.params.selected_security_function != SecurityFunction::pin_only(),
            self.params.bpd().and_then(|b| b.tan_required(code)),
            &self.tan_overrides,
            &self.params.tan_required_learned,
        );
        if let Some(hktan) = force {
            decision.hktan = hktan;
        }
        if decision.hktan {
            info!("[FinTS] {}: + HKTAN:4 ({:?})", code, decision.reason);
            segments.push(Segment::TanProcess4 {
                reference_seg: SegmentRef::new(code),
                tan_medium: self.params.selected_tan_medium.clone(),
            });
        } else {
            info!("[FinTS] {}: no HKTAN ({:?})", code, decision.reason);
        }
        let response = self.send_segments(&segments).await?;
        for c in response.all_codes() {
            if matches!(c.class(), Class::Error | Class::Warning) {
                info!("[FinTS] {}: {} - {}", code, c.code, c.text);
            }
        }
        Ok((response, decision))
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Response parsing
// ═══════════════════════════════════════════════════════════════════════════════

/// The segments of a message with HNVSD replaced by the segments it holds.
fn unwrap_hnvsd(outer_segments: Vec<RawSegment>) -> Result<Vec<RawSegment>> {
    let mut all_segments = Vec::new();
    for seg in outer_segments {
        if seg.segment_type() == "HNVSD" {
            match crate::segments::read::hnvsd_data(&seg)
                .map_err(|e| e.to_string())
                .and_then(|data| crate::codec::wire::tokenize(&data).map_err(|e| e.to_string()))
            {
                Ok(inner) => all_segments.extend(inner),
                // The encrypted envelope holds the whole message: if it does
                // not read, nothing in it is known.
                Err(e) => return Err(FinTSError::Parse(format!("unreadable HNVSD: {e}"))),
            }
        } else {
            all_segments.push(seg);
        }
    }
    Ok(all_segments)
}

/// Parses the bank's answer to `request` (the sent message) and names the
/// order each HIRMS code refers to.
fn parse_answer(data: &[u8], expected_msg_num: u16, request: &[u8]) -> Result<Response> {
    let mut response = parse_response(data, expected_msg_num)?;
    // Our own message: it always tokenizes; if not, orders stay unnamed.
    if let Ok(sent) = crate::codec::wire::tokenize(request)
        .map_err(|e| FinTSError::Parse(e.to_string()))
        .and_then(unwrap_hnvsd)
    {
        response.name_orders(&sent);
    }
    Ok(response)
}

fn parse_response(data: &[u8], expected_msg_num: u16) -> Result<Response> {
    use crate::segments::read;

    let outer_segments =
        crate::codec::wire::tokenize(data).map_err(|e| FinTSError::Parse(e.to_string()))?;

    if let Some((_, resp_num)) = outer_segments
        .iter()
        .find(|s| s.segment_type() == "HNHBK")
        .and_then(read::message_header)
        && resp_num != u64::from(expected_msg_num)
    {
        warn!(
            "Message number mismatch: expected {}, got {}",
            expected_msg_num, resp_num
        );
    }

    // The size in HNHBK counts the answer with its envelope, which is gone
    // here; `transport::exchange` checks it on the answer as it arrived.

    let all_segments = unwrap_hnvsd(outer_segments)?;

    // Every answer carries HIRMG (Formals B.7.1, message layout: "Rückmeldungen zur
    // Gesamtnachricht" is M). Without it the answer is cut off or not
    // a FinTS answer, and must not pass as a success.
    if !all_segments.iter().any(|s| s.segment_type() == "HIRMG") {
        return Err(FinTSError::InvalidResponse(
            "answer without HIRMG (cut off or not a FinTS answer)".into(),
        ));
    }

    let mut global_codes = Vec::new();
    let mut segment_codes = Vec::new();
    for seg in &all_segments {
        let target = match seg.segment_type() {
            "HIRMG" => &mut global_codes,
            "HIRMS" => &mut segment_codes,
            _ => continue,
        };
        // An unreadable HIRMG/HIRMS makes `check_errors` fail.
        match read::response_codes(seg) {
            Ok(codes) => target.extend(codes),
            Err(e) => read::report(&e),
        }
    }

    Ok(Response {
        segments: all_segments,
        global_codes,
        segment_codes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_surfaces_codec_errors_of_raw_segments() {
        let response = Response {
            segments: crate::codec::wire::tokenize(
                b"HIRMG:2:2+0010::ok'\
                  HITANS:3:6:4+1+1+1+J:N:0:921:2:T:HHD:1.4:Name:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0'\
                  HITANS:4:6:4+1+1+1+J:N:0:921:2:T:HHD:1.4:Name:6:1:TAN:x'",
            )
            .unwrap(),
            global_codes: vec![],
            segment_codes: vec![],
        };
        let decoded = response.decode_segments();
        assert_eq!(decoded.len(), 3);
        assert!(decoded[1].typed().is_some());
        let errors = response.segment_errors();
        assert_eq!(errors.len(), 1, "HIRMG has no definition yet: not an error");
        assert_eq!(
            errors[0].field,
            "parameters.methods[0].max_return_value_length"
        );
    }

    #[test]
    fn camt_format_choice_by_version_number() {
        let pick = |formats: &str| {
            params_with_bpd(&format!("HICAZS:1:1:4+1+1+1+450:N:N:{formats}'")).camt052_format()
        };
        let u = "urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001";
        let plain = |n: &str| format!("urn:iso:std:iso:20022:tech:xsd:camt.052.001.{n}");
        // Same version: the descriptor without .xsd, as the bank wrote it.
        assert_eq!(pick(&format!("{u}.02.xsd:{u}.02")), plain("02"));
        assert_eq!(pick(&format!("{u}.02:{u}.02.xsd")), plain("02"));
        assert_eq!(pick(&format!("{u}.02:{u}.08")), plain("08"));
        assert_eq!(
            pick(&format!("{u}.08.xsd:{u}.02")),
            format!("{}.xsd", plain("08"))
        );
        // Numeric, not string order.
        assert_eq!(pick(&format!("{u}.10:{u}.9")), plain("10"));
    }

    #[test]
    fn camt_format_from_hicazs() {
        assert_eq!(
            BankParams::new().camt052_format(),
            "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02"
        );
        let p = params_with_bpd(
            "HICAZS:1:1:4+1+1+1+450:N:N:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.08:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02'",
        );
        assert_eq!(
            p.camt052_format(),
            "urn:iso:std:iso:20022:tech:xsd:camt.052.001.08"
        );
        let seg = Segment::CamtTransactions {
            account: Account::new("DE00120300009900012345", "BYLADEM1001").unwrap(),
            start_date: Date::new(2026, 1, 1).unwrap(),
            end_date: Date::new(2026, 1, 31).unwrap(),
            touchdown: None,
        };
        assert!(seg.encode(&p, 3).is_ok());
        // A bank without HICAZS: VersionMismatch, not a guessed version.
        let no_camt = params_with_bpd("HIKAZS:1:7:4+1+1+1'");
        assert!(matches!(
            seg.encode(&no_camt, 3),
            Err(FinTSError::VersionMismatch(_))
        ));
    }

    fn params_with_bpd(raw: &str) -> BankParams {
        // The model needs a HIBPA; the tests only care about what follows it.
        let mut p = BankParams::new();
        let wire = format!("HIBPA:1:3:4+1+280:1+Bank+1+1+300'{raw}");
        p.set_bpd(crate::codec::wire::tokenize(wire.as_bytes()).unwrap());
        p
    }

    /// The orders a job exists for: all.
    fn built_orders() -> impl Iterator<Item = Order> {
        Order::ALL.iter().copied()
    }

    /// The parameter segment of an order offering `versions`.
    fn param_segments(order: Order, versions: &[u16]) -> String {
        let code = order.code();
        let param = format!("{}I{}S", &code[..1], &code[2..]);
        versions
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{param}:{}:{v}:4+1+1+0'", i + 2))
            .collect()
    }

    #[test]
    fn order_version_picks_highest_common_version() {
        for order in built_orders() {
            let implemented = order.implemented();
            let max = *implemented.last().unwrap();
            let mut offered = implemented.to_vec();
            offered.push(max + 1);
            let p = params_with_bpd(&param_segments(order, &offered));
            assert_eq!(p.order_version(order).unwrap(), max, "{order:?}");
            let p = params_with_bpd(&param_segments(order, &implemented[..1]));
            assert_eq!(p.order_version(order).unwrap(), implemented[0], "{order:?}");
        }
    }

    #[test]
    fn order_version_without_overlap_names_both_sides() {
        for order in built_orders() {
            let max = *order.implemented().last().unwrap();
            let p = params_with_bpd(&param_segments(order, &[max + 1, max + 2]));
            let Err(FinTSError::VersionMismatch(m)) = p.order_version(order) else {
                panic!("{order:?}: expected VersionMismatch");
            };
            assert_eq!(m.code.as_str(), order.code());
            assert_eq!(m.offered, [max + 1, max + 2]);
            assert_eq!(m.implemented, order.implemented());
        }
        let p = params_with_bpd("HICAZS:1:2:4+1+1+1'");
        assert_eq!(
            p.order_version(Order::Camt).unwrap_err().to_string(),
            "HKCAZ: no common version (bank offers 2, client implements 1)"
        );
    }

    #[test]
    fn order_version_order_not_in_bpd() {
        let p = params_with_bpd("HIKAZS:1:7:4+1+1+1'");
        for &order in Order::ALL.iter().filter(|o| **o != Order::Transactions) {
            let Err(FinTSError::VersionMismatch(m)) = p.order_version(order) else {
                panic!("{order:?}: expected VersionMismatch");
            };
            assert!(m.offered.is_empty(), "{order:?}");
        }
        assert_eq!(
            p.order_version(Order::Camt).unwrap_err().to_string(),
            "HKCAZ: bank does not offer this order (client implements 1)"
        );
    }

    #[test]
    fn order_version_without_bpd_is_an_error() {
        for &order in Order::ALL {
            let err = BankParams::new().order_version(order).unwrap_err();
            assert!(
                matches!(&err, FinTSError::NoBpd { code } if code.as_str() == order.code()),
                "{order:?}: {err}"
            );
        }
        assert_eq!(
            BankParams::new()
                .order_version(Order::Balance)
                .unwrap_err()
                .to_string(),
            "HKSAL: no bank parameter data (BPD) known; synchronise first"
        );
    }

    fn tan_method(sf: &str, v: u16) -> TanMethod {
        TanMethod {
            security_function: SecurityFunction::new(sf),
            tan_process: TanProcess::TwoStep,
            name: sf.into(),
            needs_tan_medium: false,
            decoupled_max_polls: -1,
            wait_before_first_poll: 0,
            wait_before_next_poll: 0,
            is_decoupled: false,
            hktan_version: v,
            zka_name: None,
            decoupled_auto_polling: None,
            challenge_structured: false,
        }
    }

    fn params_with_methods(methods: &[(&str, u16)], selected: &str) -> BankParams {
        let mut p = BankParams::new();
        p.tan_methods = methods.iter().map(|(sf, v)| tan_method(sf, *v)).collect();
        p.selected_security_function = SecurityFunction::new(selected);
        p
    }

    fn hktan_wire(p: &BankParams) -> String {
        let seg = Segment::TanProcess4 {
            reference_seg: SegmentRef::new("HKIDN"),
            tan_medium: None,
        };
        let raw = seg.encode(p, 3).unwrap();
        String::from_utf8(crate::codec::wire::serialize_segment(&raw).unwrap()).unwrap()
    }

    #[test]
    fn hktan_version_is_the_highest_buildable_of_the_selected_method() {
        let p = params_with_methods(&[("902", 6), ("902", 7), ("902", 8), ("912", 6)], "902");
        assert_eq!(p.hktan_version().unwrap(), 7);
        assert_eq!(p.init_hktan_version().unwrap(), Some(7));
        let p = params_with_methods(&[("902", 6), ("902", 7), ("912", 6)], "912");
        assert_eq!(p.hktan_version().unwrap(), 6, "not another method's 7");
    }

    #[test]
    fn hktan_version_without_a_buildable_version_names_both_sides() {
        let p = params_with_methods(&[("902", 8), ("912", 7)], "902");
        let err = p.hktan_version().unwrap_err();
        assert!(
            matches!(
                &err,
                FinTSError::VersionMismatch(v)
                    if v.code.as_str() == "HKTAN" && v.offered == [8] && v.implemented == [5, 6, 7]
            ),
            "{err}"
        );
        // The backstop reaches the wire builder and the init decision.
        assert!(p.init_hktan_version().is_err());
        let seg = Segment::TanPollDecoupled {
            task_reference: TaskReference::new("T"),
            tan_medium: None,
        };
        assert!(matches!(
            seg.encode(&p, 3),
            Err(FinTSError::VersionMismatch(_))
        ));
        // A method no HITANS lists has no version either (empty `offered`).
        let p = params_with_methods(&[("912", 7)], "902");
        assert!(matches!(
            p.hktan_version(),
            Err(FinTSError::VersionMismatch(v)) if v.offered.is_empty()
        ));
    }

    #[test]
    fn hktan_version_without_hitans_is_7() {
        let p = BankParams::new();
        assert_eq!(p.hktan_version().unwrap(), 7);
        assert_eq!(p.init_hktan_version().unwrap(), Some(7));
        assert_eq!(hktan_wire(&p), "HKTAN:3:7+4+HKIDN'");
    }

    #[test]
    fn init_without_hktan_when_the_method_is_below_6() {
        let p = params_with_methods(&[("902", 5), ("912", 7)], "902");
        assert_eq!(
            p.hktan_version().unwrap(),
            5,
            "orders use the method's version"
        );
        assert_eq!(p.init_hktan_version().unwrap(), None);
        assert!(hktan_wire(&p).starts_with("HKTAN:3:5+4+"));
        // PIN only with HITANS known: nothing to signal at init.
        let p = params_with_methods(&[("912", 7)], "999");
        assert_eq!(p.init_hktan_version().unwrap(), None);
    }

    #[test]
    fn unbuildable_tan_methods_are_not_offered_or_selected() {
        let v8 = "HITANS:53:8:4+1+1+1+N:N:0:941:2:TESTAPP:Decoupled::Neue App:::Freigabe:2048:N:2:N:0:0:N:N:00:2:N:1:60:4:2:J:J'";
        let v7 = "HITANS:42:7:4+1+1+1+N:N:0:\
            931:2:TESTHHD:HHD:1.4:Kartenleser Test:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0::::::\
            932:2:TESTAPP:Decoupled::App-Freigabe Test:::Freigabe:2048:N:2:N:0:0:N:N:00:2:N:1:60:4:2:J:J:\
            933:2:TESTSMS:mobileTAN::SMS Test:6:1:TAN:6:N:1:N:0:0:N:N:00:2:N:1:::::'";
        let wire = format!("HIBPA:1:3:4+1+280:1+Bank+1+1+300'{v7}{v8}");
        let mut p = BankParams::new();
        p.set_bpd(crate::codec::wire::tokenize(wire.as_bytes()).unwrap());
        let offered: Vec<&str> = p
            .tan_methods
            .iter()
            .map(|m| m.security_function.as_str())
            .collect();
        assert_eq!(offered, ["931", "932", "933"], "941 is HITANS 8");

        // The bank allows 941, but we cannot build its HKTAN: it is skipped
        // like an unknown method, also as a stored preference.
        p.allowed_security_functions =
            vec![SecurityFunction::new("941"), SecurityFunction::new("932")];
        p.preferred_security_function = Some(SecurityFunction::new("941"));
        p.select_security_function();
        assert_eq!(p.selected_security_function, SecurityFunction::new("932"));

        // The backstop still names the HITANS versions of an unbuildable method.
        p.selected_security_function = SecurityFunction::new("941");
        assert!(matches!(
            p.hktan_version(),
            Err(FinTSError::VersionMismatch(v)) if v.offered == [8]
        ));
    }

    /// Decoupled status with the HKTAN version the BPD offers: process S
    /// with HKTAN 7, process 2 with HKTAN 5 and 6 (PIN/TAN B.5.2; owner
    /// decision 2026-11-15; common client behaviour, S only from version 7).
    #[test]
    fn decoupled_poll_follows_the_hktan_version() {
        let poll = Segment::TanPollDecoupled {
            task_reference: TaskReference::new("TASK-1"),
            tan_medium: None,
        };
        let mut p = BankParams::new();
        let wire = |p: &BankParams| {
            let raw = poll.encode(p, 3).unwrap();
            String::from_utf8(crate::codec::wire::serialize_segment(&raw).unwrap()).unwrap()
        };
        for (version, expected) in [
            (7, "HKTAN:3:7+S++++TASK-1+N'"),
            (6, "HKTAN:3:6+2++++TASK-1+N'"),
            (5, "HKTAN:3:5+2++++TASK-1++N'"),
        ] {
            p.tan_methods = vec![TanMethod {
                security_function: SecurityFunction::new("902"),
                tan_process: TanProcess::TwoStep,
                name: "App".into(),
                needs_tan_medium: false,
                decoupled_max_polls: -1,
                wait_before_first_poll: 0,
                wait_before_next_poll: 0,
                is_decoupled: true,
                hktan_version: version,
                zka_name: None,
                decoupled_auto_polling: None,
                challenge_structured: false,
            }];
            p.selected_security_function = SecurityFunction::new("902");
            assert_eq!(wire(&p), expected);
        }
    }

    /// Feeds `data` to everything that reads bank answers. Used by the
    /// property tests below, which only check that nothing panics.
    fn read_everything(data: &[u8]) {
        use crate::segments::{read, response};
        let Ok(answer) = parse_response(data, 1) else {
            return;
        };
        let _ = (
            answer.check_errors(),
            answer.get_tan_challenge(),
            answer.touchdown(),
        );

        let mut params = BankParams::new();
        params.ingest_response(&answer, &mut SystemId::unassigned());
        params.select_security_function();
        let _ = params.camt052_format();

        for segment in &answer.segments {
            let _ = (read::balance(segment), read::tan_challenge(segment));
            let _ = (read::upd_account(segment), read::tan_methods(segment));
            let _ = (
                response::parse_hiupd(segment),
                response::parse_hitans(segment),
            );
        }

        let all = &answer.segments;
        let _ = (read::parse_hiwpd(all), read::parse_hiwdu(all));
        let _ = read::extract_mt940_data(all);
        let camt = read::extract_camt_data(all);
        for report in camt.booked.iter().chain(&camt.pending) {
            let _ = crate::camt::parse_camt052(report, crate::banks::BookingIdSource::None);
        }
    }

    /// Pieces that look like parts of real answers, so that random
    /// combinations reach deep into the readers.
    const ANSWER_PIECES: &[&str] = &[
        "HIRMG:2:2+",
        "HIRMS:3:2:4+",
        "HISAL:4:7:5+",
        "HITAN:5:7:6+",
        "HITANS:6:7:4+1+1+0+",
        "HIUPD:7:6:4+",
        "HICAZ:8:1:5+",
        "HICAZS:9:1:4+1+1+0+400:J:N:",
        "HIWPD:10:6:5+",
        "HIWDU:11:5:5+",
        "HNVSD:999:1+",
        "@4@",
        "@123456789012345678901@",
        "3040::mehr:TP",
        "DE14123456780000004711:TESTDEFFXXX:4711::280:12345678",
        "C:1,5:EUR:20260115",
        ":16R:GENL\r\n:35B:ISIN DE0000000001\r\n:16S:FIN\r\n",
    ];

    use proptest::strategy::Strategy as _;

    proptest::proptest! {
        #[test]
        fn readers_survive_arbitrary_bytes(
            bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..600)
        ) {
            read_everything(&bytes);
        }

        #[test]
        fn readers_survive_shuffled_answer_pieces(
            picks in proptest::collection::vec(
                proptest::prop_oneof![
                    proptest::sample::select(ANSWER_PIECES).prop_map(str::to_owned),
                    "[0-9A-Z:+?'@ ,]{0,10}",
                ],
                0..30,
            )
        ) {
            read_everything(picks.concat().as_bytes());
        }
    }

    /// Return code 3072 hands out a new user ID (Formals, return codes).
    /// The old UPD belongs to the old ID and is asked for again.
    #[test]
    fn new_user_id_from_3072_is_kept_and_the_upd_dropped() {
        let mut params = BankParams::new();
        let upd = b"HIUPA:4:4:3+kunde7+5+0'HIUPD:5:6:3+4711::280:12345678+DE14123456780000004711+kunde7++EUR+Erika Muster'";
        params.set_upd(crate::codec::wire::tokenize(upd).unwrap());
        assert_eq!(params.upd_version, 5);

        let answer = parse_response(
            b"HIRMG:2:2+0010::Nachricht entgegengenommen.'\
              HIRMS:3:2:3+3072::Neue Zugangsdaten:kunde8:kundenid8'",
            1,
        )
        .unwrap();
        // A warning, not an error.
        answer.check_errors().unwrap();
        params.ingest_response(&answer, &mut SystemId::new("SYS-0042"));
        assert_eq!(
            params.changed_user_id.as_ref().map(UserId::as_str),
            Some("kunde8")
        );
        assert_eq!(params.upd_version, 0);
        assert!(params.upd().is_none());
    }

    /// An answer that carries the BPD again (here: the same answer twice)
    /// replaces the stored BPD; nothing is listed twice.
    #[test]
    fn repeated_bpd_replaces_the_stored_one() {
        let answer = parse_response(
            b"HIRMG:2:2+0010::ok'\
              HIBPA:3:3:4+21+280:12345678+Testbank+1+1+300'\
              HITANS:4:7:4+1+1+1+J:N:0:902:2:TEST902:Decoupled:1.0:Test-App:::Freigabe:0:N:1:N:0:0:N:N:00:0:N:0:0:0:0:N:J'",
            1,
        )
        .unwrap();
        let mut params = params_with_bpd("");
        let mut system_id = SystemId::unassigned();
        for _ in 0..2 {
            params.ingest_response(&answer, &mut system_id);
        }
        assert_eq!(params.bpd_version, 21);
        let codes: Vec<_> = params
            .tan_methods
            .iter()
            .map(|m| m.security_function.as_str())
            .collect();
        assert_eq!(codes, ["902"]);
    }

    /// A bank wraps the reason for a rejected order in a generic global
    /// 9050; the segment code must win.
    #[test]
    fn segment_error_wins_over_generic_global_code() {
        let resp = parse_response(
            b"HIRMG:2:2+9050::Die Nachricht enthaelt Fehler'              HIRMS:3:2:3+0020::ok'HIRMS:4:2:4+9010::Verarbeitung nicht moeglich+9210::Keine gueltige Kontoverbindung'",
            1,
        )
        .unwrap();
        let err = resp.check_errors().unwrap_err();
        assert_eq!(
            err.to_string(),
            "Bank error 9210: Keine gueltige Kontoverbindung"
        );
        // Only generic codes: the segment one first.
        let resp = parse_response(
            b"HIRMG:2:2+9050::Teilweise fehlerhaft'HIRMS:3:2:3+9010::Abgelehnt'",
            1,
        )
        .unwrap();
        assert!(
            resp.check_errors()
                .unwrap_err()
                .to_string()
                .contains("9010")
        );
        // A wrong PIN anywhere still wins.
        let resp = parse_response(
            b"HIRMG:2:2+9050::Fehler'HIRMS:3:2:3+9210::Konto'HIRMS:4:2:1+9942::PIN falsch'",
            1,
        )
        .unwrap();
        assert_eq!(
            resp.check_errors().unwrap_err().kind(),
            Category::Credentials(crate::return_code::Credentials::PinWrong)
        );
    }

    /// Notices: warnings and unknown codes; not success, errors or the
    /// codes the protocol consumes (docs/return-codes-design.md section 2).
    #[test]
    fn notices_skip_success_errors_and_consumed_codes() {
        let answer = b"HIRMG:2:2+0010::ok+3050::veraltet+3060::Hinweise'\
            HIRMS:3:2:3+0030::TAN+3040::mehr:abc+3076::SCA+3920::Verfahren:912\
            +3955::decoupled+3956::offen+3091::VOP+3945::VOP+3072::neu:A:B\
            +3010::leer+3957::Push+3958::App+3999::bank+1234::note+9210::Konto'";
        let resp = parse_response(answer, 1).unwrap();
        let codes: Vec<String> = resp
            .notices(NoticeDialog::Job)
            .iter()
            .map(|n| n.code().unwrap().code.as_str().to_string())
            .collect();
        assert_eq!(
            codes,
            [
                "3050", "3060", "3072", "3010", "3957", "3958", "3999", "1234"
            ]
        );
        let none = parse_response(b"HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::ok'", 1).unwrap();
        assert!(none.notices(NoticeDialog::Job).is_empty());
    }

    /// A wrong typed TAN leaves the order for another TAN only when 9941
    /// is the one cause (docs/return-codes-design.md section 1): (answer,
    /// attempts left when it does).
    #[test]
    fn tan_rejection_only_where_the_order_stays() {
        let cases: &[(&[u8], Option<Option<u32>>)] = &[
            (b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9941::TAN ungueltig'", Some(None)),
            (
                b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9941::TAN ungueltig+9360::Sperre nach weiteren 2:2'",
                Some(Some(2)),
            ),
            // 9360 with no attempts left, or without P1 (locked).
            (
                b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9941::TAN ungueltig+9360::Sperre:0'",
                None,
            ),
            (b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9941::T+9360::gesperrt'", None),
            // The dialog ended, the order was deleted, the access locked.
            (
                b"HIRMG:2:2+9050::F+9800::Dialog abgebrochen'HIRMS:3:2:3+9941::T'",
                None,
            ),
            (
                b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9941::T+9210::Auftrag geloescht'",
                None,
            ),
            (b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9931::gesperrt'", None),
            // Timed out or used: not a wrong TAN.
            (b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9951::Zeit'", None),
            (b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9943::verbraucht'", None),
            // Success.
            (b"HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::ok'", None),
        ];
        for (answer, want) in cases {
            let resp = parse_response(answer, 1).unwrap();
            let got = resp.tan_rejection();
            let wire = String::from_utf8_lossy(answer);
            assert_eq!(got.as_ref().map(|(_, n)| *n), *want, "{wire}");
            if let Some((code, _)) = got {
                assert_eq!(code.code.as_str(), "9941", "{wire}");
            }
        }
    }

    /// The selection rule (docs/return-codes-design.md section 3) over
    /// multi-code answers: (answer, selected code).
    #[test]
    fn select_error_order() {
        let cases: &[(&[u8], Option<&str>)] = &[
            // Warnings and successes only.
            (b"HIRMG:2:2+3060::Hinweise'HIRMS:3:2:3+0020::ok'", None),
            // Rule 1: credentials, then system ID, wherever they are.
            (
                b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9210::Konto'HIRMS:4:2:1+9942::PIN'",
                Some("9942"),
            ),
            (
                b"HIRMG:2:2+9391::Kundensystem-ID'HIRMS:3:2:3+9931::gesperrt'",
                Some("9931"),
            ),
            (
                b"HIRMG:2:2+9391::Kundensystem-ID'HIRMS:3:2:3+9210::Konto'",
                Some("9391"),
            ),
            // Live sets (2026-11-17), synthetic texts. A wrong BLZ for the
            // user: 9210 names the cause, 9340 is its consequence.
            (
                b"HIRMG:2:2+9050::Nachricht enthaelt Fehler+9800::Dialog abgebrochen\
                  +9210::Bankleitzahl passt nicht zum Benutzer+9340::Ungueltige Signatur'",
                Some("9210"),
            ),
            // A wrong PIN: 9942 next to 9800.
            (
                b"HIRMG:2:2+9800::Dialog abgebrochen+9942::PIN ungueltig'",
                Some("9942"),
            ),
            // 9340 loses to a named credentials cause and to any specific
            // error, segment or message scope; it wins over generic codes.
            (
                b"HIRMG:2:2+9340::Signatur'HIRMS:3:2:1+9942::PIN'",
                Some("9942"),
            ),
            (
                b"HIRMG:2:2+9340::Signatur'HIRMS:3:2:3+9380::nicht berechtigt'",
                Some("9380"),
            ),
            (
                b"HIRMG:2:2+9050::F+9800::Dialog abgebrochen'HIRMS:3:2:1+9340::Signatur'",
                Some("9340"),
            ),
            // Rule 2: the first specific segment error; 9800 is generic.
            (
                b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9010::V+9210::Konto'",
                Some("9210"),
            ),
            (
                b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9800::Dialog abgebrochen+9210::Konto'",
                Some("9210"),
            ),
            (
                b"HIRMG:2:2+9800::Dialog abgebrochen'HIRMS:3:2:3+9210::Konto'",
                Some("9210"),
            ),
            (
                b"HIRMG:2:2+9000::Status indifferent'HIRMS:3:2:3+9210::Konto'",
                Some("9210"),
            ),
            // Rule 3: else the first specific message error.
            (
                b"HIRMG:2:2+9800::Dialog abgebrochen+9075::SCA'HIRMS:3:2:3+9010::V'",
                Some("9075"),
            ),
            (
                b"HIRMG:2:2+9050::F+9000::Status indifferent:HKCAZ'",
                Some("9000"),
            ),
            // Rule 5: only generic codes; segment scope first.
            (
                b"HIRMG:2:2+9050::F'HIRMS:3:2:3+9010::Abgelehnt'",
                Some("9010"),
            ),
            (
                b"HIRMG:2:2+9800::Dialog abgebrochen'HIRMS:3:2:3+9050::F'",
                Some("9050"),
            ),
            (b"HIRMG:2:2+9050::F+9800::Dialog abgebrochen'", Some("9050")),
        ];
        for (answer, want) in cases {
            let resp = parse_response(answer, 1).unwrap();
            let got = resp.select_error();
            let wire = String::from_utf8_lossy(answer);
            assert_eq!(
                got.as_ref().map(|e| e.code().code.as_str()),
                *want,
                "{wire}"
            );
            if let Some(e) = got {
                // The error keeps every code of the response.
                assert_eq!(e.codes().len(), resp.all_codes().count(), "{wire}");
                assert!(resp.check_errors().is_err());
            }
        }
    }

    /// 9000: the outcome is unknown; P1 names the order to ask with.
    #[test]
    fn outcome_unknown_names_the_order_to_check() {
        let resp = parse_response(b"HIRMG:2:2+9000::Status indifferent:HKCAZ'", 1).unwrap();
        let err = resp.check_errors().unwrap_err();
        assert_eq!(err.kind(), Category::OutcomeUnknown);
        assert!(!err.is_retryable());
        assert_eq!(
            err.bank_error().map(|e| e.code().kind()),
            Some(Kind::OutcomeUnknown {
                query: Some(SegmentType::new("HKCAZ"))
            })
        );
    }

    /// A HIRMS code names the order it refers to, read from the sent
    /// message's segment numbers (also inside HNVSD); the error keeps the
    /// whole code line.
    #[test]
    fn segment_codes_name_their_order() {
        let request = b"HNHBK:1:3+000000000100+300+0+1'HNVSK:998:3+PIN:2'\
            HNVSD:999:1+@55@HNSHK:2:4+PIN:2'HKSAL:3:7+DE00:BIC+N'HKTAN:4:7+4+HKSAL''\
            HNHBS:5:1+1'";
        let answer = b"HIRMG:2:2+9050::Fehler'HIRMS:3:2:3+9210:2:Konto unbekannt:P1'\
            HIRMS:4:2:4+0020::ok'HIRMS:5:2:7+3920::Verfahren:912'";
        let resp = parse_answer(answer, 1, request).unwrap();
        let orders: Vec<_> = resp
            .segment_codes
            .iter()
            .map(|c| match &c.scope {
                Scope::Segment { number, order } => (*number, order.clone()),
                Scope::Message => panic!("HIRMS has segment scope"),
            })
            .collect();
        assert_eq!(
            orders,
            [
                (3, Some(SegmentType::new("HKSAL"))),
                (4, Some(SegmentType::new("HKTAN"))),
                (7, None),
            ]
        );
        assert_eq!(resp.global_codes[0].scope, Scope::Message);
        match resp.check_errors() {
            Err(FinTSError::Bank(e)) => {
                let code = e.code();
                assert_eq!(code.code.as_str(), "9210");
                assert_eq!(code.reference_element.as_deref(), Some("2"));
                assert_eq!(code.parameters, ["P1"]);
                assert!(matches!(code.scope, Scope::Segment { number: 3, .. }));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A notice carries its dialog and the segment code its HIRMS
    /// reference names (segment number 3 is the HKSAL, 4 the HKTAN); a
    /// HIRMG notice is message-level, a reference outside the request
    /// shows only the number.
    #[test]
    fn notices_name_dialog_and_order() {
        let request = b"HNHBK:1:3+000000000100+300+0+1'HNVSK:998:3+PIN:2'\
            HNVSD:999:1+@55@HNSHK:2:4+PIN:2'HKSAL:3:7+DE00:BIC+N'HKTAN:4:7+4+HKSAL''\
            HNHBS:5:1+1'";
        let answer = b"HIRMG:2:2+3060::Hinweise'HIRMS:3:2:3+3010::leer'\
            HIRMS:4:2:4+3072::neu:A:B'HIRMS:5:2:7+3999::bank'";
        let resp = parse_answer(answer, 1, request).unwrap();
        let notices = resp.notices(NoticeDialog::Sync);
        let shown: Vec<String> = notices.iter().map(|n| n.to_string()).collect();
        assert_eq!(
            shown,
            [
                "3060 [sync, HIRMG]: Hinweise",
                "3010 [sync, HKSAL]: leer",
                "3072 [sync, HKTAN]: neu",
                "3999 [sync, HIRMS #7]: bank",
            ]
        );
        assert!(notices.iter().all(|n| n.dialog() == NoticeDialog::Sync));
        assert_eq!(
            resp.notices(NoticeDialog::Job)[0].dialog(),
            NoticeDialog::Job
        );
    }

    #[test]
    fn utf8_text_in_latin1_answer_is_a_client_notice() {
        // "für" as UTF-8 bytes in a Latin-1 answer; the real "ü" (0xFC) is fine.
        let bad = parse_response(b"HIRMG:2:2+3060::Hinweis f\xC3\xBCr Sie'", 1).unwrap();
        let notices = bad.notices(NoticeDialog::Job);
        assert_eq!(notices.len(), 2);
        assert_eq!(
            notices[1].client_warning(),
            Some(ClientWarning::Utf8TextInLatin1)
        );
        assert!(notices[1].code().is_none());
        assert!(notices[1].to_string().starts_with("client [job]:"));
        assert_eq!(bad.global_codes[0].text, "Hinweis f\u{c3}\u{bc}r Sie");
        let good = parse_response(b"HIRMG:2:2+3060::Hinweis f\xFCr Sie'", 1).unwrap();
        assert_eq!(good.notices(NoticeDialog::Job).len(), 1);
    }

    /// Every 9xxx code is an error, also one no table lists; the message
    /// keeps the bank's code and text.
    #[test]
    fn unlisted_error_code_fails_with_the_banks_words() {
        let answer = parse_response(b"HIRMG:2:2+9876::Systemfehler, bitte spaeter erneut'", 1);
        let error = answer.unwrap().check_errors().unwrap_err();
        let shown = error.to_string();
        assert_eq!(shown, "Bank error 9876: Systemfehler, bitte spaeter erneut");
    }

    /// A code the codec cannot read (not four digits, Formals C.8.1) is not
    /// taken for a success.
    #[test]
    fn unreadable_response_codes_are_an_error() {
        let wire = "HIRMG:2:2+0010::ok'HIRMS:3:2:3+00X0::ok'";
        let resp = parse_response(wire.as_bytes(), 1).unwrap();
        assert_eq!(resp.segment_codes.len(), 0);
        assert_eq!(resp.segment_errors().len(), 1);
        let err = resp.check_errors().unwrap_err();
        assert!(matches!(err, FinTSError::InvalidResponse(_)), "{err}");
    }

    /// Bank text is ISO 8859-1 (Formals B.1): 0xE4 is 'ä'.
    #[test]
    fn response_text_is_latin1() {
        let resp = parse_response(b"HIRMG:2:2+0010::Auftr\xe4ge ok'", 1).unwrap();
        assert_eq!(resp.global_codes[0].text, "Aufträge ok");
    }

    /// A depot listed in the UPD without IBAN reaches every order that
    /// addresses accounts nationally, and gets a clear error from the ones
    /// that need an IBAN.
    #[test]
    fn national_only_account_through_all_orders() {
        let depot = Account::national(
            "0123456789",
            Some("01"),
            &Blz::new("12030000"),
            "BYLADEM1001",
        )
        .unwrap();
        assert_eq!(depot.account_number(), Some("0123456789"));
        assert_eq!(depot.subaccount(), Some("01"));

        let p = params_with_bpd(
            "HISALS:1:6:4+1+1+1'HIKAZS:2:6:4+1+1+1'HIWPDS:3:6:4+1+1+1'HIWDUS:4:5:4+1+1+0'HICAZS:7:1:4+1+1+1'",
        );
        let date = Date::new(2026, 1, 1).unwrap();
        let national = [
            Segment::Balance {
                account: depot.clone(),
            },
            Segment::Transactions {
                account: depot.clone(),
                start_date: date,
                end_date: date,
                touchdown: None,
            },
            Segment::Holdings {
                account: depot.clone(),
                currency: None,
                touchdown: None,
            },
            Segment::DepotTransactions {
                account: depot.clone(),
                start_date: None,
                end_date: None,
                touchdown: None,
            },
        ];
        for seg in &national {
            let degs = seg.encode(&p, 3).unwrap().degs;
            assert_eq!(degs[1].get_str(0), "0123456789", "{seg:?}");
            assert_eq!(degs[1].get_str(3), "12030000", "{seg:?}");
        }
        let camt = Segment::CamtTransactions {
            account: depot,
            start_date: date,
            end_date: date,
            touchdown: None,
        };
        let err = camt.encode(&p, 3).unwrap_err();
        assert!(
            matches!(
                err,
                FinTSError::AccountNotAddressable {
                    version: 1,
                    reason: crate::error::AddressingReason::NoIbanBic,
                    ..
                }
            ),
            "{err}"
        );
        assert_eq!(
            err.to_string(),
            "HKCAZ v1 cannot address this account: it needs an IBAN and BIC, which the account lacks"
        );
        assert!(
            Account::national("1", None, &Blz::new("1203000"), "X").is_err(),
            "BLZ has 8 digits"
        );
    }

    #[test]
    fn german_iban_gives_the_national_details() {
        let german = Account::new(" de14 1234 5678 0000 0047 11 ", "TESTDEFFXXX").unwrap();
        assert_eq!(german.iban(), "DE14123456780000004711");
        assert_eq!(german.blz(), Some("12345678"));
        assert_eq!(german.account_number(), Some("4711"));
        assert_eq!(german.subaccount(), None);

        let zeros = Account::new("DE00123456780000000000", "TESTDEFFXXX").unwrap();
        assert_eq!(zeros.account_number(), Some("0"));
    }

    #[test]
    fn other_ibans_need_explicit_national_details() {
        let other = Account::new("XX00000000000000004711", "TESTXXXXXXX").unwrap();
        assert_eq!((other.blz(), other.account_number()), (None, None));

        let with = other.with_national("4711", Some(""), &Blz::new("12345678"));
        assert_eq!(with.account_number(), Some("4711"));
        assert_eq!(with.subaccount(), None);
        assert_eq!(with.blz(), Some("12345678"));
        assert_eq!(with.iban(), "XX00000000000000004711");
    }

    #[test]
    fn accounts_without_identifier_are_refused() {
        assert!(Account::new(" ", "TESTDEFFXXX").is_err());
        assert!(Account::new("DE14123456780000004711", "").is_err());
        let code = Blz::new("12345678");
        assert!(Account::national("", None, &code, "TESTDEFFXXX").is_err());
        assert!(Account::national("4711", None, &Blz::new("1234"), "TESTDEFFXXX").is_err());
        let depot = Account::national(" 4711 ", Some("01"), &code, "TESTDEFFXXX").unwrap();
        assert_eq!(depot.account_number(), Some("4711"));
        assert_eq!(depot.subaccount(), Some("01"));
        assert_eq!(depot.iban(), "");
    }
}
