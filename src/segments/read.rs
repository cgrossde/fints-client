//! Reading received segments through the codec.
//!
//! Every function here reads a segment with its declarative definition
//! (`codec::read_segment`). A segment the definition rejects is reported
//! (`tracing::warn`, and `Response::segment_errors()` for the caller) and
//! yields nothing; it is never guessed at. Versions without a definition go
//! to the clearly marked remainder in `segments::response` (see there).

use jiff::civil::Date;
use rust_decimal::Decimal;
use tracing::warn;

use crate::codec::defs::{Feedback1, HiTans6, HiTans7, Sdo2, TanMethod6, TanMethod7};
use crate::codec::{self, CodecError, ErrorKind, RawSegment, SegmentHeader, TypedSegment};
use crate::return_code::{Code, Kind, ReturnCode, Scope};
use crate::types::*;

/// Reads a received segment with its definition.
pub(crate) fn typed(raw: &RawSegment) -> Result<(SegmentHeader, TypedSegment), CodecError> {
    codec::read_segment(raw)
}

/// Reports a segment that its definition rejected.
pub(crate) fn report(error: &CodecError) {
    warn!("[FinTS] segment not read: {error}");
}

/// Reads a segment and hands back the typed value, or reports why not.
/// `Err(true)` means there is no definition for this version (the caller
/// may use the remainder in `segments::response`), `Err(false)` that the
/// definition rejected it (already reported).
fn read_or_report(raw: &RawSegment) -> Result<TypedSegment, bool> {
    match typed(raw) {
        Ok((_, t)) => Ok(t),
        Err(e) if e.kind == ErrorKind::UnknownSegment => Err(true),
        Err(e) => {
            report(&e);
            Err(false)
        }
    }
}

// ── Message frame and feedback ─────────────────────────────────────────────

/// Return codes of one HIRMG / HIRMS segment (Formals B.7.2, B.7.3), every
/// code line in wire order with everything it carries. A HIRMS code refers
/// to the customer segment named in the header's reference; its `order` is
/// left for the caller, who knows the sent message.
pub(crate) fn response_codes(raw: &RawSegment) -> Result<Vec<ReturnCode>, CodecError> {
    let (header, segment) = typed(raw)?;
    let (feedback, scope) = match segment {
        TypedSegment::HiRmg2(s) => (s.feedback, Scope::Message),
        TypedSegment::HiRms2(s) => (
            s.feedback,
            Scope::Segment {
                number: header.reference.unwrap_or(0),
                order: None,
            },
        ),
        _ => return Ok(Vec::new()),
    };
    Ok(feedback
        .iter()
        .map(|fb| return_code(fb, scope.clone()))
        .collect())
}

fn return_code(fb: &Feedback1, scope: Scope) -> ReturnCode {
    // The codec admits only `dig 4`.
    let code = Code::new(fb.code.as_str()).expect("feedback code is four digits");
    ReturnCode {
        code,
        text: fb.text.as_str().to_string(),
        scope,
        reference_element: fb
            .reference_element
            .as_ref()
            .map(|e| e.as_str().to_string()),
        parameters: fb
            .parameters
            .iter()
            .map(|p| p.as_str().to_string())
            .collect(),
    }
}

/// Dialog ID and message number from a bank's HNHBK.
pub(crate) fn message_header(raw: &RawSegment) -> Option<(String, u64)> {
    match read_or_report(raw).ok()? {
        TypedSegment::HnHbk3(h) => {
            Some((h.dialog_id.as_str().to_string(), h.message_number.value()))
        }
        _ => None,
    }
}

/// The message size stated in HNHBK.
pub(crate) fn message_size(raw: &RawSegment) -> Option<u64> {
    match read_or_report(raw).ok()? {
        TypedSegment::HnHbk3(h) => h.message_size.value(),
        _ => None,
    }
}

/// The clear-text segments inside an HNVSD (PIN/TAN B.9.10).
pub(crate) fn hnvsd_data(raw: &RawSegment) -> Result<Vec<u8>, CodecError> {
    match typed(raw)?.1 {
        TypedSegment::HnVsd1(v) => Ok(v.data.0),
        _ => Ok(Vec::new()),
    }
}

/// The customer system ID a HISYN assigns (Formals C.8.2).
pub(crate) fn system_id(raw: &RawSegment) -> Option<String> {
    match read_or_report(raw).ok()? {
        TypedSegment::HiSyn4(s) => s.system_id.map(|id| id.as_str().to_string()),
        _ => None,
    }
}

// ── TAN ────────────────────────────────────────────────────────────────

/// Parsed HITAN (TAN challenge response).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Hitan {
    pub task_reference: String,
    pub challenge: String,
    pub challenge_hhduc: Option<Vec<u8>>,
    /// "Gültigkeitsdatum und -uhrzeit für Challenge", as the bank sent it
    /// (bank local time).
    pub valid_until: Option<jiff::civil::DateTime>,
}

/// The challenge of a HITAN (PIN/TAN B.5.1, B.5.2; versions 1-5: E.1.1-
/// E.1.5). A version without a definition yields nothing: HITAN answers
/// the HKTAN we sent, and we send only versions we define. A HITAN its
/// definition rejects is an error: the TAN step
/// must not be skipped because the challenge cannot be read.
pub(crate) fn tan_challenge(raw: &RawSegment) -> Result<Option<Hitan>, CodecError> {
    let typed = match typed(raw) {
        Ok((_, t)) => t,
        Err(e) if e.kind == ErrorKind::UnknownSegment => return Ok(None),
        Err(e) => {
            report(&e);
            return Err(e);
        }
    };
    let (reference, challenge, hhd_uc, valid) = match typed {
        TypedSegment::HiTan1(t) => (
            t.order_reference,
            t.challenge,
            None,
            t.challenge_valid_until,
        ),
        TypedSegment::HiTan2(t) => (
            t.order_reference,
            t.challenge,
            None,
            t.challenge_valid_until,
        ),
        TypedSegment::HiTan3(t) => (
            t.order_reference,
            t.challenge,
            None,
            t.challenge_valid_until,
        ),
        TypedSegment::HiTan4(t) => (
            t.order_reference,
            t.challenge,
            t.challenge_hhd_uc,
            t.challenge_valid_until,
        ),
        TypedSegment::HiTan5(t) => (
            t.order_reference,
            t.challenge,
            t.challenge_hhd_uc,
            t.challenge_valid_until,
        ),
        TypedSegment::HiTan6(t) => (
            t.order_reference,
            t.challenge,
            t.challenge_hhd_uc,
            t.challenge_valid_until,
        ),
        TypedSegment::HiTan7(t) => (
            t.order_reference,
            t.challenge,
            t.challenge_hhd_uc,
            t.challenge_valid_until,
        ),
        _ => return Ok(None),
    };
    // "nochallenge" is a placeholder, not a challenge text.
    let challenge = opt_text(challenge.as_ref()).filter(|c| c != "nochallenge");
    Ok(Some(Hitan {
        task_reference: opt_text(reference.as_ref()).unwrap_or_default(),
        challenge: challenge.unwrap_or_default(),
        challenge_hhduc: hhd_uc.map(|b| b.0).filter(|b| !b.is_empty()),
        valid_until: valid.map(|v| v.date.date().to_datetime(v.time.time())),
    }))
}

