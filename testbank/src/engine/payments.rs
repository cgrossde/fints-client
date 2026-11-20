//! Payments in the test bank: HKCCS, HKIPZ, HKIPS, the payee check (HKVPP,
//! HKVPA) and the parameter segments that announce them.
//!
//! The flow follows VoP E.8.1 and Messages C.10.2: message 1 is
//! `HKVPP + order + HKTAN:4`; the bank answers by return code (HIVPP with
//! the result, `0025`, `3090`, `3091`, `3093`, `3945`); the execution order
//! is `HKVPA + order + HKTAN:4` with the same pain.001 bytes. An order is
//! executed only after the TAN is accepted (or at once on an SCA waiver).
//! Everything a bank may do differently is a field of
//! [`PaymentsModel`](crate::model::PaymentsModel).

use std::str::FromStr;

use fints_client::codec::defs::common::{Kti1, Tsp1};
use fints_client::codec::defs::{
    HiCcss1, HiIps1, HiIpss1, HiIpz1, HiIpz2, HiIpzs1, HiIpzs2, HiVpas1, HiVpp1, HiVpps1, HkCcs1,
    HkIps1, HkIpz1, HkIpz2, HkVpa1, HkVpp1, InstantPaymentParameters1, InstantPaymentParameters2,
    InstantStatusParameters1, PayeeCheckParameters1, PayeeCheckResult1,
};
use fints_client::codec::{Bin, Dat, Jn, RawSegment, Tim};
use quick_xml::Reader;
use quick_xml::events::Event;
use rust_decimal::Decimal;

use super::{
    AccountRef, DialogState, Hktan, State, TEXT_OK, TEXT_SCA_REQUIRED, an, checked_account, code,
    continue_tan, decode, exemption, method_for, num, version_of,
};
use crate::model::{
    BankModel, Delivery, LimitAnswer, OnMatch, Payment, PaymentsModel, ResultAs, TanRule, VopModel,
};
use crate::request::Request;
use crate::response::{Answer, Feedback, MESSAGE_WARNINGS, Reply};

/// The execution date a payment order must carry (Messages C.10.2.1: the
/// bank executes at once; the date is the fixed value 1999-01-01).
const EXECUTION_DATE: &str = "1999-01-01";

/// Whether `code` is a payment order the test bank executes.
pub(super) fn is_payment(code: &str) -> bool {
    matches!(code, "HKCCS" | "HKIPZ")
}

/// The result of the payee check for one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Check {
    /// RCVC.
    Match,
    /// RVMC with the registered name.
    Close(String),
    /// RVNM.
    NoMatch,
    /// RVNA with the reason text.
    NotApplicable(String),
}

impl Check {
    fn status(&self) -> &'static str {
        match self {
            Check::Match => "RCVC",
            Check::Close(_) => "RVMC",
            Check::NoMatch => "RVNM",
            Check::NotApplicable(_) => "RVNA",
        }
    }

    fn clean(&self) -> bool {
        matches!(self, Check::Match)
    }
}

/// A payee check in progress or done in one dialog.
#[derive(Debug, Clone)]
pub(super) struct PaymentCase {
    /// `HKCCS` or `HKIPZ`.
    code: &'static str,
    /// The pain.001 bytes of message 1; HKVPA must resend them.
    pain: Vec<u8>,
    /// `OrgnlMsgNmId`: the received message's name.
    msg_name: String,
    msg_id: String,
    pmt_inf_id: String,
    end_to_end_id: String,
    payee_name: String,
    payee_iban: String,
    /// The pain.002 descriptor the client asked for and the bank serves.
    descriptor: String,
    check: Check,
    polling_id: Option<Vec<u8>>,
    /// The touchdown point of the last pending answer.
    touchdown: Option<String>,
    /// HKVPP answers given so far (message 1 and each poll).
    answers_given: u32,
    /// Set when the check is complete.
    pub(super) vop_id: Option<Vec<u8>>,
}

/// What the bank read from a pain.001 (only what it needs).
#[derive(Debug, Clone)]
struct ParsedPain {
    /// The message name of the root namespace (`pain.001.001.09`).
    msg_name: String,
    msg_id: String,
    pmt_inf_id: String,
    exec_date: String,
    end_to_end_id: String,
    payee_name: String,
    payee_iban: String,
    amount: Decimal,
    instant: bool,
}

fn local_name(name: &[u8]) -> String {
    let full = String::from_utf8_lossy(name);
    match full.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => full.into_owned(),
    }
}

