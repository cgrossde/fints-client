//! CAMT.052 (ISO 20022 "Bank to Customer Account Report") parser.
//!
//! HICAZ answers HKCAZ with one or more complete camt.052 XML documents.
//! Each `<Ntry>` (entry) of a `<Rpt>` (report) becomes one [`Transaction`]:
//!
//! ```text
//! Document/BkToCstmrAcctRpt/Rpt
//!   Acct/Ccy                               account currency (fallback for Amt)
//!   Ntry
//!     Amt @Ccy, CdtDbtInd (CRDT|DBIT)      amount and sign
//!     RvslInd                              reversal (Storno)
//!     Sts (or Sts/Cd)                      BOOK | PDNG | INFO
//!     BookgDt/Dt (or DtTm)                 booking date  -> Transaction::date
//!     ValDt/Dt   (or DtTm)                 value date    -> Transaction::valuta_date
//!     AcctSvcrRef                          bank reference
//!     BkTxCd/Prtry/Cd                      DK code "SWIFT+GVC+primanota[+ext]"
//!     AddtlNtryInf                         booking text  -> posting_text
//!     NtryDtls/TxDtls                      first TxDtls only:
//!       Refs/EndToEndId, Refs/MndtId, Refs/AcctSvcrRef
//!       RltdPties/{Dbtr,Cdtr,UltmtDbtr,UltmtCdtr}[/Pty]/Nm
//!       RltdPties/{DbtrAcct,CdtrAcct}/Id/IBAN
//!       RltdAgts/{DbtrAgt,CdtrAgt}/FinInstnId/{BIC,BICFI}
//!       Purp/Cd                            ISO 20022 purpose code
//!       RmtInf/Ustrd                       remittance text, parts joined with a line break
//!       RltdPties/Cdtr[/Pty]/Id/PrvtId/Othr/Id          -> creditor_id (SEPA creditor ID)
//!       RltdPties/Dbtr[/Pty]/Id/{PrvtId,OrgId}/Othr/Id  -> debtor_id
//! ```
//!
//! Dates follow the same rule as MT940 (`crate::formats::mt940`): `date` is the
//! booking date and `valuta_date` the value date; an entry without a booking
//! date (typically a pending one) falls back to its value date.
//!
//! Element names are matched without namespace prefix, so camt.052.001.02
//! and the later versions (001.08, …) parse with the same code. Written from
//! the ISO 20022 message definition and the German banking industry's (DK)
//! camt usage notes as seen in bank responses; unknown elements are ignored.

use std::str::FromStr;

use jiff::civil::Date;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use rust_decimal::Decimal;

use crate::banks::BookingIdSource;
use crate::error::{FinTSError, Result};
use crate::types::{Bic, Currency, Iban, Transaction, TransactionStatus};

