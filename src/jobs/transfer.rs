//! The transfer job (docs/payments-design.md sections 2 and 3): HKCCS or
//! HKIPZ with the payee check (HKVPP, HKVPA) and the TAN.
//!
//! Message 1 is `HKVPP + order + HKTAN:4` (or `order + HKTAN:4` when the
//! BPD asks for no payee check), and its answer is classified by the
//! table of design section 3.2: a rejection fails the job, an unfinished
//! check pauses on [`Step::VopPending`], a check that is not a clean match
//! pauses on [`Step::NeedVopConfirmation`], a clean match goes on to the
//! TAN (or, after 3945, to the execution order `HKVPA + order + HKTAN:4`).
//! The pain.001 bytes are built once and sent unchanged in every message
//! that carries the order. The library never sends a payment twice.

use jiff::civil::Time;

use super::*;
use crate::codec::defs::{HiIpz1, HiIpz2, HiVpp1};
use crate::codec::{RawSegment, decode_segment};
use crate::formats::pain001::{self, Pain001Descriptor, Pain001Version, PainIds};
use crate::protocol::{FormatLists, VersionMismatch, VopDelivery};

mod input;
mod vop;
pub use input::{
    CheckedTransfer, Instant, InvalidReason, InvalidTransfer, Transfer, TransferField,
};
pub use vop::{PayeeOutcome, PayeeResult, VopResult};

// ═══════════════════════════════════════════════════════════════════════════
// Public values
// ═══════════════════════════════════════════════════════════════════════════

/// The value of [`transfer`]: how the bank took the payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferReceipt {
    pub sent_as: SentAs,
    pub status: TransferStatus,
    /// The day the order went out (the client's local date, or the one
    /// pinned with [`BankClient::with_now`]). [`instant_status`] reads
    /// the statement from this day when the bank offers no HKIPS.
    pub sent_on: Date,
    /// pain.001 `MsgId`, generated.
    pub message_id: String,
    /// `EndToEndId` as sent (the generated one when the transfer gave none).
    pub end_to_end_id: String,
    /// HIIPZ "Auftragsidentifikation" (instant payments).
    pub order_id: Option<String>,
    /// 3070 P1: the bank's reference for the order.
    pub bank_reference: Option<String>,
    /// What the bank reported for the payee; `None` when no HKVPP was sent.
    pub payee_check: Option<VopResult>,
}

/// The order the payment went out as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentAs {
    /// HKCCS.
    Standard,
    /// HKIPZ.
    Instant,
    /// HKIPZ that the bank runs as a standard transfer (3270, allowed by
    /// the conversion flag of HKIPZ 2).
    InstantConverted,
}

/// Where the payment stands after the bank's last answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    /// Taken for processing (0010 or 3076 without 0020).
    Accepted,
    /// Executed (0020).
    Executed,
    /// Instant payment whose outcome the bank does not know yet (3045):
    /// ask with [`instant_status`], not before `not_before` (HIIPSS
    /// "Mindestwartezeit"; `None` when the bank gives none, also when it
    /// offers no HKIPS and the status comes from the statement).
    /// `order_id` is HIIPZ "Auftragsidentifikation", `None` when the bank
    /// sent no HIIPZ.
    InstantPending {
        order_id: Option<String>,
        not_before: Option<Timestamp>,
    },
}

/// The payee check asks the user (anything but a clean match). Answer with
/// [`Dialog::confirm_vop`] or [`Dialog::cancel`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VopConfirmation {
    /// The result; no payees when the bank reported none (3095).
    pub result: VopResult,
    /// HIVPP "Aufklärungstext": the bank requires it to be shown.
    pub explanation: Option<String>,
    /// HIVPPS "Aufklärungstext strukturiert": the text may hold `<br> <p>
    /// <b> <i> <u> <ul> <ol> <li> <a href>`.
    pub explanation_markup: bool,
    /// HIVPP "VOP-ID gültig bis", bank local time.
    pub valid_until: Option<DateTime>,
}