/// The text elements of an XML document with their element paths.
fn xml_texts(xml: &[u8]) -> Result<Vec<(Vec<String>, String)>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut out = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("pain.001 is not XML: {e}"))?
        {
            Event::Start(e) => {
                path.push(local_name(e.name().as_ref()));
                text.clear();
            }
            Event::Text(t) => text.push_str(&t.decode().map_err(|e| e.to_string())?),
            Event::CData(t) => text.push_str(&String::from_utf8_lossy(&t[..])),
            Event::GeneralRef(r) => {
                let name = r.decode().map_err(|e| e.to_string())?;
                if let Some(c) = r.resolve_char_ref().map_err(|e| e.to_string())? {
                    text.push(c);
                } else if let Some(s) = quick_xml::escape::resolve_predefined_entity(&name) {
                    text.push_str(s);
                } else {
                    return Err(format!("pain.001: unknown entity &{name};"));
                }
            }
            Event::End(_) => {
                if !text.trim().is_empty() {
                    out.push((path.clone(), text.trim().to_string()));
                }
                text.clear();
                path.pop();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// The elements of a pain.001 the bank needs: `MsgId`, `ReqdExctnDt`,
/// `EndToEndId`, the creditor's name and IBAN, the amount and the instant
/// local instrument.
fn parse_pain(xml: &[u8]) -> Result<ParsedPain, String> {
    let texts = xml_texts(xml)?;
    let find = |what: &str, pred: &dyn Fn(&[String]) -> bool| -> Result<String, String> {
        texts
            .iter()
            .find(|(p, _)| pred(p))
            .map(|(_, t)| t.clone())
            .ok_or_else(|| format!("pain.001 has no {what}"))
    };
    let ends = |p: &[String], tail: &[&str]| {
        p.len() >= tail.len()
            && p[p.len() - tail.len()..]
                .iter()
                .zip(tail)
                .all(|(a, b)| a == b)
    };
    let has = |p: &[String], name: &str| p.iter().any(|e| e == name);
    let amount = find("InstdAmt", &|p| ends(p, &["Amt", "InstdAmt"]))?;
    let text = String::from_utf8_lossy(xml);
    let namespace = text
        .split_once("xmlns=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(ns, _)| ns)
        .ok_or("pain.001 has no namespace")?;
    Ok(ParsedPain {
        msg_name: namespace
            .rsplit(':')
            .next()
            .unwrap_or(namespace)
            .to_string(),
        msg_id: find("MsgId", &|p| ends(p, &["GrpHdr", "MsgId"]))?,
        pmt_inf_id: find("PmtInfId", &|p| ends(p, &["PmtInf", "PmtInfId"]))?,
        exec_date: find("ReqdExctnDt", &|p| has(p, "ReqdExctnDt"))?,
        end_to_end_id: find("EndToEndId", &|p| ends(p, &["PmtId", "EndToEndId"]))?,
        payee_name: find("Cdtr/Nm", &|p| ends(p, &["Cdtr", "Nm"]))?,
        payee_iban: find("CdtrAcct/IBAN", &|p| {
            has(p, "CdtrAcct") && ends(p, &["IBAN"])
        })?,
        amount: Decimal::from_str(&amount).map_err(|e| format!("InstdAmt {amount:?}: {e}"))?,
        instant: texts
            .iter()
            .any(|(p, t)| has(p, "LclInstrm") && ends(p, &["Cd"]) && t == "INST"),
    })
}

/// An order segment of the request, whatever its version.
struct Parts {
    version: u16,
    account: Kti1,
    descriptor: String,
    message: Vec<u8>,
    /// HKIPZ 2 "Umwandlung nach SEPA-Überweisung zulässig".
    convert: bool,
}

fn parts(code: &str, raw: &RawSegment) -> Result<Parts, String> {
    let version = version_of(raw)?;
    Ok(match (code, version) {
        ("HKCCS", 1) => {
            let s = decode::<HkCcs1>(raw)?;
            Parts {
                version,
                account: s.account,
                descriptor: s.descriptor.as_str().into(),
                message: s.message.0,
                convert: false,
            }
        }
        ("HKIPZ", 1) => {
            let s = decode::<HkIpz1>(raw)?;
            Parts {
                version,
                account: s.account,
                descriptor: s.descriptor.as_str().into(),
                message: s.message.0,
                convert: false,
            }
        }
        ("HKIPZ", 2) => {
            let s = decode::<HkIpz2>(raw)?;
            Parts {
                version,
                account: s.account,
                descriptor: s.descriptor.as_str().into(),
                message: s.message.0,
                convert: s.convert_to_sepa_allowed.is_some_and(|j| j.0),
            }
        }
        (c, v) => return Err(format!("{c} {v} is not offered by the test bank")),
    })
}

/// An order that passed the bank's checks.
struct Checked {
    code: &'static str,
    parts: Parts,
    pain: ParsedPain,
    debtor_iban: String,
    converted: bool,
}

fn order_code(code: &str) -> &'static str {
    if code == "HKCCS" { "HKCCS" } else { "HKIPZ" }
}

/// The bank's checks of a payment order. The outer error is a violation of
/// the client (the order does not fit the BPD or the pain.001 is broken),
/// the inner one the bank's rejection.
fn check_order(
    m: &BankModel,
    p: &PaymentsModel,
    done: &[Payment],
    code: &str,
    raw: &RawSegment,
) -> Result<Result<Checked, Answer>, String> {
    let parts = parts(code, raw)?;
    let (offered, formats) = match code {
        "HKCCS" => (p.hkccs, &p.hkccs_formats),
        _ => (p.hkipz_versions.contains(&parts.version), &p.hkipz_formats),
    };
    if !offered {
        return Err(format!("{code} {} is not in the BPD", parts.version));
    }
    if !formats.iter().any(|f| *f == parts.descriptor) {
        return Err(format!(
            "{code} descriptor {:?} is not in the BPD ({formats:?})",
            parts.descriptor
        ));
    }
    let account = match checked_account(m, &AccountRef::kti(&parts.account), code) {
        Ok(a) => a,
        Err(refused) => return Ok(Err(refused)),
    };
    let debtor_iban = account.iban.clone().unwrap_or_default();
    let pain = parse_pain(&parts.message)?;
    if pain.exec_date != EXECUTION_DATE {
        return Ok(Err(Answer::error(
            "9150",
            "Belegung nicht erlaubt: Ausführungsdatum muss 1999-01-01 sein",
        )));
    }
    if code == "HKIPZ" && !pain.instant {
        return Ok(Err(Answer::error(
            "9150",
            "Belegung nicht erlaubt: LclInstrm INST fehlt",
        )));
    }
    if done.iter().any(|d| d.msg_id == pain.msg_id) {
        return Ok(Err(Answer::error("9390", "Doppeleinreichung")));
    }
    let mut converted = false;
    if code == "HKIPZ" && p.instant_limit.is_some_and(|limit| pain.amount > limit) {
        if parts.version == 2 && parts.convert && p.hkipz_conversion_allowed {
            converted = true;
        } else {
            return Ok(Err(match p.instant_limit_answer {
                LimitAnswer::Code9255 => Answer::default().with(
                    Feedback::new("9255", "Echtzeitlimit überschritten")
                        .with_params(&["https://testbank.invalid/limit"]),
                ),
                LimitAnswer::Code9210 => {
                    Answer::error("9210", "Betrag zu groß für Instant Payment Zahlung")
                }
            }));
        }
    }
    Ok(Ok(Checked {
        code: order_code(code),
        parts,
        pain,
        debtor_iban,
        converted,
    }))
}

/// Executes a payment order the TAN of which is accepted (or waived) and
/// records it. Returns the order's answer.
pub(super) fn execute_payment(
    m: &BankModel,
    done: &mut Vec<Payment>,
    code: &str,
    raw: &RawSegment,
    vop_id: Option<Vec<u8>>,
) -> Result<Answer, String> {
    let p = m.payments.as_ref().ok_or("the bank offers no payments")?;
    let c = match check_order(m, p, done, code, raw)? {
        Ok(c) => c,
        Err(refused) => return Ok(refused),
    };
    let instant = c.code == "HKIPZ" && !c.converted;
    let order_id = (c.code == "HKIPZ").then(|| format!("INST{:04}", done.len() + 1));
    done.push(Payment {
        order: c.code,
        version: c.parts.version,
        debtor_iban: c.debtor_iban,
        msg_id: c.pain.msg_id,
        end_to_end_id: c.pain.end_to_end_id,
        payee_name: c.pain.payee_name,
        payee_iban: c.pain.payee_iban,
        amount: c.pain.amount,
        instant,
        converted: c.converted,
        order_id: order_id.clone(),
        vop_id,
        status_polls: 0,
    });
    let mut answer = Answer::code("0020", TEXT_OK);
    if let Some(order_id) = order_id {
        let pending = p.instant_status_after > 0;
        if c.converted {
            answer = answer.with(Feedback::new(
                "3270",
                "Auftrag wird als Standard-SEPA-Überweisung bearbeitet",
            ));
        } else if pending {
            answer = answer.with(Feedback::new(
                "3045",
                "SEPA Instant Payment Statusabfrage HKIPS veranlassen",
            ));
        }
        let status = Some(super::code(if pending && !c.converted { "3" } else { "7" }));
        let order_id = an(&order_id);
        let reply: Reply = if c.parts.version == 1 {
            HiIpz1 {
                order_id,
                sepa_c_code: None,
                status,
            }
            .into()
        } else {
            HiIpz2 {
                order_id,
                sepa_c_code: None,
                status,
            }
            .into()
        };
        answer = answer.with_data(reply);
    }
    Ok(answer)
}

/// The payee check against the registry.
fn check_payee(p: &PaymentsModel, iban: &str, name: &str) -> Check {
    let norm = |s: &str| s.replace(' ', "").to_uppercase();
    if let Some((_, reason)) = p.not_applicable.iter().find(|(i, _)| norm(i) == norm(iban)) {
        return Check::NotApplicable(reason.clone());
    }
    let Some(payee) = p.payees.iter().find(|x| norm(&x.iban) == norm(iban)) else {
        return Check::NoMatch;
    };
    if name.trim().to_lowercase() == payee.name.trim().to_lowercase() {
        return Check::Match;
    }
    fn words(s: &str) -> Vec<String> {
        let spaced: String = s
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect();
        let mut w: Vec<String> = spaced.split_whitespace().map(String::from).collect();
        w.sort();
        w
    }
    let (asked, registered) = (words(name), words(&payee.name));
    if !asked.is_empty() && asked == registered {
        return Check::Close(payee.name.clone());
    }
    // One given name missing: the registered name without one of its words
    // (not the last, the family name) is the name asked for.
    let given: Vec<&str> = payee.name.split_whitespace().collect();
    if given.len() >= 3 || (given.len() == 2 && asked.len() == 1) {
        for skip in 0..given.len().saturating_sub(1) {
            let rest: Vec<&str> = given
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, w)| *w)
                .collect();
            if words(&rest.join(" ")) == asked {
                return Check::Close(payee.name.clone());
            }
        }
    }
    Check::NoMatch
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The VoP status report of a case, pain.002.001.10 (Anlage 3, 2.2.6): the
/// original message and its result per status in `OrgnlGrpInfAndSts`, one
/// `TxInfAndSts` with `OrgnlEndToEndId`, `TxSts`, `StsRsnInf` and
/// `OrgnlTxRef`. The registered name of RVMC is the first `AddtlInf`, the
/// reason of RVNA is `Rsn/Cd` AG03 with the free text in `AddtlInf`
/// (Anlage 3, pages 249-252). One place, so the layout is easy to change.
fn pain002(v: &VopModel, case: &PaymentCase, status: Option<&Check>) -> Vec<u8> {
    let sts = status.map_or("PDNG", Check::status);
    let listed = !v.block_status_only && !(matches!(status, Some(Check::Match)) && !v.list_matches);
    let reason = match status {
        Some(Check::Close(name)) => {
            format!(
                "<StsRsnInf><AddtlInf>{}</AddtlInf></StsRsnInf>",
                escape(name)
            )
        }
        Some(Check::NotApplicable(text)) => format!(
            "<StsRsnInf><Rsn><Cd>AG03</Cd></Rsn><AddtlInf>{}</AddtlInf></StsRsnInf>",
            escape(text)
        ),
        _ => String::new(),
    };
    let counts = if v.block_status_only {
        String::new()
    } else {
        format!(
            "<NbOfTxsPerSts><DtldNbOfTxs>1</DtldNbOfTxs><DtldSts>{sts}</DtldSts></NbOfTxsPerSts>"
        )
    };
    let tx = if listed {
        format!(
            "<TxInfAndSts>\
<OrgnlEndToEndId>{e2e}</OrgnlEndToEndId><TxSts>{sts}</TxSts>{reason}\
<OrgnlTxRef><Cdtr><Pty><Nm>{name}</Nm></Pty></Cdtr>\
<CdtrAcct><Id><IBAN>{iban}</IBAN></Id></CdtrAcct></OrgnlTxRef>\
</TxInfAndSts>",
            e2e = escape(&case.end_to_end_id),
            name = escape(&case.payee_name),
            iban = escape(&case.payee_iban),
        )
    } else {
        String::new()
    };
    let block = if listed || v.block_status_only {
        format!(
            "<OrgnlPmtInfAndSts><OrgnlPmtInfId>{pmt}</OrgnlPmtInfId>{pmt_sts}{tx}</OrgnlPmtInfAndSts>",
            pmt = escape(&case.pmt_inf_id),
            pmt_sts = if v.block_status_only {
                format!("<PmtInfSts>{sts}</PmtInfSts>")
            } else {
                String::new()
            },
        )
    } else {
        String::new()
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
<Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:pain.002.001.10\"><CstmrPmtStsRpt>\
<GrpHdr><MsgId>VOP{msg}</MsgId><CreDtTm>2026-01-01T12:00:00</CreDtTm></GrpHdr>\
<OrgnlGrpInfAndSts><OrgnlMsgId>{msg}</OrgnlMsgId><OrgnlMsgNmId>{name}</OrgnlMsgNmId>\
{counts}</OrgnlGrpInfAndSts>{block}</CstmrPmtStsRpt></Document>",
        msg = escape(&case.msg_id),
        name = escape(&case.msg_name),
    )
    .into_bytes()
}

/// HIVPP of a case: pending (polling ID, wait, with `S` a partial result)
/// or final (VOP-ID, the result).
fn hivpp(v: &VopModel, case: &PaymentCase, pending: bool) -> Reply {
    let status = (!pending).then_some(&case.check);
    // Only the final answer, or a part with delivery S, carries a result.
    let with_result =
        (!pending && !v.empty_result) || (pending && v.delivery == Delivery::Stepwise);
    let (report_descriptor, report, result) = match (with_result, v.result_as) {
        (false, _) => (None, None, None),
        (true, ResultAs::Pain002) => (
            Some(an(&case.descriptor)),
            Some(Bin(pain002(v, case, status))),
            None,
        ),
        (true, ResultAs::SingleDeg) => {
            let (outcome, name, rvna) = match status {
                None => ("PDNG", None, None),
                Some(Check::Close(n)) => ("RVMC", Some(an(n)), None),
                Some(Check::NotApplicable(t)) => ("RVNA", None, Some(an(t))),
                Some(c) => (c.status(), None, None),
            };
            (
                None,
                None,
                Some(PayeeCheckResult1 {
                    iban: an(&case.payee_iban),
                    iban_info: None,
                    registered_name: name,
                    other_id: None,
                    outcome: code(outcome),
                    rvna_reason: rvna,
                }),
            )
        }
    };
    let valid_until = Tsp1 {
        date: Dat::new(jiff::civil::date(2026, 1, 1)).expect("date"),
        time: Some(Tim::new(jiff::civil::time(12, 15, 0, 0)).expect("time")),
    };
    HiVpp1 {
        vop_id: case.vop_id.clone().filter(|_| !pending).map(Bin),
        vop_id_valid_until: (!pending).then_some(valid_until),
        polling_id: case.polling_id.clone().filter(|_| pending).map(Bin),
        report_descriptor,
        report,
        result,
        explanation: v
            .explanation
            .as_deref()
            .filter(|_| !pending && !case.check.clean())
            .map(an),
        wait: pending.then(|| num(u64::from(v.wait_secs))),
    }
    .into()
}

fn pending_answer(v: &VopModel, case: &PaymentCase) -> Answer {
    let point = case.touchdown.as_deref().unwrap_or_default();
    Answer::default()
        .with(Feedback::new(
            "3093",
            "Namensabgleich ist noch in Bearbeitung",
        ))
        .with(Feedback::new("3040", "Es liegen weitere Informationen vor").with_params(&[point]))
        .with_data(hivpp(v, case, true))
}

fn final_answer(v: &VopModel, case: &PaymentCase, codes: &[(&str, &str)]) -> Answer {
    let mut answer = Answer::default();
    for (c, t) in codes {
        answer = answer.with(Feedback::new(c, t));
    }
    answer.with_data(hivpp(v, case, false))
}

const NO_DEVIATION: (&str, &str) = ("0025", "Keine Namensabweichung");
const NO_RESULT: (&str, &str) = ("3095", "Kein Namensabgleich-Ergebnis vorhanden");
const CHECK_RESULT: (&str, &str) = ("3090", "Ergebnis Namensabgleich prüfen");
const NOT_NEEDED: (&str, &str) = ("3091", "VOP-Ausführungsauftrag nicht benötigt");
const COMPLETE: (&str, &str) = ("3094", "Namensabgleich ist komplett");

/// The code of a final check result: 3090 for a deviation (or a match
/// the model flags), 3095 for an empty result, else 0025.
fn result_code(v: &VopModel, case: &PaymentCase) -> (&'static str, &'static str) {
    if !case.check.clean() || v.deviation_on_match {
        CHECK_RESULT
    } else if v.empty_result {
        NO_RESULT
    } else {
        NO_DEVIATION
    }
}

fn unknown_vop_id() -> Answer {
    Answer::error("9010", "VOP-ID unbekannt oder abgelaufen")
}

/// The parameter segments of the payments model: HICCSS, HIIPZS, HIIPSS,
/// HIVPPS and HIVPAS (HISPAS is written by the BPD).
pub(super) fn parameters(p: &PaymentsModel) -> Vec<Reply> {
    let one = || num(1u64);
    let mut out: Vec<Reply> = Vec::new();
    if p.hkccs {
        out.push(
            HiCcss1 {
                max_orders: one(),
                min_signatures: one(),
                security_class: code("0"),
            }
            .into(),
        );
    }
    let formats = |f: &[&str]| f.iter().map(|f| an(f)).collect::<Vec<_>>();
    for v in &p.hkipz_versions {
        out.push(match v {
            1 => HiIpzs1 {
                max_orders: one(),
                min_signatures: one(),
                security_class: code("0"),
                parameters: InstantPaymentParameters1 {
                    purpose_codes: p.hkipz_purpose_codes.map(an),
                    formats: formats(&p.hkipz_formats),
                },
            }
            .into(),
            2 => HiIpzs2 {
                max_orders: one(),
                min_signatures: one(),
                security_class: code("0"),
                parameters: InstantPaymentParameters2 {
                    conversion_allowed: Jn(p.hkipz_conversion_allowed),
                    purpose_codes: p.hkipz_purpose_codes.map(an),
                    formats: formats(&p.hkipz_formats),
                },
            }
            .into(),
            v => panic!("the test bank has no HIIPZS {v}"),
        });
    }
    if let Some(s) = &p.hkips {
        out.push(
            HiIpss1 {
                max_orders: one(),
                min_signatures: one(),
                security_class: code("0"),
                parameters: InstantStatusParameters1 {
                    min_wait: num(u64::from(s.min_wait)),
                    formats: formats(&s.formats),
                },
            }
            .into(),
        );
    }
    if let Some(v) = &p.vop {
        out.push(
            HiVpps1 {
                max_orders: one(),
                min_signatures: one(),
                security_class: code("0"),
                parameters: PayeeCheckParameters1 {
                    max_transactions: num(v.max_transactions),
                    explanation_structured: Jn(v.structured_text),
                    delivery: code(match v.delivery {
                        Delivery::Stepwise => "S",
                        Delivery::Complete => "V",
                    }),
                    bulk_with_one_order_allowed: Jn(false),
                    max_entries_input_allowed: Jn(false),
                    report_formats: an(&v.psr_formats.join(";")),
                    required_for: v.required_orders.iter().map(|o| an(o)).collect(),
                },
            }
            .into(),
        );
        out.push(
            HiVpas1 {
                max_orders: one(),
                min_signatures: one(),
                security_class: code("0"),
            }
            .into(),
        );
    }
    out
}

/// Whether the model waives the TAN for a payment order.
fn tan_waived(m: &BankModel, code: &str) -> bool {
    matches!(
        m.jobs.get(code).map(|j| j.server_tan),
        Some(TanRule::Exempt | TanRule::RequiredExempt)
    )
}

impl State {
    /// A message with a payment segment (HKVPP, HKVPA, HKCCS, HKIPZ, HKIPS).
    pub(super) fn payment_message(
        &mut self,
        m: &BankModel,
        req: &Request,
        dialog: &mut DialogState,
        tan: Option<&Hktan>,
    ) -> Result<Vec<(u16, Answer)>, String> {
        let p = m.payments.as_ref().ok_or("the bank offers no payments")?;
        let present: Vec<&str> = req
            .codes()
            .into_iter()
            .filter(|c| !matches!(*c, "HNSHK" | "HNSHA" | "HKTAN"))
            .collect();
        if req.has("HKIPS") {
            if present != ["HKIPS"] {
                return Err(format!("HKIPS must be alone (with HKTAN), got {present:?}"));
            }
            return self.instant_status(m, p, req, dialog, tan);
        }
        if let Some(t) = tan
            && t.process != "4"
        {
            if present != ["HKVPA"] {
                return Err(format!(
                    "HKTAN process {} with {present:?}: only HKVPA may come with it",
                    t.process
                ));
            }
            return self.vpa_with_tan(m, req, t, dialog);
        }
        let order = ["HKCCS", "HKIPZ"].into_iter().find(|c| present.contains(c));
        let Some(order) = order else {
            if present != ["HKVPP"] || tan.is_some() {
                return Err(format!("{present:?} is not a payment message of the bank"));
            }
            return self.vop_poll(m, p, req, dialog);
        };
        let expected: Vec<&str> = match (req.has("HKVPP"), req.has("HKVPA")) {
            (true, false) => vec!["HKVPP", order],
            (false, true) => vec!["HKVPA", order],
            (false, false) => vec![order],
            (true, true) => return Err("HKVPP and HKVPA in one message".into()),
        };
        if present != expected {
            return Err(format!("segments {present:?}, expected {expected:?}"));
        }
        let (order_no, raw) = req.find(order).ok_or("order unreadable")?;
        let Some(t) = tan else {
            return Ok(vec![(order_no, Answer::error("9075", TEXT_SCA_REQUIRED))]);
        };
        if t.segment_code.as_deref() != Some(order) {
            return Err(format!(
                "HKTAN names {:?}, it must name the order {order}, never HKVPP",
                t.segment_code
            ));
        }
        let required = p
            .vop
            .as_ref()
            .is_some_and(|v| v.required_orders.contains(&order));
        if req.has("HKVPA") {
            return self.execution_order(m, p, req, dialog, t, (order_no, raw), order);
        }
        if req.has("HKVPP") {
            let v = p
                .vop
                .as_ref()
                .filter(|_| required)
                .ok_or_else(|| format!("HKVPP, but the BPD lists no HIVPPS for {order}"))?;
            return self.check_and_order(m, p, v, req, dialog, t, (order_no, raw), order);
        }
        if required {
            return Ok(vec![(
                order_no,
                Answer::error("9076", "Namensabgleich erforderlich"),
            )]);
        }
        if let Err(refused) = check_order(m, p, &self.payments, order, &raw)? {
            return Ok(vec![(order_no, refused)]);
        }
        self.start_order(m, dialog, t, (order_no, raw), order, None)
    }

    /// Message 1 of a payee check: `HKVPP + order + HKTAN:4`.
    #[allow(clippy::too_many_arguments)]
    fn check_and_order(
        &mut self,
        m: &BankModel,
        p: &PaymentsModel,
        v: &VopModel,
        req: &Request,
        dialog: &mut DialogState,
        t: &Hktan,
        (order_no, raw): (u16, RawSegment),
        order: &str,
    ) -> Result<Vec<(u16, Answer)>, String> {
        let (vpp_no, _) = req.find("HKVPP").ok_or("HKVPP missing")?;
        let vpp: HkVpp1 = req.get().ok_or("HKVPP unreadable")?;
        if vpp.polling_id.is_some() || vpp.touchdown.is_some() || vpp.max_entries.is_some() {
            return Err("HKVPP of message 1 carries a polling ID, touchdown or entry count".into());
        }
        let descriptor = vpp
            .reports
            .descriptors
            .iter()
            .find(|d| v.psr_formats.contains(&d.as_str()))
            .ok_or_else(|| {
                format!(
                    "HKVPP asks for {:?}, the BPD lists {:?}",
                    vpp.reports.descriptors, v.psr_formats
                )
            })?
            .as_str()
            .to_string();
        let checked = match check_order(m, p, &self.payments, order, &raw)? {
            Ok(c) => c,
            Err(refused) => return Ok(vec![(order_no, refused)]),
        };
        self.next_vop += 1;
        let mut case = PaymentCase {
            code: checked.code,
            pain: checked.parts.message.clone(),
            msg_name: checked.pain.msg_name.clone(),
            msg_id: checked.pain.msg_id.clone(),
            pmt_inf_id: checked.pain.pmt_inf_id.clone(),
            end_to_end_id: checked.pain.end_to_end_id.clone(),
            payee_name: checked.pain.payee_name.clone(),
            payee_iban: checked.pain.payee_iban.clone(),
            descriptor,
            check: check_payee(p, &checked.pain.payee_iban, &checked.pain.payee_name),
            polling_id: None,
            touchdown: None,
            answers_given: 1,
            vop_id: None,
        };
        if v.pending_polls >= 1 {
            case.polling_id = v
                .polling_id
                .then(|| format!("POLL{:04}", self.next_vop).into_bytes());
            case.touchdown = Some("VP-1".into());
            let answer = pending_answer(v, &case);
            dialog.payment = Some(case);
            return Ok(vec![(vpp_no, answer)]);
        }
        let vop_id = format!("VOPID{:04}", self.next_vop).into_bytes();
        case.vop_id = Some(vop_id.clone());
        let mut codes = vec![result_code(v, &case)];
        let notice = v.message_notice;
        if case.check.clean() && v.hkvpa_on_match == OnMatch::Skip3091 {
            codes.push(NOT_NEEDED);
        }
        let mut vpp_answer = final_answer(v, &case, &codes);
        if let Some((c, t)) = notice {
            // Derived 3060 (the result and 3945 are warnings) plus the notice.
            vpp_answer.message = vec![Feedback::new("3060", MESSAGE_WARNINGS), Feedback::new(c, t)];
        }
        let clean = case.check.clean();
        dialog.payment = Some(case);
        let mut out = vec![(vpp_no, vpp_answer)];
        if (!clean && !v.challenge_on_deviation) || v.hkvpa_on_match == OnMatch::Always3945 {
            out.push((
                t.number,
                Answer::code("3945", "Freigabe kann nicht erteilt werden"),
            ));
            return Ok(out);
        }
        let expect_vpa = (v.hkvpa_on_match == OnMatch::Require).then_some(vop_id);
        out.extend(self.start_order(m, dialog, t, (order_no, raw), order, expect_vpa)?);
        Ok(out)
    }

    /// HKVPP alone: the next poll of a pending check.
    fn vop_poll(
        &mut self,
        _m: &BankModel,
        p: &PaymentsModel,
        req: &Request,
        dialog: &mut DialogState,
    ) -> Result<Vec<(u16, Answer)>, String> {
        let v = p.vop.as_ref().ok_or("the bank offers no payee check")?;
        let (vpp_no, _) = req.find("HKVPP").ok_or("HKVPP missing")?;
        let vpp: HkVpp1 = req.get().ok_or("HKVPP unreadable")?;
        let case = dialog
            .payment
            .as_mut()
            .ok_or("HKVPP alone without a payee check in this dialog")?;
        if case.vop_id.is_some() {
            return Err("HKVPP polls a payee check that is complete".into());
        }
        // VoP C.10.7.1.1 HKVPP field 3: the polling ID only next to a
        // touchdown point.
        let expected = case
            .polling_id
            .as_ref()
            .filter(|_| case.touchdown.is_some());
        if vpp.polling_id.as_ref().map(|b| &b.0) != expected {
            return Err(format!(
                "HKVPP polling ID {:?}, expected {expected:?}",
                vpp.polling_id
            ));
        }
        if vpp.touchdown.as_ref().map(|t| t.as_str()) != case.touchdown.as_deref() {
            return Err(format!(
                "HKVPP touchdown {:?}, the bank issued {:?}",
                vpp.touchdown, case.touchdown
            ));
        }
        case.answers_given += 1;
        if case.answers_given <= v.pending_polls {
            case.touchdown = Some(format!("VP-{}", case.answers_given));
            return Ok(vec![(vpp_no, pending_answer(v, case))]);
        }
        self.next_vop += 1;
        case.vop_id = Some(format!("VOPID{:04}", self.next_vop).into_bytes());
        case.touchdown = None;
        let result = result_code(v, case);
        Ok(vec![(vpp_no, final_answer(v, case, &[COMPLETE, result]))])
    }

    /// `HKVPA + order + HKTAN:4`: the execution order after the check.
    #[allow(clippy::too_many_arguments)]
    fn execution_order(
        &mut self,
        m: &BankModel,
        p: &PaymentsModel,
        req: &Request,
        dialog: &mut DialogState,
        t: &Hktan,
        (order_no, raw): (u16, RawSegment),
        order: &str,
    ) -> Result<Vec<(u16, Answer)>, String> {
        let (vpa_no, _) = req.find("HKVPA").ok_or("HKVPA missing")?;
        let vpa: HkVpa1 = req.get().ok_or("HKVPA unreadable")?;
        let known = dialog
            .payment
            .as_ref()
            .filter(|c| c.vop_id.as_ref() == Some(&vpa.vop_id.0));
        let Some(case) = known else {
            return Ok(vec![(vpa_no, unknown_vop_id())]);
        };
        if self
            .payments
            .iter()
            .any(|d| d.vop_id.as_ref() == Some(&vpa.vop_id.0))
        {
            return Ok(vec![(
                vpa_no,
                Answer::error("9210", "Auftrag bereits ausgeführt"),
            )]);
        }
        let same = parts(order, &raw)
            .map(|o| o.message == case.pain && case.code == order)
            .unwrap_or(false);
        if !same {
            return Ok(vec![(
                order_no,
                Answer::error("9010", "Auftrag weicht vom Ursprungsauftrag ab"),
            )]);
        }
        if let Err(refused) = check_order(m, p, &self.payments, order, &raw)? {
            return Ok(vec![(order_no, refused)]);
        }
        let mut out = Vec::new();
        if let Some((c, t)) = p.vop.as_ref().and_then(|v| v.vpa_answer) {
            out.push((vpa_no, Answer::code(c, t)));
        }
        let order_answers = self.start_order(m, dialog, t, (order_no, raw), order, None)?;
        let on_order = p.vop.as_ref().and_then(|v| v.order_answer_after_vpa);
        for (no, mut answer) in order_answers {
            if let (true, Some((c, text))) = (no == order_no, on_order) {
                for f in answer.segment.iter_mut().filter(|f| f.code == "0020") {
                    *f = Feedback::new(c, text);
                }
            }
            out.push((no, answer));
        }
        Ok(out)
    }

    /// `HKVPA + HKTAN:2/S`: the TAN of an order the check has released.
    fn vpa_with_tan(
        &mut self,
        m: &BankModel,
        req: &Request,
        t: &Hktan,
        dialog: &mut DialogState,
    ) -> Result<Vec<(u16, Answer)>, String> {
        let (vpa_no, _) = req.find("HKVPA").ok_or("HKVPA missing")?;
        let vpa: HkVpa1 = req.get().ok_or("HKVPA unreadable")?;
        let pending = dialog
            .pending
            .as_mut()
            .ok_or("HKVPA with a TAN, but no TAN is pending")?;
        match &pending.vpa_expected {
            None => {
                return Err("HKVPA with the TAN, but the bank waived it or has it already".into());
            }
            Some(id) if *id != vpa.vop_id.0 => return Ok(vec![(vpa_no, unknown_vop_id())]),
            Some(_) => pending.vpa_expected = None,
        }
        continue_tan(m, req, t, dialog, &mut self.payments)
    }

    /// The TAN challenge of a released order, or its execution when the
    /// model waives the TAN. `expect_vpa` is the VOP-ID the first TAN
    /// message must carry in an HKVPA.
    fn start_order(
        &mut self,
        m: &BankModel,
        dialog: &mut DialogState,
        t: &Hktan,
        (order_no, raw): (u16, RawSegment),
        order: &str,
        expect_vpa: Option<Vec<u8>>,
    ) -> Result<Vec<(u16, Answer)>, String> {
        // A waiver holds at HKVPP time too: comdirect executed after HKVPA
        // without a TAN (2026-11-18); a clean match without the HKVPA round
        // is unobserved, the model assumes the same.
        if tan_waived(m, order) {
            let vop_id = dialog.payment.as_ref().and_then(|c| c.vop_id.clone());
            let answer = execute_payment(m, &mut self.payments, order, &raw, vop_id)?;
            return Ok(vec![(order_no, answer), (t.number, exemption(t.version))]);
        }
        let method = method_for(m, &dialog.security_function)?;
        let (answer, mut pending) =
            self.challenge(method, t.version, Some((order.to_string(), raw)));
        pending.vpa_expected = expect_vpa;
        dialog.pending = Some(pending);
        if m.payments.as_ref().is_some_and(|p| p.tan_codes_on_order) {
            let mut on_tan = answer;
            let on_order = Answer {
                segment: std::mem::take(&mut on_tan.segment),
                ..Answer::default()
            };
            return Ok(vec![(order_no, on_order), (t.number, on_tan)]);
        }
        Ok(vec![(t.number, answer)])
    }

    /// HKIPS: the status of an instant payment.
    fn instant_status(
        &mut self,
        m: &BankModel,
        p: &PaymentsModel,
        req: &Request,
        dialog: &mut DialogState,
        tan: Option<&Hktan>,
    ) -> Result<Vec<(u16, Answer)>, String> {
        let s = p.hkips.as_ref().ok_or("HKIPS is not in the BPD")?;
        let (no, raw) = req.find("HKIPS").ok_or("HKIPS missing")?;
        let ips: HkIps1 = req.get().ok_or("HKIPS unreadable")?;
        if let Some(t) = tan
            && t.process != "4"
        {
            return Err("HKTAN with HKIPS must be process 4".into());
        }
        for d in &ips.formats.descriptors {
            if !s.formats.is_empty() && !s.formats.iter().any(|f| *f == d.as_str()) {
                return Err(format!("HKIPS asks for {d}, the BPD lists {:?}", s.formats));
            }
        }
        let mut out = Vec::new();
        // A TAN for HKIPS (HIPINS `J` or a required rule): the answer
        // comes with the TAN, like for any other order.
        if let (Some(t), Some(job)) = (tan, m.jobs.get("HKIPS"))
            && super::needs_tan(job.server_tan, job)
        {
            let method = super::method_for(m, &dialog.security_function)?;
            let (answer, pending) =
                self.challenge(method, t.version, Some(("HKIPS".to_string(), raw)));
            dialog.pending = Some(pending);
            return Ok(vec![(t.number, answer)]);
        }
        out.push((no, instant_status_answer(m, p, &mut self.payments, &ips)));
        if let Some(t) = tan {
            out.push((t.number, exemption(t.version)));
        }
        Ok(out)
    }
}

/// The bank's answer to an HKIPS: `3045` until the model's n-th query, then
/// HIIPS with status 7.
pub(super) fn instant_status_answer(
    m: &BankModel,
    p: &PaymentsModel,
    payments: &mut [Payment],
    ips: &HkIps1,
) -> Answer {
    match checked_account(m, &AccountRef::kti(&ips.account), "HKIPS") {
        Err(refused) => refused,
        Ok(_) => match payments
            .iter_mut()
            .find(|d| d.order_id.as_deref() == Some(ips.order_id.as_str()))
        {
            None => Answer::error("9010", "Auftrag unbekannt"),
            Some(d) => {
                d.status_polls += 1;
                if d.status_polls < p.instant_status_after {
                    Answer::code(
                        "3045",
                        "SEPA Instant Payment Statusabfrage HKIPS veranlassen",
                    )
                } else {
                    Answer::ok(vec![
                        HiIps1 {
                            account: ips.account.clone(),
                            descriptor: None,
                            message: None,
                            order_id: ips.order_id.clone(),
                            sepa_c_code: None,
                            status: Some(code("7")),
                        }
                        .into(),
                    ])
                }
            }
        },
    }
}
