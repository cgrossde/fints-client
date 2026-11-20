//! The payee check (Verification of Payee, VoP) result of a transfer: the
//! types the caller sees, the pain.002 formats HKVPP asks for, and reading
//! one result from an HIVPP (pain.002 parts or the single-transaction DEG;
//! docs/payments-design.md sections 3 and 5.3).

use serde::{Deserialize, Serialize};

use crate::codec::An;
use crate::codec::defs::HiVpp1;
use crate::error::{FinTSError, Result};
use crate::formats::pain002::{self, SentTx};
use crate::protocol::VopParams;
use crate::types::Iban;

/// What the bank reported for the payees of one order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VopResult {
    /// One entry per transaction; one for a single transfer. Empty when the
    /// bank delivered no result (3095, VoP C.10.7.1.1: "HIVPP ohne
    /// befüllte VOP-Prüfergebnisse").
    pub payees: Vec<PayeeResult>,
    /// Notes for the payer from the report (pain.002 group
    /// `StsRsnInf/AddtlInf`, Anlage 3 2.2.6.4), each line starting with the
    /// status it explains (`RVNM …`). Empty for the HIVPP DEG, whose
    /// explanation text is a separate HIVPP field.
    pub notes: Vec<String>,
}

impl VopResult {
    /// No payees, no notes.
    pub fn empty() -> Self {
        Self {
            payees: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Whether some payee still waits for the payee's bank (`PDNG`).
    pub fn is_pending(&self) -> bool {
        self.payees.iter().any(|p| p.outcome.is_pending())
    }

    /// Whether every payee matched (`RCVC`); true for an empty result.
    pub fn all_match(&self) -> bool {
        self.payees.iter().all(|p| p.outcome.is_match())
    }
}

/// The check of one payee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayeeResult {
    /// Normalised (upper case, no spaces).
    pub iban: String,
    /// The name sent in the order.
    pub requested_name: String,
    pub outcome: PayeeOutcome,
}

/// Result of one payee check (Anlage 3 2.2.6.7; VoP D "VOP-Prüfergebnis").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayeeOutcome {
    /// `RCVC`: the name matches the account holder.
    Match,
    /// `RVMC`: the name matches partly; the registered name when the bank
    /// gives it.
    CloseMatch { registered_name: Option<String> },
    /// `RVNM`: the name does not match.
    NoMatch,
    /// `RVNA`: no check possible (not a payment account, payee bank outside
    /// VoP, timeout); the bank's reason when given.
    NotApplicable { reason: Option<String> },
    /// `PDNG`: the payee's bank has not answered yet (only in an unfinished
    /// result).
    Pending,
    /// Another identifier than the name was checked (e.g. LEI): its name
    /// and the outcome for it.
    OtherId {
        id_name: String,
        outcome: Box<PayeeOutcome>,
    },
}

impl PayeeOutcome {
    pub fn is_pending(&self) -> bool {
        match self {
            Self::Pending => true,
            Self::OtherId { outcome, .. } => outcome.is_pending(),
            _ => false,
        }
    }

    pub fn is_match(&self) -> bool {
        match self {
            Self::Match => true,
            Self::OtherId { outcome, .. } => outcome.is_match(),
            _ => false,
        }
    }

    /// From a status code and its detail (RVMC: registered name, RVNA:
    /// reason). `None` for codes that are not transaction results.
    fn from_code(code: &str, detail: Option<String>) -> Option<Self> {
        Some(match code {
            "RCVC" => Self::Match,
            "RVMC" => Self::CloseMatch {
                registered_name: detail,
            },
            "RVNM" => Self::NoMatch,
            "RVNA" => Self::NotApplicable { reason: detail },
            "PDNG" => Self::Pending,
            _ => return None,
        })
    }
}