/// The payee check is not finished. Call [`Dialog::poll`] at or after
/// `not_before` (`None`: the bank gave no wait; use your own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VopWait {
    pub not_before: Option<Timestamp>,
    /// HKVPP polls sent so far (message 1 not counted).
    pub polls_done: u32,
}

// ═══════════════════════════════════════════════════════════════════════════
// The job as data
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PaymentKind {
    /// HKCCS.
    Ccs,
    /// HKIPZ at `version`; `convert` is HKIPZ 2's conversion flag.
    Ipz { version: u16, convert: Option<bool> },
}

/// A transfer in flight: the pain.001 bytes as built, and the payee check
/// so far. Part of a paused dialog (payments design 3.8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PaymentJob {
    from: Account,
    kind: PaymentKind,
    /// The pain.001 descriptor as the bank announced it.
    descriptor: String,
    /// The pain.001 message, verbatim: every order segment carries these
    /// bytes (VoP C.10.7.1.2).
    #[serde(with = "utf8_bytes")]
    pain: Vec<u8>,
    message_id: String,
    end_to_end_id: String,
    payee_iban: Iban,
    payee_name: String,
    vop: Option<VopState>,
    /// The next TAN message carries HKVPA (a TAN without 3945 and 3091).
    hkvpa_with_tan: bool,
}

/// The payee check of a [`PaymentJob`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct VopState {
    /// The pain.002 descriptors HKVPP asks for.
    descriptors: Vec<String>,
    /// HIVPPS "Art der Lieferung" `S`: pain.002 parts are merged.
    stepwise: bool,
    explanation_markup: bool,
    vop_id: Option<Vec<u8>>,
    valid_until: Option<DateTime>,
    polling_id: Option<Vec<u8>>,
    touchdown: Option<String>,
    polls_done: u32,
    /// pain.002 documents received, in order.
    parts: Vec<Vec<u8>>,
    result: Option<VopResult>,
    explanation: Option<String>,
    /// 3090 in the final answer.
    deviation: bool,
    /// 3091 in the final answer.
    not_required: bool,
    /// The challenge of message 1 while the user decides on the result:
    /// [`Dialog::confirm_vop`] returns it without sending when every payee
    /// matched (the user was asked for 3090 or an explanation text).
    live_tan: Option<(TaskReference, TanChallenge)>,
}

/// pain.001 is UTF-8 (the builder writes it): stored as a string.
mod utf8_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let text = std::str::from_utf8(bytes).map_err(serde::ser::Error::custom)?;
        s.serialize_str(text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Ok(String::deserialize(d)?.into_bytes())
    }
}

/// Which message the answer belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Message 1: `[HKVPP +] order + HKTAN:4`.
    First,
    /// HKVPP alone.
    Poll,
    /// `HKVPA + order + HKTAN:4`.
    Execution,
}

/// The IDs of a payment whose order went out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PaymentIds {
    end_to_end_id: String,
    message_id: String,
}

impl PaymentIds {
    /// [`FinTSError::PaymentOutcomeUnknown`] for this payment.
    pub(super) fn unknown(&self) -> FinTSError {
        FinTSError::PaymentOutcomeUnknown {
            end_to_end_id: self.end_to_end_id.clone(),
            message_id: self.message_id.clone(),
        }
    }

    /// An error after a message that carried the order or its TAN (design
    /// D2): a class 9 rejection by the bank is definite, every other error
    /// (transport, unreadable answer, unexpected answer) leaves the bank's
    /// decision unknown.
    pub(super) fn after_order(&self, e: FinTSError) -> FinTSError {
        match e {
            FinTSError::Bank(_) | FinTSError::PaymentOutcomeUnknown { .. } => e,
            e => {
                warn!("[FinTS] Payment outcome unknown: {e}");
                self.unknown()
            }
        }
    }
}

/// What follows an answer.
enum Next {
    Outcome(Outcome),
    /// Send the execution order (design 3.3).
    Execute(Box<PaymentJob>, Vec<Notice>),
}

