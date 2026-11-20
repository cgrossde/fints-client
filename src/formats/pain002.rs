//! The pain.002.001.10 VoP status report subset (docs/payments-design.md
//! section 5.3): the status the bank reported for each transaction of one
//! order. The job (`jobs::transfer::vop`) turns it into a `VopResult`.
//!
//! Sources: DFÜ-Abkommen Anlage 3 V26.11 section 2.2.6 (VOP Status Report:
//! 2.2.6.4 `OrgnlGrpInfAndSts`, 2.2.6.5 `OrgnlPmtInfAndSts`, 2.2.6.6
//! `TxInfAndSts`, 2.2.6.7 result types) and FinTS VoP V1.02 section D
//! ("Art der Lieferung Payment Status Report").
//!
//! Nothing is guessed: a document that does not parse, names another
//! message, or leaves a transaction without a status it can be read from is
//! a [`FinTSError::Pain002`].

use std::collections::BTreeMap;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{FinTSError, Result};

/// The transaction statuses of a VoP status report (Anlage 3 2.2.6.7):
/// match, close match, no match, not applicable, pending.
pub(crate) const STATUSES: [&str; 5] = ["RCVC", "RVMC", "RVNM", "RVNA", "PDNG"];

fn known_status(code: &str) -> Option<&'static str> {
    STATUSES.iter().copied().find(|s| *s == code)
}

/// One transaction of the order the report answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SentTx<'a> {
    /// `EndToEndId` as sent (`NOTPROVIDED` included).
    pub end_to_end_id: &'a str,
    /// Normalised (upper case, no spaces).
    pub iban: &'a str,
    pub name: &'a str,
}

/// The status of one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TxStatus {
    pub iban: String,
    /// The payee name the order carried.
    pub requested_name: String,
    /// One of [`STATUSES`].
    pub status: &'static str,
    /// RVMC: the registered name; RVNA: the reason text or code.
    pub detail: Option<String>,
}

/// What the reports of one order say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusReport {
    /// The sent transactions in order, then listed ones that answer none of
    /// them.
    pub transactions: Vec<TxStatus>,
    /// Group `StsRsnInf/AddtlInf` lines (Anlage 3 2.2.6.4), each starting
    /// with the status it explains (`RVNM …`).
    pub notes: Vec<String>,
}