// ── BPD / UPD ──────────────────────────────────────────────────────────────

/// TAN methods a HITANS announces. Versions 1-7 are read with their
/// definitions; a version above 7 (none is published) with the remainder
/// parser's guess.
pub(crate) fn tan_methods(raw: &RawSegment) -> Vec<TanMethod> {
    /// The fields shared by element versions #1-#5 of a method:
    /// (security function, process, name, DK method, "TAN medium name
    /// required" code).
    fn older(
        version: u16,
        methods: impl Iterator<Item = (String, String, String, Option<String>, Option<String>)>,
    ) -> Vec<TanMethod> {
        methods
            .map(|(sf, process, name, zka, medium)| TanMethod {
                name,
                security_function: SecurityFunction::new(sf),
                tan_process: TanProcess::from_str_val(&process),
                needs_tan_medium: medium.as_deref() == Some("2"),
                decoupled_max_polls: -1,
                wait_before_first_poll: 0,
                wait_before_next_poll: 0,
                is_decoupled: false,
                hktan_version: version,
                zka_name: zka,
                decoupled_auto_polling: None,
                challenge_structured: false,
            })
            .collect()
    }
    let s = |c: &codec::Code| c.as_str().to_string();
    let a = |c: &codec::An| c.as_str().to_string();
    match read_or_report(raw) {
        Ok(TypedSegment::HiTans1(t)) => older(
            1,
            t.parameters.methods.iter().map(|m| {
                (
                    s(&m.security_function),
                    s(&m.tan_process),
                    a(&m.name),
                    None,
                    None,
                )
            }),
        ),
        Ok(TypedSegment::HiTans2(t)) => older(
            2,
            t.parameters.methods.iter().map(|m| {
                (
                    s(&m.security_function),
                    s(&m.tan_process),
                    a(&m.name),
                    None,
                    None,
                )
            }),
        ),
        Ok(TypedSegment::HiTans3(t)) => older(
            3,
            t.parameters.methods.iter().map(|m| {
                let medium = Some(s(&m.tan_medium_name_required));
                (
                    s(&m.security_function),
                    s(&m.tan_process),
                    a(&m.name),
                    None,
                    medium,
                )
            }),
        ),
        Ok(TypedSegment::HiTans4(t)) => older(
            4,
            t.parameters.methods.iter().map(|m| {
                let zka = opt_text(m.zka_method.as_ref());
                let medium = Some(s(&m.tan_medium_name_required));
                (
                    s(&m.security_function),
                    s(&m.tan_process),
                    a(&m.name),
                    zka,
                    medium,
                )
            }),
        ),
        Ok(TypedSegment::HiTans5(t)) => older(
            5,
            t.parameters.methods.iter().map(|m| {
                let zka = opt_text(m.zka_method.as_ref());
                let medium = Some(s(&m.tan_medium_name_required));
                (
                    s(&m.security_function),
                    s(&m.tan_process),
                    a(&m.name),
                    zka,
                    medium,
                )
            }),
        ),
        Ok(TypedSegment::HiTans6(s)) => tan_methods6(&s),
        Ok(TypedSegment::HiTans7(s)) => tan_methods7(&s),
        Ok(_) | Err(false) => Vec::new(),
        Err(true) => super::response::parse_hitans(raw),
    }
}

fn opt_text(v: Option<&codec::An>) -> Option<String> {
    v.map(|s| s.as_str().to_string()).filter(|s| !s.is_empty())
}

fn tan_methods6(s: &HiTans6) -> Vec<TanMethod> {
    s.parameters
        .methods
        .iter()
        .map(|m: &TanMethod6| TanMethod {
            name: m.name.as_str().to_string(),
            security_function: SecurityFunction::new(m.security_function.as_str()),
            tan_process: TanProcess::from_str_val(m.tan_process.as_str()),
            needs_tan_medium: m.tan_medium_name_required.as_str() == "2",
            decoupled_max_polls: -1,
            wait_before_first_poll: 0,
            wait_before_next_poll: 0,
            is_decoupled: false,
            hktan_version: 6,
            zka_name: opt_text(m.zka_method.as_ref()),
            decoupled_auto_polling: None,
            challenge_structured: m.challenge_structured.0,
        })
        .collect()
}

fn tan_methods7(s: &HiTans7) -> Vec<TanMethod> {
    let num = |n: Option<codec::Num>| n.and_then(|n| i32::try_from(n.value()).ok());
    s.parameters
        .methods
        .iter()
        .map(|m: &TanMethod7| {
            let max_polls = num(m.max_status_polls);
            let first = num(m.wait_before_first_poll);
            let next = num(m.wait_before_next_poll);
            let zka_name = opt_text(m.dk_method.as_ref());
            // A method is decoupled when its DK method name is
            // "Decoupled*"; the decoupled timing fields are only filled
            // for decoupled methods (PIN/TAN B.5.2, TAN2StepPar7).
            let has_decoupled_fields = max_polls.is_some() || first.is_some() || next.is_some();
            let is_decoupled = has_decoupled_fields
                || zka_name
                    .as_deref()
                    .is_some_and(|z| z.to_ascii_lowercase().starts_with("decouple"));
            TanMethod {
                name: m.name.as_str().to_string(),
                security_function: SecurityFunction::new(m.security_function.as_str()),
                tan_process: TanProcess::from_str_val(m.tan_process.as_str()),
                needs_tan_medium: m.tan_medium_name_required.as_str() == "2",
                decoupled_max_polls: max_polls.unwrap_or(-1),
                wait_before_first_poll: first.unwrap_or(0),
                wait_before_next_poll: next.unwrap_or(0),
                is_decoupled,
                hktan_version: 7,
                zka_name,
                decoupled_auto_polling: m.automated_polling_allowed.map(|j| j.0),
                challenge_structured: m.challenge_structured.0,
            }
        })
        .collect()
}