impl PaymentJob {
    /// Check the transfer, choose the order and the pain.001 version,
    /// build the pain.001 with `ids` and `created` (`CreDtTm`). Sends
    /// nothing.
    fn prepare(
        params: &BankParams,
        t: &Transfer,
        ids: &PainIds,
        created: DateTime,
    ) -> Result<Self> {
        let holder = find_upd_account(params, &t.from)
            .and_then(|a| a.owner.iter().find(|o| !o.trim().is_empty()))
            .map(String::as_str);
        let checked = t.check(holder)?;
        let (kind, descriptor) = choose_order(params, t.instant)?;
        let code = kind.code();
        let vop = match params.bpd().and_then(|b| b.vop()) {
            Some(v) if v.requires(code) => {
                params.order_version(Order::VopCheck)?;
                Some(VopState {
                    descriptors: vop::choose_report_formats(v),
                    // An unknown delivery code reads like complete delivery.
                    stepwise: v.delivery == VopDelivery::Stepwise,
                    explanation_markup: v.explanation_structured,
                    ..VopState::default()
                })
            }
            _ => None,
        };
        let instant = matches!(kind, PaymentKind::Ipz { .. });
        let pain = pain001::build(&checked.pain_input(instant), &descriptor, ids, created);
        info!(
            "[FinTS] Transfer as {code} ({}), payee check: {}",
            pain.descriptor,
            if vop.is_some() { "yes" } else { "no" }
        );
        Ok(Self {
            from: t.from.clone(),
            kind,
            descriptor: pain.descriptor,
            pain: pain.xml,
            message_id: pain.message_id,
            end_to_end_id: checked.end_to_end_id().to_string(),
            payee_iban: checked.payee_iban().clone(),
            payee_name: checked.payee_name().to_string(),
            vop,
            hkvpa_with_tan: false,
        })
    }