fn format_err(msg: impl Into<String>) -> FinTSError {
    FinTSError::Pain002(msg.into())
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Read the VoP status report(s) of one order.
///
/// `parts` are the pain.002 documents in the order received: one for a
/// complete delivery, several for a stepwise one (HIVPPS "Art der
/// Lieferung" `S`); with either, the last status a transaction got wins.
/// `message_id` is the pain.001 `MsgId`; `sent` the transactions in order.
///
/// A sent transaction the reports do not list (Anlage 3 2.2.6.6 lists at
/// least every transaction that did not match), or one listed `PDNG` and
/// not listed again in a later part (VoP D: a stepwise part re-lists only
/// pending transactions that did not turn into a match), is a match when
/// the last part's statuses or counts confirm matches. Otherwise, for an
/// order of one transaction, the last part's `PmtInfSts`, else `GrpSts`,
/// else a count naming a single status, is its status (no registered name,
/// no reason). Anything else is a [`FinTSError::Pain002`].
pub(crate) fn read(parts: &[&[u8]], message_id: &str, sent: &[SentTx<'_>]) -> Result<StatusReport> {
    if parts.is_empty() {
        return Err(format_err("pain.002: no document"));
    }
    // Per sent transaction: the last listed status and the part it came in.
    let mut listed: Vec<Option<(ListedTx, usize)>> = vec![None; sent.len()];
    let mut extra: Vec<TxStatus> = Vec::new();
    let mut notes = Vec::new();
    let mut last = None;
    for (k, xml) in parts.iter().enumerate() {
        let report = Report::parse(xml).map_err(|e| format_err(format!("pain.002: {e}")))?;
        if report.original_message_id != message_id {
            return Err(format_err(format!(
                "pain.002 answers message {:?}, not {message_id:?}",
                report.original_message_id
            )));
        }
        for tx in &report.transactions {
            match find_sent(sent, tx) {
                Some(i) => listed[i] = Some((tx.clone(), k)),
                None => {
                    let status = tx.status();
                    match extra.iter_mut().find(|c| c.iban == status.iban) {
                        Some(c) => *c = status,
                        None => extra.push(status),
                    }
                }
            }
        }
        for note in &report.notes {
            if !notes.contains(note) {
                notes.push(note.clone());
            }
        }
        last = Some(report);
    }
    let last = last.expect("parts is not empty");
    let last_part = parts.len() - 1;

    let mut inferred = 0u64;
    let mut transactions = Vec::with_capacity(sent.len() + extra.len());
    for (s, l) in sent.iter().zip(listed) {
        let (status, detail) = match l {
            Some((tx, k)) if !(tx.status == "PDNG" && k < last_part) => (tx.status, tx.detail),
            _ => {
                inferred += 1;
                ("RCVC", None)
            }
        };
        transactions.push(TxStatus {
            iban: s.iban.to_string(),
            requested_name: s.name.to_string(),
            status,
            detail,
        });
    }
    if inferred > 0 && !last.confirms_matches(inferred) {
        match (transactions.as_mut_slice(), last.single_status()) {
            ([only], Some(status)) => only.status = status,
            _ => {
                return Err(format_err(format!(
                    "pain.002 gives no status for {inferred} transaction(s) and does not report them as matching"
                )));
            }
        }
    }
    transactions.extend(extra);
    Ok(StatusReport {
        transactions,
        notes,
    })
}

/// The sent transaction a listed one answers: same `EndToEndId` and IBAN
/// (IBAN alone when the report gives no end-to-end ID).
fn find_sent(sent: &[SentTx<'_>], tx: &ListedTx) -> Option<usize> {
    let same_iban = |s: &SentTx<'_>| tx.iban.is_empty() || s.iban == tx.iban;
    sent.iter()
        .position(|s| s.end_to_end_id == tx.end_to_end_id && same_iban(s))
        .or_else(|| {
            if tx.iban.is_empty() {
                return None;
            }
            let mut by_iban = sent.iter().enumerate().filter(|(_, s)| s.iban == tx.iban);
            match (by_iban.next(), by_iban.next()) {
                (Some((i, _)), None) => Some(i),
                _ => None,
            }
        })
}

fn compact(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// One listed transaction (`TxInfAndSts`).
#[derive(Debug, Clone)]
struct ListedTx {
    end_to_end_id: String,
    iban: String,
    name: String,
    status: &'static str,
    detail: Option<String>,
}

/// One pain.002 document, reduced to what the VoP result needs.
#[derive(Debug)]
struct Report {
    original_message_id: String,
    /// `GrpSts`.
    group_status: Option<String>,
    /// Every `PmtInfSts`.
    block_statuses: Vec<String>,
    /// `OrgnlGrpInfAndSts/NbOfTxsPerSts`: status -> count.
    counts: BTreeMap<String, u64>,
    notes: Vec<String>,
    transactions: Vec<ListedTx>,
}

impl Report {
    fn parse(xml: &[u8]) -> std::result::Result<Self, String> {
        let doc = Node::parse(xml)?;
        if doc.name != "Document" {
            return Err(format!("root element is {:?}, not Document", doc.name));
        }
        let rpt = doc
            .child("CstmrPmtStsRpt")
            .ok_or("no CstmrPmtStsRpt (not a payment status report)")?;
        let grp = rpt
            .child("OrgnlGrpInfAndSts")
            .ok_or("no OrgnlGrpInfAndSts")?;
        let original_message_id = grp
            .text_at(&["OrgnlMsgId"])
            .ok_or("no OrgnlMsgId")?
            .to_string();
        let group_status = grp.text_at(&["GrpSts"]).map(str::to_string);
        let mut block_statuses = Vec::new();
        let mut counts = BTreeMap::new();
        for n in grp.children("NbOfTxsPerSts") {
            let status = n
                .text_at(&["DtldSts"])
                .ok_or("NbOfTxsPerSts without DtldSts")?;
            let count = n
                .text_at(&["DtldNbOfTxs"])
                .ok_or("NbOfTxsPerSts without DtldNbOfTxs")?;
            let count: u64 = count
                .parse()
                .map_err(|_| format!("DtldNbOfTxs {count:?} is not a number"))?;
            counts.insert(status.to_string(), count);
        }
        let notes = grp
            .children("StsRsnInf")
            .flat_map(|r| r.children("AddtlInf"))
            .map(|n| n.text.clone())
            .filter(|t| !t.is_empty())
            .collect();
        let mut transactions = Vec::new();
        for pmt in rpt.children("OrgnlPmtInfAndSts") {
            if let Some(s) = pmt.text_at(&["PmtInfSts"]) {
                block_statuses.push(s.to_string());
            }
            for tx in pmt.children("TxInfAndSts") {
                transactions.push(ListedTx::parse(tx)?);
            }
        }
        Ok(Self {
            original_message_id,
            group_status,
            block_statuses,
            counts,
            notes,
            transactions,
        })
    }

    /// Whether this report says `n` (unlisted) transactions matched.
    fn confirms_matches(&self, n: u64) -> bool {
        self.block_statuses
            .iter()
            .chain(&self.group_status)
            .any(|s| s == "RCVC")
            || self.counts.get("RCVC").is_some_and(|&c| c >= n)
    }

    /// The status of an order of one transaction that is not listed:
    /// `PmtInfSts`, else `GrpSts`, else the one status the counts name
    /// (`RVCM`, "completed with mismatches", names none).
    fn single_status(&self) -> Option<&'static str> {
        self.block_statuses
            .iter()
            .chain(&self.group_status)
            .find_map(|s| known_status(s))
            .or_else(|| {
                let mut named = self.counts.iter().filter(|(_, n)| **n > 0);
                match (named.next(), named.next()) {
                    (Some((s, _)), None) => known_status(s),
                    _ => None,
                }
            })
    }
}

impl ListedTx {
    fn parse(tx: &Node) -> std::result::Result<Self, String> {
        let end_to_end_id = tx.text_at(&["OrgnlEndToEndId"]).unwrap_or("").to_string();
        let code = tx
            .text_at(&["TxSts"])
            .ok_or_else(|| format!("transaction {end_to_end_id:?} without TxSts"))?;
        let rsn = tx.child("StsRsnInf");
        let lines: Vec<&str> = rsn
            .map(|r| r.children("AddtlInf").map(|n| n.text.as_str()).collect())
            .unwrap_or_default();
        let detail = match code {
            // The registered name, continued in the second AddtlInf when
            // longer than 105 characters (Anlage 3 2.2.6.6).
            "RVMC" => non_empty(Some(&lines.concat())),
            // Free text (AG03), else the reason code (AB11, AG03).
            "RVNA" => non_empty(lines.first().copied())
                .or_else(|| non_empty(rsn.and_then(|r| r.text_at(&["Rsn", "Cd"])))),
            _ => None,
        };
        let status = known_status(code)
            .ok_or_else(|| format!("transaction {end_to_end_id:?}: unknown status {code:?}"))?;
        let reference = tx.child("OrgnlTxRef");
        let iban = reference
            .and_then(|r| r.text_at(&["CdtrAcct", "Id", "IBAN"]))
            .map(compact)
            .unwrap_or_default();
        let name = reference
            .and_then(|r| {
                r.text_at(&["Cdtr", "Pty", "Nm"])
                    .or_else(|| r.text_at(&["Cdtr", "Nm"]))
            })
            .unwrap_or("")
            .to_string();
        Ok(Self {
            end_to_end_id,
            iban,
            name,
            status,
            detail,
        })
    }

    fn status(&self) -> TxStatus {
        TxStatus {
            iban: self.iban.clone(),
            requested_name: self.name.clone(),
            status: self.status,
            detail: self.detail.clone(),
        }
    }
}

/// A minimal element tree: local names, trimmed text, no attributes.
#[derive(Debug, Default)]
struct Node {
    name: String,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    fn text_at(&self, path: &[&str]) -> Option<&str> {
        let mut n = self;
        for p in path {
            n = n.child(p)?;
        }
        Some(n.text.as_str()).filter(|t| !t.is_empty())
    }

    fn parse(xml: &[u8]) -> std::result::Result<Node, String> {
        let mut reader = Reader::from_reader(xml);
        let mut buf = Vec::new();
        let mut stack: Vec<Node> = Vec::new();
        let mut root: Option<Node> = None;
        let at = |r: &Reader<&[u8]>| r.error_position();
        loop {
            let event = reader
                .read_event_into(&mut buf)
                .map_err(|e| format!("XML error at byte {}: {e}", at(&reader)))?;
            match event {
                Event::Start(e) => stack.push(Node {
                    name: local_name(e.name().as_ref()),
                    ..Node::default()
                }),
                Event::Empty(e) => {
                    let node = Node {
                        name: local_name(e.name().as_ref()),
                        ..Node::default()
                    };
                    attach(&mut stack, &mut root, node)?;
                }
                Event::End(_) => {
                    let mut node = stack.pop().ok_or("unbalanced end tag")?;
                    node.text = node.text.trim().to_string();
                    attach(&mut stack, &mut root, node)?;
                }
                Event::Text(t) => {
                    let s = t.decode().map_err(|e| format!("bad text: {e}"))?;
                    if let Some(n) = stack.last_mut() {
                        n.text.push_str(&s);
                    }
                }
                Event::CData(t) => {
                    if let Some(n) = stack.last_mut() {
                        n.text.push_str(&String::from_utf8_lossy(&t));
                    }
                }
                Event::GeneralRef(r) => {
                    let name = r.decode().map_err(|e| format!("bad reference: {e}"))?;
                    let resolved = match r
                        .resolve_char_ref()
                        .map_err(|e| format!("bad reference &{name};: {e}"))?
                    {
                        Some(c) => c.to_string(),
                        None => quick_xml::escape::resolve_predefined_entity(&name)
                            .ok_or_else(|| format!("unknown entity &{name};"))?
                            .to_string(),
                    };
                    if let Some(n) = stack.last_mut() {
                        n.text.push_str(&resolved);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        if !stack.is_empty() {
            return Err("document ends inside an element".into());
        }
        root.ok_or_else(|| "empty document".into())
    }
}

fn attach(
    stack: &mut [Node],
    root: &mut Option<Node>,
    node: Node,
) -> std::result::Result<(), String> {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None if root.is_none() => *root = Some(node),
        None => return Err("more than one root element".into()),
    }
    Ok(())
}

fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSG: &str = "FC20261117091530ABCDEFGH";

    fn sent() -> Vec<SentTx<'static>> {
        vec![SentTx {
            end_to_end_id: "NOTPROVIDED",
            iban: "DE89370400440532013000",
            name: "Max Muster",
        }]
    }

    fn sent_two() -> Vec<SentTx<'static>> {
        let mut s = sent();
        s.push(SentTx {
            end_to_end_id: "E2E-2",
            iban: "DE02120300000000202051",
            name: "Erika Beispiel",
        });
        s
    }

    /// A synthetic VoP status report (Anlage 3 2.2.6 layout).
    fn report(grp: &str, counts: &[(&str, u32)], notes: &[&str], txs: &[String]) -> String {
        let counts: String = counts
            .iter()
            .map(|(s, n)| {
                format!("<NbOfTxsPerSts><DtldNbOfTxs>{n}</DtldNbOfTxs><DtldSts>{s}</DtldSts></NbOfTxsPerSts>")
            })
            .collect();
        let notes: String = if notes.is_empty() {
            String::new()
        } else {
            let lines: String = notes
                .iter()
                .map(|n| format!("<AddtlInf>{n}</AddtlInf>"))
                .collect();
            format!("<StsRsnInf>{lines}</StsRsnInf>")
        };
        let grp = if grp.is_empty() {
            String::new()
        } else {
            format!("<GrpSts>{grp}</GrpSts>")
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:pain.002.001.10">
 <CstmrPmtStsRpt>
  <GrpHdr><MsgId>BANK-PSR-1</MsgId><CreDtTm>2026-11-17T10:15:31</CreDtTm>
   <DbtrAgt><FinInstnId><BICFI>BYLADEM1001</BICFI></FinInstnId></DbtrAgt></GrpHdr>
  <OrgnlGrpInfAndSts><OrgnlMsgId>{MSG}</OrgnlMsgId><OrgnlMsgNmId>pain.001.001.09</OrgnlMsgNmId>
   <OrgnlNbOfTxs>1</OrgnlNbOfTxs>{grp}{notes}{counts}</OrgnlGrpInfAndSts>
  <OrgnlPmtInfAndSts><OrgnlPmtInfId>FC20261117091530IJKLMNOP</OrgnlPmtInfId>
   <OrgnlNbOfTxs>1</OrgnlNbOfTxs>{}</OrgnlPmtInfAndSts>
 </CstmrPmtStsRpt>
</Document>"#,
            txs.concat()
        )
    }

    fn tx(e2e: &str, iban: &str, name: &str, sts: &str, rsn: &str) -> String {
        format!(
            "<TxInfAndSts><OrgnlEndToEndId>{e2e}</OrgnlEndToEndId><TxSts>{sts}</TxSts>{rsn}\
             <OrgnlTxRef><Amt><InstdAmt Ccy=\"EUR\">1.00</InstdAmt></Amt>\
             <Cdtr><Pty><Nm>{name}</Nm></Pty></Cdtr><CdtrAcct><Id><IBAN>{iban}</IBAN></Id></CdtrAcct>\
             </OrgnlTxRef></TxInfAndSts>"
        )
    }

    fn one(xml: &str) -> Result<StatusReport> {
        read(&[xml.as_bytes()], MSG, &sent())
    }

    fn status(r: &StatusReport, i: usize) -> (&'static str, Option<&str>) {
        let t = &r.transactions[i];
        (t.status, t.detail.as_deref())
    }

    #[test]
    fn match_without_listing() {
        // A full match need not be listed (Anlage 3 2.2.6.6).
        let r = one(&report("RCVC", &[("RCVC", 1)], &[], &[])).unwrap();
        assert_eq!(
            r.transactions,
            vec![TxStatus {
                iban: "DE89370400440532013000".into(),
                requested_name: "Max Muster".into(),
                status: "RCVC",
                detail: None,
            }]
        );
        // Counts alone confirm it too.
        let r = one(&report("", &[("RCVC", 1)], &[], &[])).unwrap();
        assert_eq!(status(&r, 0), ("RCVC", None));
    }

    #[test]
    fn close_match_with_registered_name() {
        let long_a = "A".repeat(105);
        let rsn = format!(
            "<StsRsnInf><AddtlInf>{long_a}</AddtlInf><AddtlInf>Rest</AddtlInf></StsRsnInf>"
        );
        let xml = report(
            "RVCM",
            &[("RVMC", 1)],
            &["RVMC Der Name weicht ab.", "RVMC Bitte pruefen."],
            &[tx(
                "NOTPROVIDED",
                "DE89370400440532013000",
                "Max Muster",
                "RVMC",
                &rsn,
            )],
        );
        let r = one(&xml).unwrap();
        let name = format!("{long_a}Rest");
        assert_eq!(status(&r, 0), ("RVMC", Some(name.as_str())));
        assert_eq!(
            r.notes,
            vec!["RVMC Der Name weicht ab.", "RVMC Bitte pruefen."]
        );
    }

    #[test]
    fn no_match_and_not_applicable() {
        let xml = report(
            "RVCM",
            &[("RVNM", 1)],
            &[],
            &[tx(
                "NOTPROVIDED",
                "DE89370400440532013000",
                "Max Muster",
                "RVNM",
                "",
            )],
        );
        assert_eq!(status(&one(&xml).unwrap(), 0), ("RVNM", None));

        let rsn = "<StsRsnInf><Rsn><Cd>AB11</Cd></Rsn></StsRsnInf>";
        let xml = report(
            "RVCM",
            &[("RVNA", 1)],
            &[],
            &[tx(
                "NOTPROVIDED",
                "DE89370400440532013000",
                "Max Muster",
                "RVNA",
                rsn,
            )],
        );
        assert_eq!(status(&one(&xml).unwrap(), 0), ("RVNA", Some("AB11")));
        let rsn = "<StsRsnInf><Rsn><Cd>AG03</Cd></Rsn><AddtlInf>Kein Zahlungskonto</AddtlInf></StsRsnInf>";
        let xml = report(
            "RVCM",
            &[("RVNA", 1)],
            &[],
            &[tx(
                "NOTPROVIDED",
                "DE89370400440532013000",
                "Max Muster",
                "RVNA",
                rsn,
            )],
        );
        assert_eq!(
            status(&one(&xml).unwrap(), 0),
            ("RVNA", Some("Kein Zahlungskonto"))
        );
    }

    #[test]
    fn stepwise_parts_merge() {
        let s = sent_two();
        // Part 1: both pending.
        let p1 = report(
            "",
            &[("PDNG", 2)],
            &[],
            &[
                tx(
                    "NOTPROVIDED",
                    "DE89370400440532013000",
                    "Max Muster",
                    "PDNG",
                    "",
                ),
                tx(
                    "E2E-2",
                    "DE02120300000000202051",
                    "Erika Beispiel",
                    "PDNG",
                    "",
                ),
            ],
        );
        let r = read(&[p1.as_bytes()], MSG, &s).unwrap();
        assert_eq!((status(&r, 0).0, status(&r, 1).0), ("PDNG", "PDNG"));
        // Part 2: the first turned into a match (not re-listed), the second
        // is still pending (re-listed).
        let p2 = report(
            "",
            &[("RCVC", 1), ("PDNG", 1)],
            &[],
            &[tx(
                "E2E-2",
                "DE02120300000000202051",
                "Erika Beispiel",
                "PDNG",
                "",
            )],
        );
        let r = read(&[p1.as_bytes(), p2.as_bytes()], MSG, &s).unwrap();
        assert_eq!((status(&r, 0).0, status(&r, 1).0), ("RCVC", "PDNG"));
        // Part 3: the second has no match.
        let p3 = report(
            "RVCM",
            &[("RCVC", 1), ("RVNM", 1)],
            &[],
            &[tx(
                "E2E-2",
                "DE02120300000000202051",
                "Erika Beispiel",
                "RVNM",
                "",
            )],
        );
        let parts = [p1.as_bytes(), p2.as_bytes(), p3.as_bytes()];
        let r = read(&parts, MSG, &s).unwrap();
        assert_eq!((status(&r, 0).0, status(&r, 1).0), ("RCVC", "RVNM"));
        // A complete delivery: the last document alone gives the same.
        let r = read(&[p3.as_bytes()], MSG, &s).unwrap();
        assert_eq!((status(&r, 0).0, status(&r, 1).0), ("RCVC", "RVNM"));
    }

    /// A single transaction that is not listed takes the block status, else
    /// the group status, else the one status the counts name.
    #[test]
    fn unlisted_single_transaction_takes_the_block_or_group_status() {
        let only_block = report("", &[], &[], &[]).replace(
            "<OrgnlNbOfTxs>1</OrgnlNbOfTxs></OrgnlPmtInfAndSts>",
            "<OrgnlNbOfTxs>1</OrgnlNbOfTxs><PmtInfSts>RVNM</PmtInfSts></OrgnlPmtInfAndSts>",
        );
        assert_eq!(status(&one(&only_block).unwrap(), 0), ("RVNM", None));
        let block_first = report("RVCM", &[], &[], &[]).replace(
            "<OrgnlNbOfTxs>1</OrgnlNbOfTxs></OrgnlPmtInfAndSts>",
            "<OrgnlNbOfTxs>1</OrgnlNbOfTxs><PmtInfSts>RVMC</PmtInfSts></OrgnlPmtInfAndSts>",
        );
        assert_eq!(status(&one(&block_first).unwrap(), 0), ("RVMC", None));
        assert_eq!(
            status(&one(&report("RVNA", &[], &[], &[])).unwrap(), 0),
            ("RVNA", None)
        );
        assert_eq!(
            status(&one(&report("RVCM", &[("RVNM", 1)], &[], &[])).unwrap(), 0),
            ("RVNM", None)
        );
        // Two transactions: no block status stands for either.
        let two = report("RVNM", &[("RVNM", 2)], &[], &[]);
        assert!(matches!(
            read(&[two.as_bytes()], MSG, &sent_two()),
            Err(FinTSError::Pain002(_))
        ));
    }

    #[test]
    fn never_guesses() {
        // Unlisted, and nothing names its status.
        let err = one(&report("RVCM", &[], &[], &[])).unwrap_err();
        assert!(matches!(err, FinTSError::Pain002(_)), "{err}");
        let err = one(&report("RVCM", &[("RVNM", 1), ("RVMC", 1)], &[], &[])).unwrap_err();
        assert!(matches!(err, FinTSError::Pain002(_)), "{err}");
        // Another message.
        let xml = report("RCVC", &[("RCVC", 1)], &[], &[]).replace(MSG, "OTHER");
        assert!(matches!(one(&xml), Err(FinTSError::Pain002(_))));
        // Unknown transaction status.
        let xml = report(
            "RVCM",
            &[],
            &[],
            &[tx(
                "NOTPROVIDED",
                "DE89370400440532013000",
                "Max Muster",
                "ACCP",
                "",
            )],
        );
        assert!(matches!(one(&xml), Err(FinTSError::Pain002(_))));
        // Broken XML, wrong root, no document.
        for bad in ["<Document><CstmrPmtStsRpt>", "<Foo/>", "", "not xml"] {
            assert!(matches!(one(bad), Err(FinTSError::Pain002(_))), "{bad}");
        }
        assert!(matches!(
            read(&[], MSG, &sent()),
            Err(FinTSError::Pain002(_))
        ));
        let pain001 = include_str!("../../tests/fixtures/pain/pain.001.001.09.xml");
        assert!(matches!(one(pain001), Err(FinTSError::Pain002(_))));
    }

    #[test]
    fn unknown_listed_payee_is_kept() {
        let xml = report(
            "RVCM",
            &[("RCVC", 1), ("RVNM", 1)],
            &[],
            &[tx(
                "X",
                "DE02120300000000202051",
                "Erika Beispiel",
                "RVNM",
                "",
            )],
        );
        let r = one(&xml).unwrap();
        assert_eq!(r.transactions.len(), 2);
        assert_eq!(status(&r, 0).0, "RCVC");
        assert_eq!(r.transactions[1].requested_name, "Erika Beispiel");
        assert_eq!(status(&r, 1).0, "RVNM");
    }
}