/// One account from HIUPD. Versions 4 and 6 are read with their
/// definitions; other versions (5 has no published table) with the
/// remainder parser.
#[cfg(any(test, feature = "trace"))]
pub(crate) fn upd_account(raw: &RawSegment) -> Option<SepaAccount> {
    match read_or_report(raw) {
        Ok(TypedSegment::HiUpd4(s)) => Some(upd_account4(&s)),
        Ok(TypedSegment::HiUpd6(s)) => Some(upd_account6(&s)),
        Ok(_) | Err(false) => None,
        Err(true) => super::response::parse_hiupd(raw),
    }
}

/// HIUPD 4 (HBCI 2.2 V.3) has no IBAN and no account type.
#[cfg(any(test, feature = "trace"))]
pub(crate) fn upd_account4(s: &crate::codec::defs::HiUpd4) -> SepaAccount {
    let k = &s.account;
    SepaAccount {
        iban: Iban::new(""),
        bic: Bic::new(""),
        account_number: k.account_number.as_str().to_string(),
        sub_account: k
            .subaccount
            .as_ref()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default(),
        blz: Blz::new(
            k.bank
                .bank_code
                .as_ref()
                .map(|b| b.as_str())
                .unwrap_or_default(),
        ),
        owner: opt_text(Some(&s.holder_name_1)).or_else(|| opt_text(s.holder_name_2.as_ref())),
        product_name: opt_text(s.product_name.as_ref()),
        currency: s.currency.as_ref().map(|c| Currency::new(c.as_str())),
        account_type: None,
        allowed_jobs: allowed_jobs(&s.allowed_orders),
    }
}

#[cfg(any(test, feature = "trace"))]
pub(crate) fn upd_account6(s: &crate::codec::defs::HiUpd6) -> SepaAccount {
    let (account_number, sub_account, blz) = match &s.account {
        Some(k) => (
            k.account_number.as_str().to_string(),
            k.subaccount
                .as_ref()
                .map(|s| s.as_str().to_string())
                .unwrap_or_default(),
            k.bank
                .bank_code
                .as_ref()
                .map(|b| b.as_str().to_string())
                .unwrap_or_default(),
        ),
        None => Default::default(),
    };
    SepaAccount {
        iban: Iban::new(opt_text(s.iban.as_ref()).unwrap_or_default()),
        // HIUPD carries no BIC; `AccountInfo` falls back to the
        // bank's BIC.
        bic: Bic::new(""),
        account_number,
        sub_account,
        blz: Blz::new(blz),
        owner: opt_text(Some(&s.holder_name_1)).or_else(|| opt_text(s.holder_name_2.as_ref())),
        product_name: opt_text(s.product_name.as_ref()),
        currency: s.currency.as_ref().map(|c| Currency::new(c.as_str())),
        account_type: s
            .account_type
            .and_then(|t| u8::try_from(t.value()).ok())
            .filter(|t| (1..=99).contains(t)),
        allowed_jobs: allowed_jobs(&s.allowed_orders),
    }
}

#[cfg(any(test, feature = "trace"))]
fn allowed_jobs(orders: &[crate::codec::defs::AllowedOrder2]) -> Vec<String> {
    orders
        .iter()
        .map(|o| o.segment_code.as_str().to_string())
        .collect()
}

// ── Response codes ─────────────────────────────────────────────────────

/// Allowed TAN security functions from response code 3920.
pub(crate) fn extract_allowed_security_functions(codes: &[ReturnCode]) -> Vec<SecurityFunction> {
    codes
        .iter()
        .find_map(|code| match code.kind() {
            Kind::AllowedSecurityFunctions(sfs) => Some(sfs),
            _ => None,
        })
        .unwrap_or_default()
}

/// Touchdown point from response code 3040.
pub(crate) fn find_touchdown(codes: &[ReturnCode]) -> Option<TouchdownPoint> {
    codes.iter().find_map(|code| match code.kind() {
        Kind::Touchdown(td) => Some(td),
        _ => None,
    })
}

// ── Job responses ──────────────────────────────────────────────────────

/// The generic header of a job parameter segment (`HI…S`, `DI…S`), Formals
/// D.6: Maximale Anzahl Aufträge (DE 2), Anzahl Signaturen mindestens
/// (DE 3), Sicherheitsklasse (DE 4, only in FinTS 3.0 versions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParamsHeader {
    pub max_orders: u16,
    pub min_signatures: u8,
    /// `None` in pre-FinTS versions (HISALS 3-5), which go on with the job
    /// parameters right after the signature count.
    pub security_class: Option<u8>,
}

/// Reads the D.6 header of a parameter segment that has no definition (or
/// whose definition rejected it). Tolerant of everything after the header:
/// job-specific elements are not looked at. Returns `None` if the segment has
/// no readable "Maximale Anzahl Aufträge" and "Anzahl Signaturen mindestens".
///
/// The security class is taken to be present if the element after the
/// signature count is a single digit 0 to 4. A pre-FinTS version whose first
/// job parameter is such a digit would be misread; the segment versions
/// with a definition do not depend on this.
pub(crate) fn params_header(raw: &RawSegment) -> Option<ParamsHeader> {
    let single = |index: usize| -> Option<String> {
        let deg = raw.deg(index);
        (deg.len() == 1).then(|| deg.get_str(0))
    };
    let max_orders = single(1)?.parse::<u16>().ok()?;
    let min_signatures = single(2)?.parse::<u8>().ok()?;
    let security_class = single(3)
        .filter(|s| s.len() == 1)
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|c| *c <= 4);
    Some(ParamsHeader {
        max_orders,
        min_signatures,
        security_class,
    })
}

/// Signed amount and date of a balance (Saldo, Messages B.4: `C`/`D`).
fn balance_of(sdo: &Sdo2) -> (Decimal, Option<Date>) {
    let amount = sdo.amount.value.value();
    let signed = if sdo.debit_credit.as_str() == "D" {
        -amount
    } else {
        amount
    };
    (signed, sdo.date.as_ref().map(|d| d.date()))
}