    fn code(&self) -> &'static str {
        self.kind.code()
    }

    fn order_segment(&self) -> Segment {
        match self.kind {
            PaymentKind::Ccs => Segment::CreditTransfer {
                account: self.from.clone(),
                descriptor: self.descriptor.clone(),
                pain: self.pain.clone(),
            },
            PaymentKind::Ipz { version, convert } => Segment::InstantTransfer {
                account: self.from.clone(),
                version,
                descriptor: self.descriptor.clone(),
                pain: self.pain.clone(),
                convert,
            },
        }
    }

    /// HKVPA for the next TAN message, when the bank expects it.
    pub(super) fn tan_extra(&self) -> Vec<Segment> {
        match self.vop.as_ref().and_then(|v| v.vop_id.clone()) {
            Some(vop_id) if self.hkvpa_with_tan => vec![Segment::VopExecute { vop_id }],
            _ => Vec::new(),
        }
    }

    /// HKVPA went out with a TAN message; later polls go without it.
    pub(super) fn tan_extra_sent(&mut self) {
        self.hkvpa_with_tan = false;
    }

    /// The IDs the order carries, for [`FinTSError::PaymentOutcomeUnknown`]
    /// and [`Cancelled::OrderSubmitted`].
    pub(super) fn ids(&self) -> PaymentIds {
        PaymentIds {
            end_to_end_id: self.end_to_end_id.clone(),
            message_id: self.message_id.clone(),
        }
    }

    /// What a cancel on `wait` leaves behind (design D1). At a TAN the
    /// order is at the bank: a late approval in the app may execute it.
    /// At the payee check nothing is released: the bank executes only
    /// after the client's execution order (HKVPA, VoP C.10.7: "zeigt der
    /// Kunde mit der Einreichung des Ausführungsauftrags (HKVPA) an, dass
    /// er der Ausführung … zustimmt"), and an unfinished check has no
    /// VOP-ID to release and no challenge yet. The exception is a live
    /// decoupled challenge of message 1 that the bank releases without
    /// HKVPA (3091): an approval in the app may execute it.
    pub(super) fn cancelled(&self, wait: &Wait) -> Cancelled {
        let at_tan = match wait {
            Wait::Tan(_) => true,
            Wait::VopConfirmation(_) => {
                !self.hkvpa_with_tan
                    && self.vop.as_ref().is_some_and(|v| {
                        v.live_tan
                            .as_ref()
                            .is_some_and(|(_, c)| matches!(c.kind, TanKind::Decoupled { .. }))
                    })
            }
            Wait::VopPoll(_) => false,
        };
        if !at_tan {
            return Cancelled::NothingSent;
        }
        warn!("[FinTS] Transfer cancelled after its order went out: the bank may still execute it");
        let PaymentIds {
            end_to_end_id,
            message_id,
        } = self.ids();
        Cancelled::OrderSubmitted {
            end_to_end_id,
            message_id,
        }
    }

    fn payee_check(&self) -> Option<VopResult> {
        self.vop.as_ref().and_then(|v| v.result.clone())
    }

    /// Every payee matched, the bank sent no 3090 and no explanation
    /// (design 3.2). An empty result (3095, or a final HIVPP without one)
    /// and a transfer without a payee check count as clean.
    fn clean_match(&self) -> bool {
        let Some(v) = &self.vop else { return true };
        !v.deviation && v.explanation.is_none() && self.all_match()
    }

    /// No payee is reported other than `Match` (an empty result included).
    fn all_match(&self) -> bool {
        self.vop
            .as_ref()
            .and_then(|v| v.result.as_ref())
            .is_none_or(VopResult::all_match)
    }

    fn confirmation(&self) -> VopConfirmation {
        let v = self.vop.clone().unwrap_or_default();
        VopConfirmation {
            result: v.result.unwrap_or_else(VopResult::empty),
            explanation: v.explanation,
            explanation_markup: v.explanation_markup,
            valid_until: v.valid_until,
        }
    }

    /// The result of the final HIVPP ([`vop::read_result`]).
    fn read_result(&self, hivpp: &HiVpp1) -> Result<Option<VopResult>> {
        let Some(v) = &self.vop else { return Ok(None) };
        let sent = vop::Sent {
            message_id: &self.message_id,
            end_to_end_id: &self.end_to_end_id,
            iban: &self.payee_iban,
            name: &self.payee_name,
        };
        vop::read_result(hivpp, &v.parts, v.stepwise, &sent)
    }

    /// The receipt from the answer that ended the job.
    pub(super) fn receipt(
        &self,
        response: &Response,
        params: &BankParams,
        sent_on: Date,
    ) -> TransferReceipt {
        let order_id = read_hiipz(response);
        let sent_as = match self.kind {
            PaymentKind::Ccs => SentAs::Standard,
            PaymentKind::Ipz { .. } if response.has(Kind::ConvertedToStandard) => {
                SentAs::InstantConverted
            }
            PaymentKind::Ipz { .. } => SentAs::Instant,
        };
        let status = if response.has(Kind::InstantStatusQuery) {
            let min_wait = params
                .bpd()
                .and_then(|b| b.instant())
                .and_then(|i| i.status)
                .map(|s| Duration::from_secs(u64::from(s.min_wait)));
            TransferStatus::InstantPending {
                order_id: order_id.clone(),
                not_before: not_before_after(Timestamp::now(), min_wait),
            }
        } else if response
            .segment_codes
            .iter()
            .any(|c| c.kind() == Kind::OrderExecuted)
        {
            TransferStatus::Executed
        } else {
            TransferStatus::Accepted
        };
        let bank_reference = response.all_codes().find_map(|c| match c.kind() {
            Kind::ProcessingReference { reference } => reference,
            _ => None,
        });
        TransferReceipt {
            sent_as,
            status,
            sent_on,
            message_id: self.message_id.clone(),
            end_to_end_id: self.end_to_end_id.clone(),
            order_id,
            bank_reference,
            payee_check: self.payee_check(),
        }
    }
}

impl PaymentKind {
    fn code(self) -> &'static str {
        match self {
            Self::Ccs => "HKCCS",
            Self::Ipz { .. } => "HKIPZ",
        }
    }
}