/// Parse one camt.052 XML document into transactions, in document order.
///
/// Malformed XML, or an entry whose amount or dates cannot be read, is an
/// error ([`FinTSError::Camt`]); the document is not partially returned.
/// The status comes from each entry's `<Sts>` (`PDNG` = pending); callers
/// that know a document holds pending entries (HICAZ's second data element)
/// may override it.
///
/// `booking_id` is the bank profile's rule for
/// [`Transaction::booking_id`]: with
/// [`BookingIdSource::CamtEntryAcctSvcrRef`] an entry with `<Sts>BOOK` gets
/// its entry-level `AcctSvcrRef` (never the one of a `TxDtls`: an entry is
/// one transaction, and its first `TxDtls` describes only part of a batch).
/// A reference that occurs on more than one entry of the document is
/// dropped from all of them.
pub fn parse_camt052(xml: &[u8], booking_id: BookingIdSource) -> Result<Vec<Transaction>> {
    let mut reader = Reader::from_reader(xml);

    let mut buf = Vec::new();
    let mut path: Vec<String> = Vec::new();
    // Text of the current element: text, CDATA and entity references arrive
    // as separate events and are joined here until the next tag.
    let mut text = String::new();
    let mut report_currency: Option<String> = None;
    // When the bank made the report (`CreDtTm` of the report, else of the
    // message): the date of a pending entry that carries none.
    let mut report_created: Option<String> = None;
    let mut entry: Option<Entry> = None;
    let mut out = Vec::new();

    loop {
        let event = reader.read_event_into(&mut buf).map_err(|e| {
            camt_err(format!(
                "XML error at byte {}: {e}",
                reader.error_position()
            ))
        })?;
        if matches!(
            event,
            Event::Start(_) | Event::Empty(_) | Event::End(_) | Event::Eof
        ) {
            let t = text.trim();
            if !t.is_empty() {
                if entry.is_none() && ends_with(&path, &["Rpt", "Acct", "Ccy"]) {
                    report_currency = Some(t.to_string());
                }
                if entry.is_none()
                    && (ends_with(&path, &["Rpt", "CreDtTm"])
                        || (report_created.is_none() && ends_with(&path, &["GrpHdr", "CreDtTm"])))
                {
                    report_created = Some(t.to_string());
                }
                if let Some(e) = entry.as_mut() {
                    e.on_text(&path, t);
                }
            }
            text.clear();
        }
        match event {
            Event::Start(ref e) => {
                path.push(local_name(e.name().as_ref()));
                check_root(&path)?;
                on_start(&path, e, &mut entry);
            }
            Event::Empty(ref e) => {
                // <Amt Ccy="EUR"/> etc. carry no text; only attributes matter.
                path.push(local_name(e.name().as_ref()));
                check_root(&path)?;
                on_start(&path, e, &mut entry);
                path.pop();
            }
            Event::End(_) => {
                let closed = path.pop();
                if closed.as_deref() == Some("Ntry")
                    && let Some(e) = entry.take()
                {
                    let n = out.len() + 1;
                    let fallback = report_currency.as_deref();
                    out.push(
                        e.build(fallback, report_created.as_deref(), booking_id)
                            .map_err(|why| camt_err(format!("entry {n}: {why}")))?,
                    );
                }
            }
            Event::Text(ref t) => {
                let decoded = t.decode().map_err(|e| camt_err(format!("bad text: {e}")))?;
                text.push_str(&decoded);
            }
            Event::CData(ref t) => text.push_str(&String::from_utf8_lossy(&t[..])),
            Event::GeneralRef(ref r) => {
                let name = r
                    .decode()
                    .map_err(|e| camt_err(format!("bad reference: {e}")))?;
                if let Some(c) = r
                    .resolve_char_ref()
                    .map_err(|e| camt_err(format!("bad reference &{name};: {e}")))?
                {
                    text.push(c);
                } else if let Some(s) = quick_xml::escape::resolve_predefined_entity(&name) {
                    text.push_str(s);
                } else {
                    return Err(camt_err(format!("unknown entity &{name};")));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !path.is_empty() {
        return Err(camt_err(format!(
            "document ends inside <{}>",
            path.join("/")
        )));
    }
    drop_shared_booking_ids(&mut out);
    Ok(out)
}

/// Clear the `booking_id` of every transaction whose ID another one in
/// `txs` carries too: an ID that is not unique is no ID.
pub(crate) fn drop_shared_booking_ids(txs: &mut [Transaction]) {
    let mut count = std::collections::HashMap::<String, u32>::new();
    for id in txs.iter().filter_map(|t| t.booking_id.as_ref()) {
        *count.entry(id.clone()).or_default() += 1;
    }
    let shared = count.values().filter(|n| **n > 1).count();
    if shared == 0 {
        return;
    }
    tracing::warn!(
        "[FinTS] {shared} booking reference(s) occur on more than one entry; \
         those entries get no booking ID"
    );
    for t in txs {
        if t.booking_id.as_ref().is_some_and(|id| count[id] > 1) {
            t.booking_id = None;
        }
    }
}

/// The document must be a camt.052 account report: `Document` with
/// `BkToCstmrAcctRpt` below it. Anything else (camt.053, an error
/// document) is not a statement and must not be read as an empty one.
fn check_root(path: &[String]) -> Result<()> {
    match path {
        [root] if root != "Document" => Err(camt_err(format!(
            "root element is <{root}>, expected <Document> (camt.052)"
        ))),
        [_, message] if message != "BkToCstmrAcctRpt" => Err(camt_err(format!(
            "document holds <{message}>, expected <BkToCstmrAcctRpt> (camt.052)"
        ))),
        _ => Ok(()),
    }
}

/// The `NN` of the first `camt.052.001.NN` in `text`: a descriptor, or the
/// namespace at the head of a document.
pub(crate) fn camt052_version(text: &[u8]) -> Option<u32> {
    const MARK: &[u8] = b"camt.052.001.";
    let at = text.windows(MARK.len()).position(|w| w == MARK)? + MARK.len();
    let digits: Vec<u8> = text[at..]
        .iter()
        .copied()
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

/// Log when the descriptor of an answer or the namespace of its document
/// names another camt.052 version than the one requested. The document is
/// parsed by element name either way.
pub(crate) fn note_version_mismatch(requested: &str, descriptor: &str, document: &[u8]) {
    let head = &document[..document.len().min(1024)];
    let requested = camt052_version(requested.as_bytes());
    let descriptor = camt052_version(descriptor.as_bytes());
    let namespace = camt052_version(head);
    if descriptor != requested {
        tracing::debug!(
            "[FinTS] HICAZ descriptor names camt.052.001.{descriptor:?}, requested .{requested:?}"
        );
    }
    if namespace != descriptor {
        tracing::warn!(
            "[FinTS] HICAZ document namespace is camt.052.001.{namespace:?}, its descriptor .{descriptor:?}; parsing by element name"
        );
    }
}

fn camt_err(msg: String) -> FinTSError {
    FinTSError::Camt(msg)
}

fn on_start(path: &[String], e: &BytesStart<'_>, entry: &mut Option<Entry>) {
    let Some(last) = path.last() else {
        return;
    };
    match last.as_str() {
        "Ntry" if ends_with(path, &["Rpt", "Ntry"]) => *entry = Some(Entry::default()),
        "TxDtls" => {
            if let Some(en) = entry.as_mut() {
                en.tx_details += 1;
            }
        }
        "Amt" if ends_with(path, &["Ntry", "Amt"]) => {
            if let Some(en) = entry.as_mut() {
                en.currency = ccy_attr(e);
            }
        }
        "Amt" if ends_with(path, &["AmtDtls", "InstdAmt", "Amt"]) => {
            if let Some(en) = entry.as_mut()
                && en.original_currency.is_none()
            {
                en.original_currency = ccy_attr(e);
            }
        }
        "TtlChrgsAndTaxAmt" if ends_with(path, &["Chrgs", "TtlChrgsAndTaxAmt"]) => {
            if let Some(en) = entry.as_mut()
                && en.fee_total_currency.is_none()
            {
                en.fee_total_currency = ccy_attr(e);
            }
        }
        "Amt" if ends_with(path, &["Chrgs", "Rcrd", "Amt"]) => {
            if let Some(en) = entry.as_mut()
                && en.fee_record_currency.is_none()
            {
                en.fee_record_currency = ccy_attr(e);
            }
        }
        _ => {}
    }
}

/// The `Ccy` attribute of an amount element.
fn ccy_attr(e: &BytesStart<'_>) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| local_name(a.key.as_ref()) == "Ccy")
        .map(|a| String::from_utf8_lossy(&a.value).trim().to_string())
}

/// Element name without namespace prefix.
fn local_name(name: &[u8]) -> String {
    let full = String::from_utf8_lossy(name);
    match full.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => full.into_owned(),
    }
}

fn ends_with(path: &[String], suffix: &[&str]) -> bool {
    path.len() >= suffix.len()
        && path[path.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(a, b)| a == b)
}

/// Fields collected for one `<Ntry>`.
#[derive(Default)]
struct Entry {
    amount: Option<String>,
    currency: Option<String>,
    credit_debit: Option<String>,
    reversal: bool,
    status: Option<String>,
    booking_date: Option<String>,
    value_date: Option<String>,
    bank_reference: Option<String>,
    bank_tx_code: Option<String>,
    additional_info: Option<String>,
    /// Number of `<TxDtls>` seen; only the first one is mapped.
    tx_details: u32,
    end_to_end_id: Option<String>,
    mandate_id: Option<String>,
    tx_bank_reference: Option<String>,
    proprietary_reference: Option<String>,
    debtor_name: Option<String>,
    debtor_iban: Option<String>,
    debtor_bic: Option<String>,
    creditor_name: Option<String>,
    creditor_iban: Option<String>,
    creditor_bic: Option<String>,
    creditor_id: Option<String>,
    debtor_id: Option<String>,
    original_amount: Option<String>,
    original_currency: Option<String>,
    fee_total: Option<String>,
    fee_total_currency: Option<String>,
    fee_record: Option<String>,
    fee_record_currency: Option<String>,
    return_reason: Option<String>,
    sequence_type: Option<String>,
    ultimate_debtor: Option<String>,
    ultimate_creditor: Option<String>,
    purpose_code: Option<String>,
    remittance: Option<String>,
}

fn set(slot: &mut Option<String>, text: &str) {
    if slot.is_none() {
        *slot = Some(text.to_string());
    }
}

impl Entry {
    fn on_text(&mut self, path: &[String], text: &str) {
        let p = |suffix: &[&str]| ends_with(path, suffix);
        // Entry level (directly under Ntry).
        if p(&["Ntry", "Amt"]) {
            set(&mut self.amount, text);
        } else if p(&["Ntry", "CdtDbtInd"]) {
            set(&mut self.credit_debit, text);
        } else if p(&["Ntry", "RvslInd"]) {
            self.reversal = text.eq_ignore_ascii_case("true");
        } else if p(&["Ntry", "Sts"]) || p(&["Ntry", "Sts", "Cd"]) {
            set(&mut self.status, text);
        } else if p(&["Ntry", "BookgDt", "Dt"]) || p(&["Ntry", "BookgDt", "DtTm"]) {
            set(&mut self.booking_date, text);
        } else if p(&["Ntry", "ValDt", "Dt"]) || p(&["Ntry", "ValDt", "DtTm"]) {
            set(&mut self.value_date, text);
        } else if p(&["Ntry", "AcctSvcrRef"]) {
            set(&mut self.bank_reference, text);
        } else if p(&["Ntry", "BkTxCd", "Prtry", "Cd"]) {
            set(&mut self.bank_tx_code, text);
        } else if p(&["Ntry", "AddtlNtryInf"]) {
            set(&mut self.additional_info, text);
        }

        // Fees may sit on the entry or in its details.
        if p(&["Chrgs", "TtlChrgsAndTaxAmt"]) {
            set(&mut self.fee_total, text);
        } else if p(&["Chrgs", "Rcrd", "Amt"]) {
            set(&mut self.fee_record, text);
        }

        // Transaction details: only the first TxDtls of a (batch) entry.
        if self.tx_details != 1 || !path.iter().any(|s| s == "TxDtls") {
            return;
        }
        let name =
            |party: &str| p(&["RltdPties", party, "Nm"]) || p(&["RltdPties", party, "Pty", "Nm"]);
        let bic = |agent: &str| {
            p(&["RltdAgts", agent, "FinInstnId", "BIC"])
                || p(&["RltdAgts", agent, "FinInstnId", "BICFI"])
        };
        if p(&["Refs", "EndToEndId"]) {
            set(&mut self.end_to_end_id, text);
        } else if p(&["Refs", "MndtId"]) {
            set(&mut self.mandate_id, text);
        } else if p(&["Refs", "AcctSvcrRef"]) {
            set(&mut self.tx_bank_reference, text);
        } else if p(&["Refs", "Prtry", "Ref"]) {
            set(&mut self.proprietary_reference, text);
        } else if p(&["TxDtls", "BkTxCd", "Prtry", "Cd"]) {
            set(&mut self.bank_tx_code, text);
        } else if name("Dbtr") {
            set(&mut self.debtor_name, text);
        } else if name("Cdtr") {
            set(&mut self.creditor_name, text);
        } else if name("UltmtDbtr") {
            set(&mut self.ultimate_debtor, text);
        } else if name("UltmtCdtr") {
            set(&mut self.ultimate_creditor, text);
        } else if p(&["AmtDtls", "InstdAmt", "Amt"]) {
            set(&mut self.original_amount, text);
        } else if p(&["RtrInf", "Rsn", "Cd"]) {
            set(&mut self.return_reason, text);
        } else if p(&["PmtTpInf", "SeqTp"]) {
            set(&mut self.sequence_type, text);
        } else if p(&["RltdPties", "Dbtr", "Id", "PrvtId", "Othr", "Id"])
            || p(&["RltdPties", "Dbtr", "Pty", "Id", "PrvtId", "Othr", "Id"])
            || p(&["RltdPties", "Dbtr", "Id", "OrgId", "Othr", "Id"])
            || p(&["RltdPties", "Dbtr", "Pty", "Id", "OrgId", "Othr", "Id"])
        {
            set(&mut self.debtor_id, text);
        } else if p(&["RltdPties", "DbtrAcct", "Id", "IBAN"]) {
            set(&mut self.debtor_iban, text);
        } else if p(&["RltdPties", "CdtrAcct", "Id", "IBAN"]) {
            set(&mut self.creditor_iban, text);
        } else if p(&["RltdPties", "Cdtr", "Id", "PrvtId", "Othr", "Id"])
            || p(&["RltdPties", "Cdtr", "Pty", "Id", "PrvtId", "Othr", "Id"])
        {
            // The SEPA creditor identifier of a direct debit sits in the
            // creditor's PrvtId/Othr, as in pain.008 (Anlage 3 V26.11,
            // 7.1.7.4 Related Parties). camt.052 has no CdtrSchmeId, and an
            // OrgId/Othr is some other organisation identifier.
            set(&mut self.creditor_id, text);
        } else if bic("DbtrAgt") {
            set(&mut self.debtor_bic, text);
        } else if bic("CdtrAgt") {
            set(&mut self.creditor_bic, text);
        } else if p(&["Purp", "Cd"]) {
            set(&mut self.purpose_code, text);
        } else if p(&["RmtInf", "Ustrd"]) {
            // Several <Ustrd> parts are the bank's lines (comdirect sends
            // lines of differing length, live 2026-11-19): keep them apart
            // with a line break (owner 2026-11-19).
            match self.remittance.as_mut() {
                Some(r) => {
                    r.push('\n');
                    r.push_str(text);
                }
                None => self.remittance = Some(text.to_string()),
            }
        }
    }

    fn build(
        self,
        report_currency: Option<&str>,
        report_created: Option<&str>,
        booking_id: BookingIdSource,
    ) -> std::result::Result<Transaction, String> {
        let amount_text = self.amount.as_deref().ok_or("no <Amt>")?;
        let magnitude = Decimal::from_str(amount_text.trim())
            .map_err(|e| format!("bad amount '{amount_text}': {e}"))?;
        let is_debit = match self.credit_debit.as_deref() {
            Some("DBIT") => true,
            Some("CRDT") => false,
            other => return Err(format!("bad <CdtDbtInd> {other:?}")),
        };
        let amount = if is_debit { -magnitude } else { magnitude };

        let value_date = self.value_date.as_deref().map(iso_date).transpose()?;
        let booking_date = self.booking_date.as_deref().map(iso_date).transpose()?;
        // Same rule as MT940: booking date, else value date. A pending entry
        // may carry neither (comdirect, live 2026-11-19); it then takes the
        // day the bank made the report. A booked entry without a date stays
        // an error.
        let pending_date = match (self.status.as_deref(), report_created) {
            (Some("PDNG"), Some(created)) => iso_date(created).ok(),
            _ => None,
        };
        let date = booking_date
            .or(value_date)
            .or(pending_date)
            .ok_or("neither <BookgDt> nor <ValDt>")?;

        let currency = self
            .currency
            .clone()
            .or_else(|| report_currency.map(str::to_string))
            .unwrap_or_else(|| "EUR".to_string());

        // The counterparty: the creditor of a debit, the debtor of a credit.
        let (name, iban, bic) = if is_debit {
            (
                self.creditor_name.clone(),
                self.creditor_iban.clone(),
                self.creditor_bic.clone(),
            )
        } else {
            (
                self.debtor_name.clone(),
                self.debtor_iban.clone(),
                self.debtor_bic.clone(),
            )
        };

        // `Sts` PDNG marks a pending entry; BOOK, INFO and a missing status
        // count as booked.
        let pending = self.status.as_deref() == Some("PDNG");
        let status = if pending {
            TransactionStatus::Pending
        } else {
            TransactionStatus::Booked
        };
        let (gvc, prima_nota) = dk_bank_tx_code(self.bank_tx_code.as_deref());
        let end_to_end_id = self
            .end_to_end_id
            .clone()
            .filter(|r| !crate::formats::absent(r));
        let mandate_id = self
            .mandate_id
            .clone()
            .filter(|r| !crate::formats::absent(r));
        let creditor_id = self
            .creditor_id
            .clone()
            .filter(|r| !crate::formats::absent(r));
        let debtor_id = self
            .debtor_id
            .clone()
            .filter(|r| !crate::formats::absent(r));
        let money = |amount: Option<&str>, ccy: Option<&str>| -> Option<(Decimal, Currency)> {
            let a = Decimal::from_str(amount?.trim()).ok()?.abs();
            Some((a, Currency::new(ccy.unwrap_or(&currency))))
        };
        let original_amount = money(
            self.original_amount.as_deref(),
            self.original_currency.as_deref(),
        );
        let fees = if self.fee_total.is_some() {
            money(
                self.fee_total.as_deref(),
                self.fee_total_currency.as_deref(),
            )
        } else {
            money(
                self.fee_record.as_deref(),
                self.fee_record_currency.as_deref(),
            )
        };
        // The remittance text only. An entry without <Ustrd> has no purpose:
        // its additional entry information is the booking text, not a
        // second copy of it (owner 2026-11-19).
        let purpose = self.remittance.clone();
        let booking_id = match booking_id {
            BookingIdSource::CamtEntryAcctSvcrRef if self.status.as_deref() == Some("BOOK") => {
                self.bank_reference.clone()
            }
            _ => None,
        };
        let bank_reference = self
            .tx_bank_reference
            .clone()
            .or_else(|| self.bank_reference.clone())
            .or_else(|| self.proprietary_reference.clone());

        let raw = serde_json::json!({
            "source": "camt.052",
            "booking_date": self.booking_date,
            "value_date": self.value_date,
            "amount": amount.to_string(),
            "currency": currency,
            "credit_debit": self.credit_debit,
            "reversal": self.reversal,
            "status": self.status,
            "bank_tx_code": self.bank_tx_code,
            "end_to_end_id": self.end_to_end_id,
            "mandate_id": self.mandate_id,
            "creditor_id": self.creditor_id,
            "proprietary_reference": self.proprietary_reference,
            "account_servicer_reference": self.tx_bank_reference.or(self.bank_reference),
            "debtor_name": self.debtor_name,
            "creditor_name": self.creditor_name,
            "ultimate_debtor": self.ultimate_debtor,
            "ultimate_creditor": self.ultimate_creditor,
            "debtor_id": self.debtor_id,
            "return_reason": self.return_reason,
            "sequence_type": self.sequence_type,
            "tx_details_count": self.tx_details,
        });

        Ok(Transaction {
            date,
            valuta_date: value_date,
            amount,
            currency: Currency::new(currency),
            applicant_name: name,
            applicant_iban: iban.map(Iban::new),
            applicant_bic: bic.map(Bic::new),
            purpose,
            posting_text: self.additional_info,
            customer_reference: None,
            bank_reference,
            booking_id,
            gvc,
            prima_nota,
            purpose_code: self.purpose_code,
            mandate_id,
            creditor_id,
            end_to_end_id,
            debtor_id,
            ultimate_debtor: self.ultimate_debtor.clone(),
            ultimate_creditor: self.ultimate_creditor.clone(),
            reversal: self.reversal,
            original_amount,
            fees,
            return_reason: self.return_reason.clone(),
            sequence_type: self.sequence_type.clone(),
            raw,
            status,
        })
    }
}

/// `2026-08-20` or `2026-08-20T10:15:00[+02:00]` → date.
fn iso_date(s: &str) -> std::result::Result<Date, String> {
    s.get(..10)
        .and_then(|d| d.parse::<Date>().ok())
        .ok_or_else(|| format!("bad date '{s}'"))
}

/// German banks fill `BkTxCd/Prtry/Cd` as `<SWIFT code>+<GVC>+<primanota>[+…]`,
/// e.g. `NDDT+105+9208+888`. Returns (GVC, primanota) when it has that shape.
fn dk_bank_tx_code(code: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(code) = code else {
        return (None, None);
    };
    let mut parts = code.split('+');
    let _swift = parts.next();
    let gvc = parts
        .next()
        .map(str::trim)
        .filter(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_string);
    if gvc.is_none() {
        return (None, None);
    }
    let prima_nota = parts
        .next()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    (gvc, prima_nota)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i16, m: i8, day: i8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    /// Synthetic report: invented names, references and DE00 IBANs.
    const REPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.052.001.02">
 <BkToCstmrAcctRpt>
  <GrpHdr><MsgId>TEST-MSG-1</MsgId></GrpHdr>
  <Rpt>
   <Id>TEST-RPT-1</Id>
   <Acct><Id><IBAN>DE00120300009900012345</IBAN></Id><Ccy>EUR</Ccy></Acct>
   <Ntry>
    <Amt Ccy="EUR">42.50</Amt>
    <CdtDbtInd>DBIT</CdtDbtInd>
    <Sts>BOOK</Sts>
    <BookgDt><Dt>2026-03-02</Dt></BookgDt>
    <ValDt><Dt>2026-02-27</Dt></ValDt>
    <AcctSvcrRef>BANKREF-0001</AcctSvcrRef>
    <BkTxCd><Prtry><Cd>NDDT+105+9208+888</Cd><Issr>DK</Issr></Prtry></BkTxCd>
    <AddtlNtryInf>FOLGELASTSCHRIFT</AddtlNtryInf>
    <NtryDtls><TxDtls>
     <Refs><EndToEndId>E2E-TEST-0001</EndToEndId><MndtId>MANDATE-TEST-7</MndtId></Refs>
     <RltdPties>
      <Dbtr><Nm>Erika Beispiel</Nm></Dbtr>
      <DbtrAcct><Id><IBAN>DE00120300009900012345</IBAN></Id></DbtrAcct>
      <Cdtr><Nm>Zahlungsdienst Test GmbH</Nm>
       <Id><PrvtId><Othr><Id>DE00ZZZ00000000001</Id></Othr></PrvtId></Id></Cdtr>
      <CdtrAcct><Id><IBAN>DE00500000000000000001</IBAN></Id></CdtrAcct>
      <UltmtCdtr><Nm>Beispielladen am Markt</Nm></UltmtCdtr>
     </RltdPties>
     <RltdAgts><CdtrAgt><FinInstnId><BIC>TESTDEFFXXX</BIC></FinInstnId></CdtrAgt></RltdAgts>
     <Purp><Cd>IDCP</Cd></Purp>
     <RmtInf><Ustrd>Bestellung 4711</Ustrd><Ustrd>Kundennummer 12</Ustrd></RmtInf>
    </TxDtls></NtryDtls>
   </Ntry>
   <Ntry>
    <Amt Ccy="EUR">1500.00</Amt>
    <CdtDbtInd>CRDT</CdtDbtInd>
    <Sts>BOOK</Sts>
    <BookgDt><Dt>2026-03-01</Dt></BookgDt>
    <ValDt><Dt>2026-03-01</Dt></ValDt>
    <AddtlNtryInf>GUTSCHRIFT</AddtlNtryInf>
    <NtryDtls><TxDtls>
     <Refs><EndToEndId>NOTPROVIDED</EndToEndId></Refs>
     <RltdPties>
      <Dbtr><Nm>Musterfirma AG</Nm></Dbtr>
      <DbtrAcct><Id><IBAN>DE00500000000000000002</IBAN></Id></DbtrAcct>
      <UltmtDbtr><Nm>Musterfirma Lohnbuero</Nm></UltmtDbtr>
     </RltdPties>
     <RltdAgts><DbtrAgt><FinInstnId><BIC>TESTDEFFXXX</BIC></FinInstnId></DbtrAgt></RltdAgts>
     <RmtInf><Ustrd>Lohn Maerz</Ustrd></RmtInf>
    </TxDtls></NtryDtls>
   </Ntry>
   <Ntry>
    <Amt>25.00</Amt>
    <CdtDbtInd>DBIT</CdtDbtInd>
    <Sts>PDNG</Sts>
    <ValDt><Dt>2026-03-04</Dt></ValDt>
    <AddtlNtryInf>DAUERAUFTRAG</AddtlNtryInf>
   </Ntry>
  </Rpt>
 </BkToCstmrAcctRpt>
</Document>"#;

    /// Synthetic report in the shape DKB sends (camt.052.001.08, `Sts/Cd`,
    /// the bank reference at entry level, live 2026-11-17): two booked
    /// entries, one booked batch entry whose `TxDtls` carry their own
    /// references, a pending entry with a reference and a booked one
    /// without. References and names are invented.
    const DKB_SHAPED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.052.001.08">
 <BkToCstmrAcctRpt><Rpt>
  <Acct><Id><IBAN>DE00120300000000000201</IBAN></Id></Acct>
  <Ntry><Amt Ccy="EUR">10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts><Cd>BOOK</Cd></Sts>
   <BookgDt><Dt>2026-03-02</Dt></BookgDt><ValDt><Dt>2026-03-02</Dt></ValDt>
   <AcctSvcrRef>TESTREF00000000000000000001</AcctSvcrRef>
   <NtryDtls><TxDtls><RmtInf><Ustrd>Test 1</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  <Ntry><Amt Ccy="EUR">20.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Sts><Cd>BOOK</Cd></Sts>
   <BookgDt><Dt>2026-03-03</Dt></BookgDt><ValDt><Dt>2026-03-03</Dt></ValDt>
   <AcctSvcrRef>TESTREF00000000000000000002</AcctSvcrRef>
   <NtryDtls><TxDtls><RmtInf><Ustrd>Test 2</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
  <Ntry><Amt Ccy="EUR">30.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts><Cd>BOOK</Cd></Sts>
   <BookgDt><Dt>2026-03-04</Dt></BookgDt><ValDt><Dt>2026-03-04</Dt></ValDt>
   <AcctSvcrRef>TESTREF00000000000000000003</AcctSvcrRef>
   <NtryDtls>
    <TxDtls><Refs><AcctSvcrRef>TESTTX-A</AcctSvcrRef></Refs><Amt Ccy="EUR">10.00</Amt></TxDtls>
    <TxDtls><Refs><AcctSvcrRef>TESTTX-B</AcctSvcrRef></Refs><Amt Ccy="EUR">20.00</Amt></TxDtls>
   </NtryDtls></Ntry>
  <Ntry><Amt Ccy="EUR">5.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts><Cd>PDNG</Cd></Sts>
   <ValDt><Dt>2026-03-05</Dt></ValDt>
   <AcctSvcrRef>TESTREF00000000000000000004</AcctSvcrRef></Ntry>
  <Ntry><Amt Ccy="EUR">7.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts><Cd>BOOK</Cd></Sts>
   <BookgDt><Dt>2026-03-05</Dt></BookgDt></Ntry>
 </Rpt></BkToCstmrAcctRpt>
</Document>"#;

    fn booking_ids(xml: &str, rule: BookingIdSource) -> Vec<Option<String>> {
        parse_camt052(xml.as_bytes(), rule)
            .unwrap()
            .into_iter()
            .map(|t| t.booking_id)
            .collect()
    }

    #[test]
    fn booked_entries_get_their_entry_reference_as_booking_id() {
        let ids = booking_ids(DKB_SHAPED, BookingIdSource::CamtEntryAcctSvcrRef);
        assert_eq!(
            ids,
            [
                Some("TESTREF00000000000000000001".to_string()),
                Some("TESTREF00000000000000000002".to_string()),
                // The batch entry: the entry's reference, not a TxDtls one.
                Some("TESTREF00000000000000000003".to_string()),
                // Pending: unverified whether it keeps the reference.
                None,
                // No reference.
                None,
            ]
        );
        // bank_reference is unchanged: the TxDtls reference first.
        let txs =
            parse_camt052(DKB_SHAPED.as_bytes(), BookingIdSource::CamtEntryAcctSvcrRef).unwrap();
        assert_eq!(txs[2].bank_reference.as_deref(), Some("TESTTX-A"));
        assert_eq!(
            txs[3].bank_reference.as_deref(),
            Some("TESTREF00000000000000000004")
        );
    }

    #[test]
    fn without_the_profile_rule_no_entry_gets_a_booking_id() {
        let ids = booking_ids(DKB_SHAPED, BookingIdSource::None);
        assert!(ids.iter().all(Option::is_none), "{ids:?}");
        let ids = booking_ids(REPORT, BookingIdSource::None);
        assert!(ids.iter().all(Option::is_none), "{ids:?}");
    }

    #[test]
    fn a_reference_on_two_entries_is_no_booking_id() {
        let twice =
            DKB_SHAPED.replace("TESTREF00000000000000000002", "TESTREF00000000000000000001");
        let ids = booking_ids(&twice, BookingIdSource::CamtEntryAcctSvcrRef);
        assert_eq!(ids[0], None);
        assert_eq!(ids[1], None);
        assert_eq!(ids[2].as_deref(), Some("TESTREF00000000000000000003"));
    }

    #[test]
    fn an_entry_without_explicit_booked_status_gets_no_booking_id() {
        let no_status = DKB_SHAPED.replacen("<Sts><Cd>BOOK</Cd></Sts>", "", 1);
        let txs =
            parse_camt052(no_status.as_bytes(), BookingIdSource::CamtEntryAcctSvcrRef).unwrap();
        assert_eq!(txs[0].status, TransactionStatus::Booked);
        assert_eq!(txs[0].booking_id, None);
        assert!(txs[1].booking_id.is_some());
    }

    #[test]
    fn maps_debit_entry_with_ultimate_party_purpose_code_and_mandate() {
        let txs = parse_camt052(REPORT.as_bytes(), BookingIdSource::None).unwrap();
        assert_eq!(txs.len(), 3);
        let tx = &txs[0];
        assert_eq!(tx.amount, dec("-42.50"));
        assert_eq!(tx.currency.as_str(), "EUR");
        assert_eq!(
            tx.applicant_name.as_deref(),
            Some("Zahlungsdienst Test GmbH")
        );
        assert_eq!(
            tx.ultimate_creditor.as_deref(),
            Some("Beispielladen am Markt")
        );
        assert_eq!(tx.ultimate_debtor, None);
        assert_eq!(
            tx.applicant_iban.as_ref().map(|i| i.as_str()),
            Some("DE00500000000000000001")
        );
        assert_eq!(
            tx.applicant_bic.as_ref().map(|b| b.as_str()),
            Some("TESTDEFFXXX")
        );
        assert_eq!(tx.purpose_code.as_deref(), Some("IDCP"));
        assert_eq!(tx.mandate_id.as_deref(), Some("MANDATE-TEST-7"));
        assert_eq!(tx.end_to_end_id.as_deref(), Some("E2E-TEST-0001"));
        assert_eq!(
            tx.customer_reference, None,
            "CAMT has no customer reference"
        );
        assert!(!tx.reversal);
        assert_eq!(tx.bank_reference.as_deref(), Some("BANKREF-0001"));
        assert_eq!(tx.gvc.as_deref(), Some("105"));
        assert_eq!(tx.prima_nota.as_deref(), Some("9208"));
        assert_eq!(tx.posting_text.as_deref(), Some("FOLGELASTSCHRIFT"));
        assert_eq!(
            tx.purpose.as_deref(),
            Some("Bestellung 4711\nKundennummer 12")
        );
        assert_eq!(tx.raw["creditor_id"], "DE00ZZZ00000000001");
        assert_eq!(tx.status, TransactionStatus::Booked);
    }

    /// Same date semantics as MT940: `date` = booking date (BookgDt),
    /// `valuta_date` = value date (ValDt).
    /// One entry with the optional fields of F3, all synthetic.
    fn rich_entry(extra_entry: &str, details: &str) -> Transaction {
        let xml = format!(
            r#"<Document><BkToCstmrAcctRpt><Rpt>
<Acct><Ccy>EUR</Ccy></Acct>
<Ntry><Amt Ccy="EUR">10.00</Amt><CdtDbtInd>CRDT</CdtDbtInd>{extra_entry}
<Sts>BOOK</Sts><BookgDt><Dt>2026-03-01</Dt></BookgDt>
<NtryDtls><TxDtls>{details}</TxDtls></NtryDtls></Ntry>
</Rpt></BkToCstmrAcctRpt></Document>"#
        );
        parse_camt052(xml.as_bytes(), BookingIdSource::None)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn reversal_indicator() {
        assert!(rich_entry("<RvslInd>true</RvslInd>", "").reversal);
        assert!(!rich_entry("<RvslInd>false</RvslInd>", "").reversal);
        assert!(!rich_entry("", "").reversal);
    }

    /// The SEPA creditor ID is the creditor's PrvtId/Othr/Id (Anlage 3
    /// V26.11, 7.1.7.4), with or without the `Pty` of camt.052.001.08's
    /// Party40Choice; an OrgId/Othr is not a creditor ID.
    #[test]
    fn creditor_id_from_private_id_only() {
        let plain = rich_entry(
            "",
            "<RltdPties><Cdtr><Nm>C</Nm><Id><PrvtId><Othr><Id>DE00ZZZ00000000002</Id><SchmeNm><Prtry>SEPA</Prtry></SchmeNm></Othr></PrvtId></Id></Cdtr></RltdPties>",
        );
        assert_eq!(plain.creditor_id.as_deref(), Some("DE00ZZZ00000000002"));
        let party = rich_entry(
            "",
            "<RltdPties><Cdtr><Pty><Nm>C</Nm><Id><PrvtId><Othr><Id>DE00ZZZ00000000003</Id></Othr></PrvtId></Id></Pty></Cdtr></RltdPties>",
        );
        assert_eq!(party.creditor_id.as_deref(), Some("DE00ZZZ00000000003"));
        let org = rich_entry(
            "",
            "<RltdPties><Cdtr><Nm>C</Nm><Id><OrgId><Othr><Id>ORG-ID-1</Id></Othr></OrgId></Id></Cdtr></RltdPties>",
        );
        assert_eq!(org.creditor_id, None);
    }

    #[test]
    fn debtor_id_from_private_or_organisation_id() {
        let private = rich_entry(
            "",
            "<RltdPties><Dbtr><Nm>A</Nm><Id><PrvtId><Othr><Id>DEBTOR-ID-1</Id></Othr></PrvtId></Id></Dbtr></RltdPties>",
        );
        assert_eq!(private.debtor_id.as_deref(), Some("DEBTOR-ID-1"));
        let org = rich_entry(
            "",
            "<RltdPties><Dbtr><Nm>A</Nm><Id><OrgId><Othr><Id>DEBTOR-ID-2</Id></Othr></OrgId></Id></Dbtr></RltdPties>",
        );
        assert_eq!(org.debtor_id.as_deref(), Some("DEBTOR-ID-2"));
        let placeholder = rich_entry(
            "",
            "<RltdPties><Dbtr><Id><PrvtId><Othr><Id>NOTPROVIDED</Id></Othr></PrvtId></Id></Dbtr></RltdPties>",
        );
        assert_eq!(placeholder.debtor_id, None);
    }

    #[test]
    fn original_amount_fees_return_reason_and_sequence_type() {
        let tx = rich_entry(
            "<Chrgs><TtlChrgsAndTaxAmt Ccy=\"EUR\">1.50</TtlChrgsAndTaxAmt></Chrgs>",
            r#"<AmtDtls><InstdAmt><Amt Ccy="USD">11.20</Amt></InstdAmt></AmtDtls>
<RtrInf><Rsn><Cd>MD06</Cd></Rsn></RtrInf>
<RltdPties/><PmtTpInf><SeqTp>RCUR</SeqTp></PmtTpInf>"#,
        );
        assert_eq!(
            tx.original_amount,
            Some((dec("11.20"), Currency::new("USD")))
        );
        assert_eq!(tx.fees, Some((dec("1.50"), Currency::new("EUR"))));
        assert_eq!(tx.return_reason.as_deref(), Some("MD06"));
        assert_eq!(tx.sequence_type.as_deref(), Some("RCUR"));
        let none = rich_entry("", "");
        assert_eq!(none.original_amount, None);
        assert_eq!(none.fees, None);
        assert_eq!(none.return_reason, None);
        assert_eq!(none.sequence_type, None);
    }

    #[test]
    fn fee_record_is_used_without_a_total() {
        let tx = rich_entry(
            r#"<Chrgs><Rcrd><Amt Ccy="CHF">2.00</Amt></Rcrd></Chrgs>"#,
            "",
        );
        assert_eq!(tx.fees, Some((dec("2.00"), Currency::new("CHF"))));
    }

    #[test]
    fn camt_booking_and_value_date_not_swapped() {
        let tx = &parse_camt052(REPORT.as_bytes(), BookingIdSource::None).unwrap()[0];
        assert_eq!(tx.date, d(2026, 3, 2), "date is the booking date");
        assert_eq!(
            tx.valuta_date,
            Some(d(2026, 2, 27)),
            "valuta_date is the value date"
        );
    }

    #[test]
    fn maps_credit_entry_from_debtor_side() {
        let tx = &parse_camt052(REPORT.as_bytes(), BookingIdSource::None).unwrap()[1];
        assert_eq!(tx.amount, dec("1500.00"));
        assert_eq!(tx.applicant_name.as_deref(), Some("Musterfirma AG"));
        assert_eq!(tx.ultimate_debtor.as_deref(), Some("Musterfirma Lohnbuero"));
        assert_eq!(tx.ultimate_creditor, None);
        assert_eq!(tx.debtor_id, None);
        assert_eq!(
            tx.applicant_iban.as_ref().map(|i| i.as_str()),
            Some("DE00500000000000000002")
        );
        assert_eq!(tx.end_to_end_id, None, "NOTPROVIDED is no reference");
        assert_eq!(tx.mandate_id, None);
    }

    /// comdirect's shape (live 2026-11-19): a pending entry with no dates.
    fn dateless(status: &str, header: &str) -> String {
        format!(
            r#"<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.052.001.08"><BkToCstmrAcctRpt>{header}<Rpt>
<Acct><Id><IBAN>DE00200411550000000101</IBAN></Id></Acct>
<Ntry><Amt Ccy="EUR">1.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts><Cd>{status}</Cd></Sts>
<NtryDtls><TxDtls><RmtInf><Ustrd>Test</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>
</Rpt></BkToCstmrAcctRpt></Document>"#
        )
    }

    const CREATED: &str = "<GrpHdr><CreDtTm>2026-11-19T22:04:56.236+01:00</CreDtTm></GrpHdr>";

    #[test]
    fn a_pending_entry_without_dates_takes_the_day_of_the_report() {
        let xml = dateless("PDNG", CREATED);
        let tx = &parse_camt052(xml.as_bytes(), BookingIdSource::None).unwrap()[0];
        assert_eq!(tx.status, TransactionStatus::Pending);
        assert_eq!(tx.date, d(2026, 11, 19));
        assert_eq!(tx.valuta_date, None);
    }

    #[test]
    fn a_dateless_entry_is_an_error_when_booked_or_without_a_report_time() {
        for xml in [dateless("BOOK", CREATED), dateless("PDNG", "")] {
            let err = parse_camt052(xml.as_bytes(), BookingIdSource::None).unwrap_err();
            assert!(
                err.to_string().contains("neither <BookgDt> nor <ValDt>"),
                "{err}"
            );
        }
    }

    #[test]
    fn pending_entry_without_booking_date_uses_value_date_and_report_currency() {
        let tx = &parse_camt052(REPORT.as_bytes(), BookingIdSource::None).unwrap()[2];
        assert_eq!(tx.status, TransactionStatus::Pending);
        assert_eq!(tx.date, d(2026, 3, 4));
        assert_eq!(tx.valuta_date, Some(d(2026, 3, 4)));
        assert_eq!(tx.currency.as_str(), "EUR");
        assert_eq!(tx.applicant_name, None);
        // No <Ustrd>: no purpose; the entry information is the booking text.
        assert_eq!(tx.purpose, None);
        assert_eq!(tx.posting_text.as_deref(), Some("DAUERAUFTRAG"));
    }

    #[test]
    fn later_schema_versions_with_prefixes_and_datetimes() {
        let xml = r#"<?xml version="1.0"?>
<c:Document xmlns:c="urn:iso:std:iso:20022:tech:xsd:camt.052.001.08">
 <c:BkToCstmrAcctRpt><c:Rpt>
  <c:Acct><c:Ccy>USD</c:Ccy></c:Acct>
  <c:Ntry>
   <c:Amt>7.5</c:Amt><c:CdtDbtInd>CRDT</c:CdtDbtInd>
   <c:Sts><c:Cd>BOOK</c:Cd></c:Sts>
   <c:BookgDt><c:DtTm>2026-05-06T09:30:00+02:00</c:DtTm></c:BookgDt>
   <c:NtryDtls><c:TxDtls>
    <c:RltdPties><c:Dbtr><c:Pty><c:Nm>Pty &amp; Name</c:Nm></c:Pty></c:Dbtr></c:RltdPties>
    <c:RltdAgts><c:DbtrAgt><c:FinInstnId><c:BICFI>TESTDEFFXXX</c:BICFI></c:FinInstnId></c:DbtrAgt></c:RltdAgts>
   </c:TxDtls><c:TxDtls>
    <c:RltdPties><c:Dbtr><c:Pty><c:Nm>Second detail ignored</c:Nm></c:Pty></c:Dbtr></c:RltdPties>
   </c:TxDtls></c:NtryDtls>
  </c:Ntry>
 </c:Rpt></c:BkToCstmrAcctRpt>
</c:Document>"#;
        let txs = parse_camt052(xml.as_bytes(), BookingIdSource::None).unwrap();
        assert_eq!(txs.len(), 1);
        let tx = &txs[0];
        assert_eq!(tx.date, d(2026, 5, 6));
        assert_eq!(tx.valuta_date, None);
        assert_eq!(tx.currency.as_str(), "USD");
        assert_eq!(tx.applicant_name.as_deref(), Some("Pty & Name"));
        assert_eq!(
            tx.applicant_bic.as_ref().map(|b| b.as_str()),
            Some("TESTDEFFXXX")
        );
        assert_eq!(tx.raw["tx_details_count"], 2);
    }

    #[test]
    fn empty_report_and_errors() {
        let empty = r#"<Document><BkToCstmrAcctRpt><Rpt><Acct><Ccy>EUR</Ccy></Acct></Rpt></BkToCstmrAcctRpt></Document>"#;
        assert!(
            parse_camt052(empty.as_bytes(), BookingIdSource::None)
                .unwrap()
                .is_empty()
        );
        assert!(parse_camt052(b"<not valid xml", BookingIdSource::None).is_err());
        let no_amount = "<Document><Rpt><Ntry><CdtDbtInd>CRDT</CdtDbtInd>\
            <BookgDt><Dt>2026-01-01</Dt></BookgDt></Ntry></Rpt></Document>";
        assert!(matches!(
            parse_camt052(no_amount.as_bytes(), BookingIdSource::None),
            Err(FinTSError::Camt(_))
        ));
        let bad_date = "<Document><Rpt><Ntry><Amt>1</Amt><CdtDbtInd>CRDT</CdtDbtInd>\
            <BookgDt><Dt>2026-02-30</Dt></BookgDt></Ntry></Rpt></Document>";
        assert!(parse_camt052(bad_date.as_bytes(), BookingIdSource::None).is_err());
    }

    #[test]
    fn dk_code_split() {
        assert_eq!(
            dk_bank_tx_code(Some("NTRF+166+9310+997")),
            (Some("166".into()), Some("9310".into()))
        );
        assert_eq!(dk_bank_tx_code(Some("PMNT-RCDT-ESCT")), (None, None));
        assert_eq!(dk_bank_tx_code(None), (None, None));
    }

    proptest::proptest! {
        #[test]
        fn never_panics_on_random_bytes(data in proptest::collection::vec(proptest::num::u8::ANY, 0..512)) {
            let _ = parse_camt052(&data, BookingIdSource::None);
        }

        #[test]
        fn never_panics_on_xml_shaped_input(
            parts in proptest::collection::vec(
                proptest::prop_oneof![
                    proptest::strategy::Just("<Rpt>".to_string()),
                    proptest::strategy::Just("<Ntry>".to_string()),
                    proptest::strategy::Just("</Ntry>".to_string()),
                    proptest::strategy::Just("<Amt Ccy=\"EUR\">".to_string()),
                    proptest::strategy::Just("</Amt>".to_string()),
                    proptest::strategy::Just("<CdtDbtInd>DBIT</CdtDbtInd>".to_string()),
                    proptest::strategy::Just("<BookgDt><Dt>".to_string()),
                    proptest::strategy::Just("</Dt></BookgDt>".to_string()),
                    proptest::strategy::Just("<TxDtls><RmtInf><Ustrd>".to_string()),
                    proptest::strategy::Just("</Ustrd></RmtInf></TxDtls>".to_string()),
                    "[0-9A-Za-z<>/&;.: -]{0,12}",
                ],
                0..24,
            )
        ) {
            let _ = parse_camt052(parts.concat().as_bytes(), BookingIdSource::None);
        }
    }
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[test]
    fn rejects_other_messages() {
        let camt053 = br#"<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02"><BkToCstmrStmt/></Document>"#;
        assert!(matches!(
            parse_camt052(camt053, BookingIdSource::None),
            Err(FinTSError::Camt(_))
        ));
        let other = br#"<Foo><BkToCstmrAcctRpt/></Foo>"#;
        assert!(matches!(
            parse_camt052(other, BookingIdSource::None),
            Err(FinTSError::Camt(_))
        ));
        let ok = br#"<Document><BkToCstmrAcctRpt/></Document>"#;
        assert!(parse_camt052(ok, BookingIdSource::None).unwrap().is_empty());
    }

    #[test]
    fn version_from_descriptor_and_namespace() {
        assert_eq!(
            camt052_version(b"urn:iso:std:iso:20022:tech:xsd:camt.052.001.02.xsd"),
            Some(2)
        );
        assert_eq!(camt052_version(b"camt.052.001.08"), Some(8));
        assert_eq!(camt052_version(b"camt.053.001.08"), None);
    }
}