/// The pain.002 descriptors HKVPP asks for: those of the HIVPPS list that
/// name pain.002.001.10 (the version the reader knows), else the whole list
/// (the bank may still answer with the DEG).
pub(super) fn choose_report_formats(params: &VopParams) -> Vec<String> {
    let readable: Vec<String> = params
        .report_formats
        .iter()
        .filter(|f| f.contains("pain.002.001.10"))
        .cloned()
        .collect();
    if readable.is_empty() {
        params.report_formats.clone()
    } else {
        readable
    }
}

/// The transfer a result answers.
#[derive(Debug, Clone, Copy)]
pub(super) struct Sent<'a> {
    /// pain.001 `MsgId`.
    pub message_id: &'a str,
    pub end_to_end_id: &'a str,
    pub iban: &'a Iban,
    pub name: &'a str,
}

/// The result of a final HIVPP: the pain.002 `parts` received so far (the
/// last one with complete delivery, all merged with `stepwise` delivery),
/// else the HIVPP single-transaction DEG. `None` when the bank sent
/// neither.
pub(super) fn read_result(
    hivpp: &HiVpp1,
    parts: &[Vec<u8>],
    stepwise: bool,
    sent: &Sent<'_>,
) -> Result<Option<VopResult>> {
    if let Some(last) = parts.last() {
        let parts: Vec<&[u8]> = if stepwise {
            parts.iter().map(Vec::as_slice).collect()
        } else {
            vec![last.as_slice()]
        };
        let tx = [SentTx {
            end_to_end_id: sent.end_to_end_id,
            iban: sent.iban.as_str(),
            name: sent.name,
        }];
        let report = pain002::read(&parts, sent.message_id, &tx)?;
        let payees = report
            .transactions
            .into_iter()
            .map(|t| PayeeResult {
                outcome: PayeeOutcome::from_code(t.status, t.detail)
                    .expect("pain002 returns known statuses"),
                iban: t.iban,
                requested_name: t.requested_name,
            })
            .collect();
        return Ok(Some(VopResult {
            payees,
            notes: report.notes,
        }));
    }
    let Some(deg) = &hivpp.result else {
        return Ok(None);
    };
    let text = |a: &Option<An>| {
        a.as_ref()
            .map(|a| a.as_str().trim())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let code = deg.outcome.as_str().trim();
    let detail = match code {
        "RVMC" => text(&deg.registered_name),
        "RVNA" => text(&deg.rvna_reason),
        _ => None,
    };
    let outcome = PayeeOutcome::from_code(code, detail).ok_or_else(|| {
        FinTSError::InvalidResponse(format!("HIVPP: unknown VOP result {code:?}"))
    })?;
    let outcome = match text(&deg.other_id) {
        Some(id_name) => PayeeOutcome::OtherId {
            id_name,
            outcome: Box::new(outcome),
        },
        None => outcome,
    };
    let iban: String = deg
        .iban
        .as_str()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    if iban.is_empty() {
        return Err(FinTSError::InvalidResponse(
            "HIVPP: VOP result without IBAN".into(),
        ));
    }
    Ok(Some(VopResult {
        payees: vec![PayeeResult {
            iban,
            requested_name: sent.name.to_string(),
            outcome,
        }],
        notes: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::PayeeCheckResult1;
    use crate::codec::{Bin, Code};

    const MSG: &str = "FC20261117091530ABCDEFGH";

    fn iban() -> Iban {
        Iban::parse("DE89370400440532013000").unwrap()
    }

    fn hivpp(result: Option<PayeeCheckResult1>) -> HiVpp1 {
        HiVpp1 {
            vop_id: Some(Bin(b"VOPID".to_vec())),
            vop_id_valid_until: None,
            polling_id: None,
            report_descriptor: None,
            report: None,
            result,
            explanation: None,
            wait: None,
        }
    }

    fn deg(outcome: &str) -> PayeeCheckResult1 {
        PayeeCheckResult1 {
            iban: An::new("DE89370400440532013000").unwrap(),
            iban_info: None,
            registered_name: None,
            other_id: None,
            outcome: Code::new(outcome).unwrap(),
            rvna_reason: None,
        }
    }

    fn read(h: &HiVpp1, parts: &[Vec<u8>]) -> Result<Option<VopResult>> {
        let iban = iban();
        let sent = Sent {
            message_id: MSG,
            end_to_end_id: "NOTPROVIDED",
            iban: &iban,
            name: "Max Muster",
        };
        read_result(h, parts, false, &sent)
    }

    fn outcome(d: PayeeCheckResult1) -> Result<PayeeOutcome> {
        read(&hivpp(Some(d)), &[]).map(|r| r.unwrap().payees[0].outcome.clone())
    }

    #[test]
    fn deg_outcomes() {
        assert_eq!(outcome(deg("RCVC")).unwrap(), PayeeOutcome::Match);
        assert_eq!(outcome(deg("RVNM")).unwrap(), PayeeOutcome::NoMatch);
        assert_eq!(outcome(deg("PDNG")).unwrap(), PayeeOutcome::Pending);
        let mut d = deg("RVNA");
        d.rvna_reason = Some(An::new("Zeitueberschreitung").unwrap());
        assert_eq!(
            outcome(d).unwrap(),
            PayeeOutcome::NotApplicable {
                reason: Some("Zeitueberschreitung".into())
            }
        );
        assert_eq!(
            outcome(deg("RVMC")).unwrap(),
            PayeeOutcome::CloseMatch {
                registered_name: None
            }
        );
        let mut d = deg("RCVC");
        d.other_id = Some(An::new("LEI").unwrap());
        assert_eq!(
            outcome(d).unwrap(),
            PayeeOutcome::OtherId {
                id_name: "LEI".into(),
                outcome: Box::new(PayeeOutcome::Match)
            }
        );
        assert!(matches!(
            outcome(deg("XXXX")),
            Err(FinTSError::InvalidResponse(_))
        ));
        let mut d = deg("RCVC");
        d.iban = An::new("").unwrap();
        assert!(matches!(outcome(d), Err(FinTSError::InvalidResponse(_))));
        // Neither DEG nor report.
        assert_eq!(read(&hivpp(None), &[]).unwrap(), None);
    }

    /// The DEG and the pain.002 give the same result for the same answer.
    #[test]
    fn deg_and_pain_agree() {
        let mut d = deg("RVMC");
        d.registered_name = Some(An::new("Maximilian Muster").unwrap());
        let from_deg = read(&hivpp(Some(d)), &[]).unwrap();
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:pain.002.001.10"><CstmrPmtStsRpt>
<GrpHdr><MsgId>BANK-PSR-1</MsgId><CreDtTm>2026-11-17T10:15:31</CreDtTm></GrpHdr>
<OrgnlGrpInfAndSts><OrgnlMsgId>{MSG}</OrgnlMsgId><OrgnlMsgNmId>pain.001.001.09</OrgnlMsgNmId>
<GrpSts>RVCM</GrpSts></OrgnlGrpInfAndSts>
<OrgnlPmtInfAndSts><OrgnlPmtInfId>P</OrgnlPmtInfId><TxInfAndSts>
<OrgnlEndToEndId>NOTPROVIDED</OrgnlEndToEndId><TxSts>RVMC</TxSts>
<StsRsnInf><AddtlInf>Maximilian Muster</AddtlInf></StsRsnInf>
<OrgnlTxRef><Cdtr><Pty><Nm>Max Muster</Nm></Pty></Cdtr>
<CdtrAcct><Id><IBAN>DE89370400440532013000</IBAN></Id></CdtrAcct></OrgnlTxRef>
</TxInfAndSts></OrgnlPmtInfAndSts></CstmrPmtStsRpt></Document>"#
        );
        let from_pain = read(&hivpp(Some(deg("RCVC"))), &[xml.into_bytes()]).unwrap();
        assert_eq!(from_pain, from_deg, "the report wins over the DEG");
    }
}