/// HKCCS or HKIPZ and the pain.001 version, before anything is sent
/// (design section 2, "Instant choice").
fn choose_order(params: &BankParams, instant: Instant) -> Result<(PaymentKind, Pain001Descriptor)> {
    let standard = || -> Result<(PaymentKind, Pain001Descriptor)> {
        params.order_version(Order::Transfer)?;
        Ok((PaymentKind::Ccs, descriptor_for(params, Order::Transfer)?))
    };
    let instant_order = || -> Result<(PaymentKind, Pain001Descriptor)> {
        let version = params.order_version(Order::InstantTransfer)?;
        let descriptor = descriptor_for(params, Order::InstantTransfer)?;
        let allowed = params
            .bpd()
            .and_then(|b| b.instant())
            .is_some_and(|i| i.payment.conversion_allowed);
        let convert = (version >= 2 && allowed).then_some(true);
        Ok((PaymentKind::Ipz { version, convert }, descriptor))
    };
    match instant {
        Instant::No => standard(),
        Instant::Require => instant_order(),
        Instant::Prefer => match instant_order() {
            Err(FinTSError::VersionMismatch(why)) => {
                info!("[FinTS] No instant payment ({why}); sending HKCCS");
                standard()
            }
            other => other,
        },
    }
}

/// The pain.001 version for `order`, or a [`VersionMismatch`] on code
/// `pain.001` with both lists. HKIPZ needs pain.001.001.09 (`INST`).
pub(super) fn descriptor_for(params: &BankParams, order: Order) -> Result<Pain001Descriptor> {
    let offered = params.bpd().map_or(&[][..], |b| b.sepa_formats(order));
    let instant = matches!(order, Order::InstantTransfer | Order::InstantStatus);
    pain001::choose_descriptor(offered, instant).ok_or_else(|| {
        FinTSError::VersionMismatch(VersionMismatch {
            code: SegmentType::new("pain.001"),
            offered: Vec::new(),
            implemented: Vec::new(),
            formats: Some(Box::new(FormatLists {
                offered: offered.to_vec(),
                implemented: Pain001Version::ALL
                    .iter()
                    .filter(|v| !instant || **v != Pain001Version::V001_003_03)
                    .map(|v| v.name().to_string())
                    .collect(),
            })),
        })
    })
}

fn read_hivpp(response: &Response) -> Result<Option<HiVpp1>> {
    response
        .find_segment("HIVPP")
        .map(|raw| {
            decode_segment::<HiVpp1>(raw)
                .map(|(_, v)| v)
                .map_err(FinTSError::SegmentUnreadable)
        })
        .transpose()
}

/// HIIPZ "Auftragsidentifikation"; an unreadable HIIPZ is logged, never
/// an error (the payment is through).
fn read_hiipz(response: &Response) -> Option<String> {
    let raw: &RawSegment = response.find_segment("HIIPZ")?;
    let id = decode_segment::<HiIpz2>(raw)
        .map(|(_, v)| v.order_id)
        .or_else(|_| decode_segment::<HiIpz1>(raw).map(|(_, v)| v.order_id));
    match id {
        Ok(id) => Some(id.as_str().to_string()),
        Err(e) => {
            warn!("[FinTS] HIIPZ unreadable: {e}");
            None
        }
    }
}

fn local_time(t: &crate::codec::defs::Tsp1) -> DateTime {
    t.date
        .date()
        .to_datetime(t.time.map_or(Time::MIN, |t| t.time()))
}

// ═══════════════════════════════════════════════════════════════════════════
// The job
// ═══════════════════════════════════════════════════════════════════════════