/// The balance from HISAL (Messages C.2.1.2), versions 5-8. Another
/// version yields nothing: HISAL answers the HKSAL we sent. A booked
/// balance without date stays without one; without currency it takes the
/// account currency (HISAL element 4).
pub(crate) fn balance(raw: &RawSegment) -> Option<AccountBalance> {
    let (account_currency, booked, pending, credit_line, available) = match read_or_report(raw)
        .ok()?
    {
        TypedSegment::HiSal5(s) => (s.currency, s.booked, s.pending, s.credit_line, s.available),
        TypedSegment::HiSal6(s) => (s.currency, s.booked, s.pending, s.credit_line, s.available),
        TypedSegment::HiSal7(s) => (s.currency, s.booked, s.pending, s.credit_line, s.available),
        TypedSegment::HiSal8(s) => (s.currency, s.booked, s.pending, s.credit_line, s.available),
        _ => return None,
    };
    let (amount, date) = balance_of(&booked);
    let pending = pending.as_ref().map(balance_of);
    Some(AccountBalance {
        amount,
        date,
        currency: Currency::new(
            booked
                .amount
                .currency
                .as_ref()
                .unwrap_or(&account_currency)
                .as_str(),
        ),
        credit_line: credit_line.map(|b| b.value.value()),
        available: available.map(|b| b.value.value()),
        pending_amount: pending.map(|p| p.0),
        pending_date: pending.and_then(|p| p.1),
    })
}

/// MT940 data from HIKAZ segments, booked and pending separately.
pub struct Mt940ExtractedData {
    pub booked: Vec<u8>,
    pub pending: Vec<u8>,
    /// Whether any HIKAZ carried the pending part (possibly empty).
    pub pending_sent: bool,
}

/// The MT940 payloads of all HIKAZ segments (touchdown pages), in order
/// (Messages C.2.1.1.1).
pub(crate) fn extract_mt940_data(segments: &[RawSegment]) -> Mt940ExtractedData {
    let mut out = Mt940ExtractedData {
        booked: Vec::new(),
        pending: Vec::new(),
        pending_sent: false,
    };
    for seg in segments.iter().filter(|s| s.segment_type() == "HIKAZ") {
        let (booked, pending) = match read_or_report(seg) {
            Ok(TypedSegment::HiKaz5(s)) => (s.booked, s.unbooked),
            Ok(TypedSegment::HiKaz6(s)) => (s.booked, s.unbooked),
            Ok(TypedSegment::HiKaz7(s)) => (s.booked, s.unbooked),
            _ => continue,
        };
        out.booked.extend_from_slice(booked.as_bytes());
        if let Some(p) = pending {
            out.pending_sent = true;
            out.pending.extend_from_slice(p.as_bytes());
        }
    }
    out
}

/// camt.052 documents from HICAZ segments, booked and pending separately.
/// Each entry is one complete XML document.
pub struct CamtExtractedData {
    pub booked: Vec<Vec<u8>>,
    pub pending: Vec<Vec<u8>>,
    /// Whether any HICAZ carried the pending part (possibly empty).
    pub pending_sent: bool,
    /// The `camt-Descriptor` of each HICAZ, as the bank wrote it.
    pub descriptors: Vec<String>,
}

/// The camt documents of all HICAZ segments (touchdown pages), in order
/// (Messages C.2.3.1.1.1).
pub(crate) fn extract_camt_data(segments: &[RawSegment]) -> CamtExtractedData {
    let mut out = CamtExtractedData {
        booked: Vec::new(),
        pending: Vec::new(),
        pending_sent: false,
        descriptors: Vec::new(),
    };
    for seg in segments.iter().filter(|s| s.segment_type() == "HICAZ") {
        let Ok(TypedSegment::HiCaz1(s)) = read_or_report(seg) else {
            continue;
        };
        out.descriptors.push(s.camt_descriptor.as_str().to_string());
        out.booked.extend(
            s.booked
                .documents
                .into_iter()
                .map(|d| d.0)
                .filter(|d| !d.is_empty()),
        );
        if let Some(pending) = s.pending {
            out.pending_sent = true;
            if !pending.0.is_empty() {
                out.pending.push(pending.0);
            }
        }
    }
    out
}

/// The SWIFT statements of all `code` segments (touchdown pages) joined
/// into one stream, each page on its own line.
fn joined_statements(segments: &[RawSegment], code: &str) -> Vec<u8> {
    let mut joined = Vec::new();
    for seg in segments.iter().filter(|s| s.segment_type() == code) {
        let statement = match read_or_report(seg) {
            Ok(TypedSegment::HiWpd5(s)) => s.statement,
            Ok(TypedSegment::HiWpd6(s)) => s.statement,
            Ok(TypedSegment::HiWdu5(s)) => s.statement,
            _ => continue,
        };
        if !joined.is_empty() && !joined.ends_with(b"\n") {
            joined.extend_from_slice(b"\r\n");
        }
        joined.extend_from_slice(statement.as_bytes());
    }
    joined
}

/// Holdings from HIWPD: one MT535 statement per segment (Messages C.4.3.1).
pub(crate) fn parse_hiwpd(segments: &[RawSegment]) -> Vec<SecurityHolding> {
    let mt535 = joined_statements(segments, "HIWPD");
    if mt535.is_empty() {
        return Vec::new();
    }
    super::mt535::parse_mt535(&mt535)
}