/// Send `transfer` (payments design sections 2 and 3).
///
/// Checks first ([`Transfer::check`]; the debtor name falls back to the
/// UPD holder) and chooses the order before anything is sent:
/// [`Instant::No`] sends HKCCS, [`Instant::Require`] HKIPZ or
/// [`FinTSError::VersionMismatch`], [`Instant::Prefer`] HKIPZ when the bank
/// offers it with pain.001.001.09 (instant needs `INST`), else HKCCS. No
/// pain.001 version in common is a `VersionMismatch` on code `pain.001`. When the BPD
/// (HIVPPS) asks for a payee check on the order, HKVPP goes with it.
///
/// The step is [`Step::NeedTan`] (the payee matched or no check was
/// needed; the challenge carries the result), [`Step::NeedVopConfirmation`]
/// (show the result and ask the user), [`Step::VopPending`] (the bank is
/// still checking; poll later) or `Done` when the bank waived the TAN.
/// A transfer ends in one of three ways: a receipt, a definite rejection
/// (a class 9 return code from the bank, or an error before anything was
/// sent), or [`FinTSError::PaymentOutcomeUnknown`]. Once a message carried
/// the order or its TAN, every other error (transport, unreadable HITAN,
/// HIVPP or pain.002, an answer that fits no case) is an unknown outcome,
/// and so are a [`Dialog::poll`] past `max_polls` and a dropped future;
/// [`Dialog::cancel`] at the TAN says [`Cancelled::OrderSubmitted`]. Check
/// the account by `end_to_end_id` and `message_id` before trying again;
/// the library never resends a payment. A rejection leaves the dialog open.
///
/// Not cancel-safe (see [`Dialog`]): a dropped future loses the IDs of
/// the order in flight until the next call reports them.
pub async fn transfer(dialog: &mut Dialog, transfer: &Transfer) -> Result<Step<TransferReceipt>> {
    dialog.ready()?;
    let ids = PainIds::generate(Timestamp::now(), &mut rand::rng());
    // CreDtTm: the machine's local time (payments design 5.4).
    let created = dialog.inner.now();
    let job = PaymentJob::prepare(dialog.bank_params(), transfer, &ids, created)?;
    dialog.inner.take_notices();
    dialog
        .run_payment(job, Vec::new(), Stage::First)
        .await?
        .into_step()
}

impl Dialog {
    /// Confirm the payee check result of the paused transfer: sends the
    /// execution order (`HKVPA + order + HKTAN:4`) and returns its step,
    /// usually `NeedTan`. When every payee matched (the user was asked
    /// because of 3090 or an explanation text) and the bank's TAN of
    /// message 1 is still live, returns `NeedTan` for that challenge
    /// without sending. A payee that did not match always goes through the
    /// execution order (VoP E.8.1 point 4), even when message 1 had a
    /// challenge. On a job that waits for something else:
    /// [`FinTSError::Dialog`], and the job stays paused.
    ///
    /// Not cancel-safe (see [`Dialog`]).
    pub async fn confirm_vop<T: JobValue>(&mut self, resume: Resume<T>) -> Result<Step<T>> {
        let pending = self.pending_for(&resume)?;
        if !matches!(pending.wait, Wait::VopConfirmation(_)) {
            return Err(FinTSError::Dialog(
                "the job does not wait for a payee check confirmation".into(),
            ));
        }
        let PendingJob { job, notices, .. } = self.take_pending();
        let JobInput::Transfer(mut payment) = job else {
            return Err(FinTSError::Internal(
                "payee check confirmation without a transfer".into(),
            ));
        };
        let all_match = payment.all_match();
        let live_tan = payment.vop.as_mut().and_then(|v| v.live_tan.take());
        if let Some((task_reference, challenge)) = live_tan.filter(|_| all_match) {
            info!("[FinTS] Payee check confirmed; the TAN of message 1 is live");
            return self
                .pause_on(
                    JobInput::Transfer(payment),
                    Wait::Tan(challenge),
                    Ticket::Task(task_reference),
                    notices,
                )
                .into_step();
        }
        info!("[FinTS] Payee check confirmed; sending the execution order");
        self.run_payment(*payment, notices, Stage::Execution)
            .await?
            .into_step()
    }

    /// The next HKVPP poll of a paused transfer.
    pub(super) async fn vop_poll(&mut self, pending: PendingJob) -> Result<Outcome> {
        let PendingJob { job, notices, .. } = pending;
        let JobInput::Transfer(mut payment) = job else {
            return Err(FinTSError::Internal(
                "payee check poll without a transfer".into(),
            ));
        };
        if let Some(v) = payment.vop.as_mut() {
            v.polls_done += 1;
        }
        self.run_payment(*payment, notices, Stage::Poll).await
    }

    /// Send the payment messages from `stage` on until the job pauses or
    /// ends. After a message that carried the order, every error but a
    /// class 9 rejection is [`FinTSError::PaymentOutcomeUnknown`].
    async fn run_payment(
        &mut self,
        mut job: PaymentJob,
        mut notices: Vec<Notice>,
        mut stage: Stage,
    ) -> Result<Outcome> {
        loop {
            let ids = (stage != Stage::Poll).then(|| job.ids());
            let next = self
                .payment_round(job, notices, stage)
                .await
                .map_err(|e| match &ids {
                    Some(ids) => ids.after_order(e),
                    None => e,
                })?;
            match next {
                Next::Outcome(outcome) => return Ok(outcome),
                Next::Execute(j, n) => {
                    (job, notices, stage) = (*j, n, Stage::Execution);
                }
            }
        }
    }

    /// Send one payment message of `stage` and classify its answer.
    async fn payment_round(
        &mut self,
        job: PaymentJob,
        notices: Vec<Notice>,
        stage: Stage,
    ) -> Result<Next> {
        let response = match stage {
            Stage::Poll => {
                let v = job
                    .vop
                    .as_ref()
                    .ok_or_else(|| FinTSError::Internal("poll without a payee check".into()))?;
                // VoP C.10.7.1.1 HKVPP field 3: the polling ID only
                // next to a touchdown point.
                let poll = Segment::VopCheck {
                    descriptors: v.descriptors.clone(),
                    polling_id: v.touchdown.as_ref().and(v.polling_id.clone()),
                    touchdown: v.touchdown.clone(),
                };
                self.in_flight = Some(InFlight::Message);
                let sent = self.inner.send_vop_poll(poll).await;
                // Nothing was ordered by an HKVPP alone: plain errors.
                self.settle_message(false, sent)?
            }
            Stage::First | Stage::Execution => {
                let extra = match (stage, &job.vop) {
                    (Stage::First, Some(v)) => vec![Segment::VopCheck {
                        descriptors: v.descriptors.clone(),
                        polling_id: None,
                        touchdown: None,
                    }],
                    (Stage::First, None) => Vec::new(),
                    _ => {
                        let vop_id =
                            job.vop
                                .as_ref()
                                .and_then(|v| v.vop_id.clone())
                                .ok_or_else(|| {
                                    FinTSError::Internal("execution order without VOP-ID".into())
                                })?;
                        vec![Segment::VopExecute { vop_id }]
                    }
                };
                self.in_flight = Some(InFlight::Payment(job.ids()));
                let sent = self
                    .inner
                    .send_payment(&extra, job.order_segment(), job.code())
                    .await;
                self.settle_message(false, sent)?
            }
        };
        self.payment_answer(job, &response, notices, stage)
    }