/// Depot transactions from HIWDU: one MT536 statement per segment
/// (Messages C.4.3.2).
pub(crate) fn parse_hiwdu(segments: &[RawSegment]) -> Vec<DepotTransaction> {
    super::mt536::parse_mt536(&joined_statements(segments, "HIWDU"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_without_date_or_currency_keeps_the_amount() {
        let b = |booked: &str| {
            balance(&parse_segment(&format!(
                "HISAL:4:6:3+1234567890::280:12345678+Girokonto+USD+{booked}'"
            )))
            .expect("balance")
        };
        let full = b("D:10,5:EUR:20260315");
        assert_eq!(full.amount, Decimal::new(-105, 1));
        assert_eq!(full.currency.as_str(), "EUR");
        assert_eq!(full.date.unwrap().to_string(), "2026-03-15");
        let no_date = b("C:10,5:EUR");
        assert_eq!(no_date.date, None);
        assert_eq!(no_date.currency.as_str(), "EUR");
        let no_currency = b("C:10,5::20260315");
        assert_eq!(no_currency.currency.as_str(), "USD");
        assert!(no_currency.date.is_some());
        let neither = b("C:10,5");
        assert_eq!((neither.amount, neither.date), (Decimal::new(105, 1), None));
        assert_eq!(neither.currency.as_str(), "USD");
    }

    #[test]
    fn params_header_reads_undefined_segments() {
        let h = |s: &str| params_header(&parse_segment(s));
        // FinTS 3.0: with security class, job parameters ignored.
        assert_eq!(
            h("HICCSS:9:1:4+1+1+0+J:N:3:foo'"),
            Some(ParamsHeader {
                max_orders: 1,
                min_signatures: 1,
                security_class: Some(0)
            })
        );
        // Several signatures, header only.
        assert_eq!(
            h("HIPROS:9:4:4+999+2+4'"),
            Some(ParamsHeader {
                max_orders: 999,
                min_signatures: 2,
                security_class: Some(4)
            })
        );
        // Pre-FinTS (HISALS 5): the third element is a job parameter.
        assert_eq!(
            h("HISALS:9:5:4+1+1+J'"),
            Some(ParamsHeader {
                max_orders: 1,
                min_signatures: 1,
                security_class: None
            })
        );
        // Pre-FinTS, parameter group right after the header.
        assert_eq!(
            h("HIKAZS:9:5:4+1+1+60:J:J'"),
            Some(ParamsHeader {
                max_orders: 1,
                min_signatures: 1,
                security_class: None
            })
        );
        // Not a header.
        assert_eq!(h("HIXXXS:9:1:4'"), None);
        assert_eq!(h("HIXXXS:9:1:4+x+1+0'"), None);
        assert_eq!(h("HIXXXS:9:1:4+1:2+1+0'"), None);
    }

    /// Tokenizes one segment with the codec's tokenizer.
    pub(crate) fn parse_segment(s: &str) -> RawSegment {
        codec::wire::tokenize(s.as_bytes()).unwrap().remove(0)
    }

    // ── Feedback, frame ────────────────────────────────────────────────

    #[test]
    fn response_codes_with_parameters() {
        let seg = parse_segment("HIRMS:4:2:3+0020::ok+3920::Zugelassene Verfahren:912:940'");
        let codes = response_codes(&seg).unwrap();
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0].code.as_str(), "0020");
        assert_eq!(
            codes[1].kind(),
            Kind::AllowedSecurityFunctions(vec![
                SecurityFunction::new("912"),
                SecurityFunction::new("940")
            ])
        );
        let scope = Scope::Segment {
            number: 3,
            order: None,
        };
        assert!(codes.iter().all(|c| c.scope == scope));
    }

    /// Nothing of a code line is dropped: reference element, parameters,
    /// scope. An empty parameter is an absent repetition (codec rule), so
    /// it does not appear.
    #[test]
    fn response_codes_keep_the_whole_line() {
        let seg = parse_segment("HIRMS:4:2:5+9210:3:Konto unbekannt:A1::B2'");
        let c = &response_codes(&seg).unwrap()[0];
        assert_eq!(c.code.as_str(), "9210");
        assert_eq!(c.text, "Konto unbekannt");
        assert_eq!(c.reference_element.as_deref(), Some("3"));
        assert_eq!(c.parameters, ["A1", "B2"]);
        assert_eq!(
            c.scope,
            Scope::Segment {
                number: 5,
                order: None
            }
        );
        let seg = parse_segment("HIRMG:3:2+9999::Unbekannt:P1'");
        let c = &response_codes(&seg).unwrap()[0];
        assert_eq!((c.scope.clone(), c.kind()), (Scope::Message, Kind::Other));
        assert_eq!(c.parameters, ["P1"]);
    }

    #[test]
    fn feedback_text_longer_than_80_is_read() {
        // Longer than its `an ..80` (Formals C.8.1), as DKB sent on a failed
        // login: the code and the whole text come through.
        let long = "x".repeat(93);
        for wire in [
            format!("HIRMS:4:2:3+9931::{long}'"),
            format!("HIRMG:2:2+9050::{long}'"),
        ] {
            let c = &response_codes(&parse_segment(&wire)).unwrap()[0];
            assert_eq!(c.text, long);
        }
    }

    #[test]
    fn dialog_id_from_hnhbk() {
        let seg = parse_segment("HNHBK:1:3+000000000123+300+DLG?:1+2+DLG?:1:1'");
        assert_eq!(message_header(&seg), Some(("DLG:1".into(), 2)));
    }

    // ── Simple parsers ─────────────────────────────────────────────────

    #[test]
    fn system_id_from_hisyn() {
        let seg = parse_segment("HISYN:5:4:3+SYS-0042'");
        assert_eq!(system_id(&seg).as_deref(), Some("SYS-0042"));
    }

    // ── HITAN ──────────────────────────────────────────────────────────

    #[test]
    fn hitan_7_reference_and_text() {
        let seg = parse_segment("HITAN:4:7:3+4++AUF-0815+Freigabe in der Test-App'");
        let step = tan_challenge(&seg).unwrap().unwrap();
        assert_eq!(
            (step.task_reference.as_str(), step.challenge.as_str()),
            ("AUF-0815", "Freigabe in der Test-App")
        );
        assert_eq!(step.challenge_hhduc, None);
    }

    /// A HITAN its definition rejects is an error naming the segment, not
    /// "no TAN step".
    #[test]
    fn unreadable_hitan_6_and_7_are_errors() {
        for (wire, version) in [
            ("HITAN:4:6:4+44++TASK-1+Bitte freigeben'", 6),
            ("HITAN:4:7:4+44++TASK-1+Bitte freigeben'", 7),
            ("HITAN:4:7:4'", 7),
        ] {
            let seg = parse_segment(wire);
            let e = tan_challenge(&seg).unwrap_err();
            let id = e.segment.as_ref().expect("segment in the error");
            assert_eq!((id.code.as_str(), id.version), ("HITAN", version), "{wire}");
            assert!(!e.field.is_empty(), "{wire}: {e}");
        }
    }

    /// HITAN 6 DE 3 is the order hash (binary); the reference comes after
    /// it, and the placeholder `nochallenge` means no text.
    #[test]
    fn hitan_6_order_hash_is_not_the_reference() {
        let seg = parse_segment("HITAN:4:6:3+4+@5@HASH1+AUF-0816+nochallenge'");
        let step = tan_challenge(&seg).unwrap().unwrap();
        assert_eq!(step.task_reference, "AUF-0816");
        assert!(step.challenge.is_empty());
    }

    #[test]
    fn hitan_hhd_uc_challenge() {
        let seg = parse_segment("HITAN:5:7:4+4++REF-1+Bitte scannen+@3@\x01\x02\x03'");
        let h = tan_challenge(&seg).unwrap().unwrap();
        assert_eq!(h.challenge_hhduc.as_deref(), Some(&[1u8, 2, 3][..]));
    }

    /// HITAN 5 has no definition; the remainder parser reads it.
    #[test]
    fn hitan_v5_through_the_remainder() {
        let seg = parse_segment("HITAN:4:5:4+4++TASK-7+Bitte geben Sie die TAN ein'");
        let h = tan_challenge(&seg).unwrap().unwrap();
        assert_eq!(h.task_reference, "TASK-7");
        assert_eq!(h.challenge, "Bitte geben Sie die TAN ein");
    }

    // ── HITANS (TAN2StepPar6/7) ────────────────────────────────────────

    /// One method of a HITANS 7 parameter group, TAN2StepParams7 with its
    /// 26 elements: DE 19 says whether a TAN medium must be named, and the
    /// last five are the decoupled poll parameters (`::::` for none).
    fn tan2step7(code: &str, zka: &str, name: &str, medium: &str, decoupled: &str) -> String {
        let method = format!(
            "{code}:2:T{code}:{zka}::{name}:::Freigabe:2048:N:1:N:0:0:N:N:00:{medium}:N:1:{decoupled}"
        );
        assert_eq!(method.split(':').count(), 26, "{method}");
        method
    }

    fn hitans7(methods: &[String]) -> Vec<TanMethod> {
        let wire = format!("HITANS:8:7:4+1+1+1+N:N:0:{}'", methods.join(":"));
        tan_methods(&parse_segment(&wire))
    }

    #[test]
    fn hitans_7_decoupled_and_chip_methods() {
        let methods = hitans7(&[
            tan2step7("942", "Decoupled", "Test-App", "2", "120:4:3:J:J"),
            tan2step7("972", "HHD", "Kartenleser", "0", "::::"),
        ]);
        let [app, reader] = methods.as_slice() else {
            panic!("{methods:?}");
        };

        assert_eq!(app.security_function.as_str(), "942");
        assert_eq!(app.name, "Test-App");
        assert_eq!(app.zka_name.as_deref(), Some("Decoupled"));
        assert_eq!(app.hktan_version, 7);
        assert!(app.is_decoupled && app.needs_tan_medium);
        assert_eq!(
            (
                app.decoupled_max_polls,
                app.wait_before_first_poll,
                app.wait_before_next_poll
            ),
            (120, 4, 3)
        );
        assert_eq!(app.decoupled_auto_polling, Some(true));

        assert_eq!(reader.security_function.as_str(), "972");
        assert!(!reader.is_decoupled && !reader.needs_tan_medium);
        // No poll limit given.
        assert_eq!(reader.decoupled_max_polls, -1);
    }

    /// A poll limit of 0 means "no limit", kept as 0; medium flag 1 means
    /// a medium may be named, not that it must.
    #[test]
    fn hitans_7_poll_limit_zero_and_optional_medium() {
        let methods = hitans7(&[tan2step7("943", "DecoupledPush", "Push", "1", "0:6:6:N:N")]);
        let push = &methods[0];
        assert!(push.is_decoupled);
        assert_eq!(push.decoupled_max_polls, 0);
        assert_eq!(push.decoupled_auto_polling, Some(false));
        assert!(!push.needs_tan_medium);
    }

    /// Synthetic HITANS v6: three TAN methods in ONE parameter group
    /// (TAN2StepParams6, 21 DEs per method). Method names and IDs are invented.
    pub(crate) const SYNTHETIC_HITANS_V6: &str = "HITANS:41:6:4+1+1+1+J:N:0:\
        921:2:TESTHHD:HHD:1.4:Testverfahren Alpha:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0:\
        922:2:TESTSMS:mobileTAN::Testverfahren Beta:6:1:TAN:6:N:1:N:0:0:N:N:00:2:N:1:\
        923:2:TESTOPT:HHDOPT1:1.4:Testverfahren Gamma:6:1:TAN:6:N:1:N:0:0:N:N:00:1:N:1'";

    /// Synthetic HITANS v7: three TAN methods in ONE parameter group
    /// (TAN2StepParams7, 26 DEs per method), one of them decoupled with poll
    /// parameters. Method names and IDs are invented.
    pub(crate) const SYNTHETIC_HITANS_V7: &str = "HITANS:42:7:4+1+1+1+N:N:0:\
        931:2:TESTHHD:HHD:1.4:Kartenleser Test:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0::::::\
        932:2:TESTAPP:Decoupled::App-Freigabe Test:::Freigabe:2048:N:2:N:0:0:N:N:00:2:N:1:60:4:2:J:J:\
        933:2:TESTSMS:mobileTAN::SMS Test:6:1:TAN:6:N:1:N:0:0:N:N:00:2:N:1:::::'";

    #[test]
    fn test_parse_hitans_v6() {
        let methods = tan_methods(&parse_segment(SYNTHETIC_HITANS_V6));
        let got: Vec<_> = methods
            .iter()
            .map(|m| (m.security_function.as_str(), m.name.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("921", "Testverfahren Alpha"),
                ("922", "Testverfahren Beta"),
                ("923", "Testverfahren Gamma")
            ]
        );
        assert_eq!(methods[0].zka_name.as_deref(), Some("HHD"));
        assert!(!methods[0].needs_tan_medium);
        assert!(methods[1].needs_tan_medium, "2 = required");
        assert!(!methods[2].needs_tan_medium, "1 = optional, not required");
        assert!(
            methods
                .iter()
                .all(|m| !m.is_decoupled && m.hktan_version == 6)
        );
    }

    #[test]
    fn test_parse_hitans_v7_synthetic_group() {
        let methods = tan_methods(&parse_segment(SYNTHETIC_HITANS_V7));
        let sfs: Vec<_> = methods
            .iter()
            .map(|m| m.security_function.as_str())
            .collect();
        assert_eq!(sfs, ["931", "932", "933"]);

        let card = &methods[0];
        assert_eq!(card.name, "Kartenleser Test");
        assert!(!card.is_decoupled);
        assert_eq!(card.decoupled_max_polls, -1);

        let app = &methods[1];
        assert_eq!(app.name, "App-Freigabe Test");
        assert_eq!(app.zka_name.as_deref(), Some("Decoupled"));
        assert!(app.is_decoupled);
        assert!(app.needs_tan_medium);
        assert_eq!(
            (
                app.decoupled_max_polls,
                app.wait_before_first_poll,
                app.wait_before_next_poll
            ),
            (60, 4, 2)
        );
        assert_eq!(app.decoupled_auto_polling, Some(true));
        assert_eq!(app.hktan_version, 7);

        let sms = &methods[2];
        assert_eq!(sms.name, "SMS Test");
        assert!(!sms.is_decoupled);
        assert!(sms.needs_tan_medium);
    }

    /// Regression for the HITANS parser that read one TAN method per data
    /// element group: all methods share the single ParTAN2Step group, so that
    /// parser returned one bogus method here instead of three.
    #[test]
    fn regression_hitans_all_methods_in_one_group() {
        for raw in [SYNTHETIC_HITANS_V6, SYNTHETIC_HITANS_V7] {
            let seg = parse_segment(raw);
            assert_eq!(
                seg.deg_count(),
                5,
                "fixture: header + 3 DEs + one parameter group"
            );
            let methods = tan_methods(&seg);
            assert_eq!(methods.len(), 3, "{raw}");
            for m in &methods {
                let sf = m.security_function.as_str();
                assert!(
                    sf.len() == 3 && sf.bytes().all(|b| b.is_ascii_digit()),
                    "misaligned group: security function {sf:?}"
                );
                assert!(!m.name.starts_with("TAN-"), "missing name for {sf}");
            }
        }
    }

    /// Development-only oracle check: parses every HITANS line of a local TSV
    /// file (`<source>\t<segment>` per line, `#` comments) named by the
    /// `FINTS_HITANS_ORACLE` environment variable. Skipped when the variable is
    /// unset. The file holds third-party test data and is never committed.
    #[test]
    fn hitans_oracle_from_env() {
        let Ok(path) = std::env::var("FINTS_HITANS_ORACLE") else {
            return;
        };
        let text = std::fs::read_to_string(&path).expect("read FINTS_HITANS_ORACLE");
        let mut checked = 0;
        for line in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let (src, raw) = line.split_once('\t').unwrap_or(("?", line));
            let seg = parse_segment(raw);
            let methods = tan_methods(&seg);
            assert!(
                !methods.is_empty(),
                "{src} v{}: no methods",
                seg.segment_version()
            );
            for m in &methods {
                let sf = m.security_function.as_str();
                assert!(
                    sf.len() == 3 && sf.bytes().all(|b| b.is_ascii_digit()),
                    "{src} v{}: bad security function {sf:?} ({})",
                    seg.segment_version(),
                    m.name
                );
                assert!(!m.name.starts_with("TAN-"), "{src}: missing name for {sf}");
                if seg.segment_version() >= 7 {
                    assert_eq!(
                        m.is_decoupled,
                        m.zka_name
                            .as_deref()
                            .is_some_and(|z| z.to_ascii_lowercase().starts_with("decouple")),
                        "{src}: decoupled flag for {sf}"
                    );
                }
            }
            checked += 1;
        }
        eprintln!("HITANS oracle: {checked} segments checked");
    }

    #[test]
    fn test_parse_hitans_garbage_does_not_panic() {
        for raw in [
            "HITANS:1:7:4'",
            "HITANS:1:7:4+1+1+1+J'",
            "HITANS:1:99:4+1+1+1+J:N:0:900'",
            "HITANS:1:1:4+x'",
        ] {
            let _ = tan_methods(&parse_segment(raw));
        }
    }

    // ── Response code helpers ──────────────────────────────────────────

    fn rc(code: &str, params: &[&str]) -> ReturnCode {
        ReturnCode::new(Code::new(code).unwrap(), "t", Scope::Message)
            .with_parameters(params.iter().map(|p| p.to_string()).collect())
    }

    #[test]
    fn test_extract_allowed_security_functions_empty() {
        let codes = vec![rc("0020", &[])];
        assert!(extract_allowed_security_functions(&codes).is_empty());
    }

    #[test]
    fn test_extract_allowed_security_functions() {
        let codes = vec![rc("3920", &["912", "940"])];
        let sfs = extract_allowed_security_functions(&codes);
        assert_eq!(sfs.len(), 2);
        assert_eq!(sfs[0], SecurityFunction::new("912"));
        assert_eq!(sfs[1], SecurityFunction::new("940"));
    }

    #[test]
    fn test_find_touchdown() {
        let codes = vec![rc("0020", &[]), rc("3040", &["TOUCH123"])];
        let td = find_touchdown(&codes);
        assert_eq!(td, Some(TouchdownPoint::new("TOUCH123")));
    }

    #[test]
    fn test_find_touchdown_none() {
        let codes = vec![rc("0020", &[])];
        assert!(find_touchdown(&codes).is_none());
    }

    // ── HISAL ──────────────────────────────────────────────────────────

    #[test]
    fn hisal_v7_balance_pending_and_limits() {
        let seg = parse_segment(
            "HISAL:5:7:3+DE00120300009900012345:BYLADEM1001+Girokonto+EUR+\
             C:1234,56:EUR:20260315+D:10,:EUR:20260316+500,:EUR+1734,56:EUR'",
        );
        let b = balance(&seg).unwrap();
        assert_eq!(b.amount, Decimal::new(123456, 2));
        assert_eq!(b.currency.as_str(), "EUR");
        assert_eq!(b.date, Some(Date::new(2026, 3, 15).unwrap()));
        assert_eq!(b.pending_amount, Some(Decimal::from(-10)));
        assert_eq!(b.pending_date, Some(Date::constant(2026, 3, 16)));
        assert_eq!(b.credit_line, Some(Decimal::from(500)));
        assert_eq!(b.available, Some(Decimal::new(173456, 2)));
    }

    #[test]
    fn hisal_v6_debit_balance() {
        let seg = parse_segment(
            "HISAL:5:6:3+9900012345::280:12030000+Girokonto+EUR+D:42,5:EUR:20260101'",
        );
        let b = balance(&seg).unwrap();
        assert_eq!(b.amount, Decimal::new(-425, 1));
        assert!(b.pending_amount.is_none() && b.credit_line.is_none());
    }

    #[test]
    fn hisal_v8_through_the_remainder() {
        let seg = parse_segment(
            "HISAL:5:8:3+DE00120300009900012345:BYLADEM1001+Girokonto+EUR+C:1,5:EUR:20260101'",
        );
        assert_eq!(balance(&seg).unwrap().amount, Decimal::new(15, 1));
    }

    // ── HIKAZ, HICAZ, HICAZS ───────────────────────────────────────────

    #[test]
    fn mt940_pages_are_joined() {
        let segs = [
            parse_segment("HIKAZ:5:7:3+@3@abc+@2@xy'"),
            parse_segment("HIKAZ:6:7:3+@3@def'"),
            parse_segment("HICAZ:7:1:3+@3@zzz'"),
        ];
        let data = extract_mt940_data(&segs);
        assert_eq!(data.booked, b"abcdef");
        assert_eq!(data.pending, b"xy");
    }

    #[test]
    fn extract_camt_documents_from_hicaz() {
        let seg = parse_segment(
            "HICAZ:5:1:3+DE00120300009900012345:BYLADEM1001+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+@5@<a/>1:@5@<b/>2+@5@<c/>3'",
        );
        let other = parse_segment("HIKAZ:6:7:3+@3@abc'");
        let camt = extract_camt_data(&[seg, other]);
        assert_eq!(camt.booked, vec![b"<a/>1".to_vec(), b"<b/>2".to_vec()]);
        assert_eq!(camt.pending, vec![b"<c/>3".to_vec()]);
    }

    // ── HIWPD, HIWDU ───────────────────────────────────────────────────

    /// A segment whose only data element is `payload` as binary.
    fn binary_segment(header: &str, payload: &[u8]) -> RawSegment {
        let mut wire = Vec::from(format!("{header}+@{}@", payload.len()));
        wire.extend(payload);
        wire.push(b'\'');
        let mut segments = codec::wire::tokenize(&wire).unwrap();
        segments.swap_remove(0)
    }

    const DEPOT_HEADER: &str = ":16R:GENL\r\n:97A::SAFE//12345678/0000004711\r\n";

    #[test]
    fn hiwpd_holdings_from_mt535() {
        let mt535 = format!(
            "{DEPOT_HEADER}:16S:GENL\r\n:16R:FIN\r\n:35B:ISIN DE0000000001\r\n/DE/A0TEST\r\n\
             TESTAKTIE AG\r\n:93B::AGGR//UNIT/8,\r\n:19A::HOLD//EUR100,\r\n:16S:FIN\r\n-"
        );
        let holdings = parse_hiwpd(&[binary_segment("HIWPD:4:6:3", mt535.as_bytes())]);
        let [share] = holdings.as_slice() else {
            panic!("{holdings:?}");
        };
        assert_eq!(share.name, "TESTAKTIE AG");
        assert_eq!(share.quantity, Decimal::from(8));
        assert_eq!(share.market_value, Some(Decimal::from(100)));
        assert_eq!(share.depot_id.as_deref(), Some("0000004711"));
    }

    #[test]
    fn hiwpd_with_an_inactive_depot_has_no_holdings() {
        let mt535 = format!("{DEPOT_HEADER}:17B::ACTI//N\r\n:16S:GENL\r\n-");
        assert!(parse_hiwpd(&[binary_segment("HIWPD:4:6:3", mt535.as_bytes())]).is_empty());
    }

    /// HIWPD carries one binary MT535 (Messages C.4.3.1). A segment
    /// without one, and other segments, give no holdings.
    #[test]
    fn hiwpd_without_mt535_and_other_segments() {
        let text_only = parse_segment("HIWPD:4:6:3+DE00123456780000004711:TESTDEFFXXX'");
        let balance =
            parse_segment("HISAL:5:7:3+DE00123456780000004711:TESTDEFFXXX+C:10,:EUR:20260131'");
        assert!(parse_hiwpd(&[text_only, balance]).is_empty());
    }

    /// A statement split over several HIWDU segments is one MT536.
    #[test]
    fn hiwdu_segments_form_one_mt536() {
        let first = b":16R:FIN\r\n:35B:ISIN DE0000000001\r\n:16R:TRAN\r\n:36B::PSTA//UNIT/2,\r\n";
        let second = b":22H::REDE//DELI\r\n:16S:TRAN\r\n:16S:FIN";
        let transactions = parse_hiwdu(&[
            binary_segment("HIWDU:4:5:3", first),
            binary_segment("HIWDU:5:5:3", second),
        ]);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].quantity, Decimal::from(2));
        assert_eq!(
            transactions[0].direction,
            DepotTransactionDirection::Deliver
        );
        assert!(parse_hiwdu(&[parse_segment("HIWDU:4:5:3+'")]).is_empty());
    }

    // ── HIUPD ──────────────────────────────────────────────────────────

    /// HIUPD 6 for user `kunde7` with KTV `account`, the given IBAN and
    /// Kontoart (DE 5), then the product name and one allowed order.
    fn hiupd6(account: &str, iban: &str, kind: &str, product: &str) -> RawSegment {
        parse_segment(&format!(
            "HIUPD:6:6:4+{account}::280:12345678+{iban}+kunde7+{kind}+EUR+Erika Muster++{product}++HKSAL:1'"
        ))
    }

    #[test]
    fn hiupd_6_giro_account() {
        let giro = upd_account(&hiupd6("4711", "DE00123456780000004711", "", "Girokonto")).unwrap();
        assert_eq!(giro.iban.as_str(), "DE00123456780000004711");
        assert_eq!(
            (giro.account_number.as_str(), giro.blz.as_str()),
            ("4711", "12345678")
        );
        assert_eq!(giro.owner.as_deref(), Some("Erika Muster"));
        assert_eq!(giro.product_name.as_deref(), Some("Girokonto"));
        assert_eq!(giro.currency.as_ref().map(Currency::as_str), Some("EUR"));
        assert_eq!(giro.account_type, None);
    }

    /// Kontoart is `num ..2`, 1 to 99, in ranges of ten per class.
    #[test]
    fn hiupd_6_kontoart_gives_the_class() {
        for (kind, class) in [
            ("1", AccountClass::Checking),
            ("30", AccountClass::Depot),
            ("50", AccountClass::CreditCard),
        ] {
            let account = upd_account(&hiupd6("4711", "", kind, "Testkonto")).unwrap();
            assert_eq!(account.account_type, kind.parse().ok(), "{kind}");
            assert_eq!(account.classify().map(|(c, _)| c), Some(class), "{kind}");
        }
        // 0 is out of range: no type.
        let zero = upd_account(&hiupd6("4711", "", "0", "Testkonto")).unwrap();
        assert_eq!(zero.account_type, None);
        // Not a number: the definition rejects the segment.
        assert!(upd_account(&hiupd6("4711", "", "xy", "Testkonto")).is_none());
    }
}