    /// Classify an answer by the table of design section 3.2.
    fn payment_answer(
        &mut self,
        mut job: PaymentJob,
        response: &Response,
        mut notices: Vec<Notice>,
        stage: Stage,
    ) -> Result<Next> {
        // Every submission of a payee-checked transfer (HKVPP, HKVPA, each
        // status request) can carry the same 3060; one is enough.
        super::extend_notices(&mut notices, self.inner.take_notices());
        // After the order, `run_payment` turns every error but a class 9
        // rejection into an unknown outcome.
        response.check_errors()?;
        let checked = stage != Stage::Execution && job.vop.is_some();
        let challenge = response.order_tan_challenge()?;
        if checked {
            let hivpp = read_hivpp(response)?;
            if let (Some(report), Some(v)) = (
                hivpp.as_ref().and_then(|h| h.report.as_ref()),
                job.vop.as_mut(),
            ) {
                v.parts.push(report.0.clone());
            }
            // Final only with a VOP-ID (VoP C.10.7.1.1: "bis eine
            // abschließende Antwort (HIVPP mit VOP-ID) vorliegt"); an
            // answer with a challenge is never a poll. Polling ID and
            // touchdown point are both optional.
            let pending = response.has(Kind::VopInProgress)
                || (challenge.is_none() && hivpp.as_ref().is_some_and(|h| h.vop_id.is_none()));
            if pending {
                let wait = hivpp
                    .as_ref()
                    .and_then(|h| h.wait)
                    .map(|w| Duration::from_secs(w.value()));
                let v = job.vop.as_mut().expect("checked");
                v.polling_id = hivpp
                    .as_ref()
                    .and_then(|h| h.polling_id.as_ref())
                    .map(|p| p.0.clone());
                v.touchdown = response.touchdown().map(|t| t.as_str().to_string());
                let wait = VopWait {
                    not_before: not_before_after(Timestamp::now(), wait),
                    polls_done: v.polls_done,
                };
                info!("[FinTS] Payee check in progress");
                return Ok(Next::Outcome(self.pause_on(
                    JobInput::Transfer(Box::new(job)),
                    Wait::VopPoll(wait),
                    Ticket::local(),
                    notices,
                )));
            }
            let result = match &hivpp {
                Some(h) => job.read_result(h)?,
                None => None,
            };
            let v = job.vop.as_mut().expect("checked");
            if let Some(h) = hivpp {
                v.vop_id = h.vop_id.map(|b| b.0);
                v.valid_until = h.vop_id_valid_until.as_ref().map(local_time);
                v.explanation = h
                    .explanation
                    .map(|e| e.as_str().to_string())
                    .filter(|e| !e.trim().is_empty());
            }
            // A final answer without a result: the bank had none to give
            // (3095; VoP C.10.7.1.1 "HIVPP ohne befüllte
            // VOP-Prüfergebnisse"), shown as an empty result.
            v.result = Some(result.unwrap_or_else(VopResult::empty));
            v.polling_id = None;
            v.touchdown = None;
            v.deviation = response.has(Kind::VopCheckResult);
            v.not_required = response.has(Kind::VopNotRequired);
        }
        // After the final poll, or 3945: the execution order is next.
        if checked
            && (stage == Stage::Poll
                || (challenge.is_none() && response.has(Kind::VopApprovalNotPossible)))
        {
            if job.clean_match() {
                return Ok(Next::Execute(Box::new(job), notices));
            }
            return Ok(Next::Outcome(self.ask_vop(job, None, notices)));
        }
        if let Some(challenge) = challenge {
            let mut tan =
                TanChallenge::from_bank(&challenge, self.inner.bank_params(), Timestamp::now());
            tan.payee_check = job.payee_check();
            job.hkvpa_with_tan = checked
                && job
                    .vop
                    .as_ref()
                    .is_some_and(|v| !v.not_required && v.vop_id.is_some());
            let task_reference = challenge.task_reference;
            if checked && !job.clean_match() {
                return Ok(Next::Outcome(self.ask_vop(
                    job,
                    Some((task_reference, tan)),
                    notices,
                )));
            }
            return Ok(Next::Outcome(self.pause_on(
                JobInput::Transfer(Box::new(job)),
                Wait::Tan(tan),
                Ticket::Task(task_reference),
                notices,
            )));
        }
        // No challenge: the bank executed or took the order without SCA
        // (3076), which needs its 0010/0020.
        let taken = response.has_sca_exemption()
            || response
                .segment_codes
                .iter()
                .any(|c| matches!(c.kind(), Kind::OrderExecuted | Kind::MessageReceived));
        if !taken {
            return Err(FinTSError::InvalidResponse(
                "payment answer without challenge or confirmation".into(),
            ));
        }
        let receipt = job.receipt(response, self.inner.bank_params(), self.inner.now().date());
        Ok(Next::Outcome(Outcome::Done(
            Value::Transfer(receipt),
            notices,
        )))
    }

    /// Pause on the user's decision about the payee check result.
    fn ask_vop(
        &mut self,
        mut job: PaymentJob,
        live_tan: Option<(TaskReference, TanChallenge)>,
        notices: Vec<Notice>,
    ) -> Outcome {
        info!("[FinTS] Payee check: asking the user");
        let confirmation = job.confirmation();
        if let Some(v) = job.vop.as_mut() {
            v.live_tan = live_tan;
        }
        self.pause_on(
            JobInput::Transfer(Box::new(job)),
            Wait::VopConfirmation(confirmation),
            Ticket::local(),
            notices,
        )
    }
}
