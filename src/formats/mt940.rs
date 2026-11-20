//! MT940 (booked) and MT942 (pending) statement parsing.
//!
//! Written from HBCI 2.2 Final, chapter IX.2.8 (MT 940) and IX.2.9
//! (MT 942), and the bank output seen in live runs. (FinTS Messages -
//! Finanzdatenformate B.8 is not in `spec/`; DFÜ-Abkommen Anlage 3 V26.11
//! chapter 8 says the MT940/MT942 section was dropped and points to the
//! V3.8 archive, which is not in `spec/` either.)
//!
//! - **Statements** start at `:20:`. Each field starts a line with its tag
//!   (`:20:`, `:21:`, `:25:`, `:28C:`, `:60F:`/`:60M:`, `:61:`, `:86:`,
//!   `:62F:`/`:62M:`, `:64:`, `:65:`, and for MT942 `:34F:`, `:13:`/`:13D:`,
//!   `:90D:`/`:90C:`); lines without a tag continue the field before. The
//!   order of the fields is not checked and unknown tags are skipped.
//! - **Read:** the `:61:` bookings, the `:86:` body that follows each, and
//!   the currency: from `:60F:`/`:60M:`, for an MT942 without opening
//!   balance from the first `:34F:`, else from `:90D:`/`:90C:`.
//! - **MT942** ([`parse_mt942`]) also reads the creation time (`:13:` or
//!   `:13D:`) and checks the booking counts against `:90D:`/`:90C:`.
//! - **Separators** (`-`, `--`) and blank lines are skipped, also a `-`
//!   glued to the next tag (`-:20:`, when touchdown pages are joined).
//!   Data of separators and blank lines only is no statement, not an
//!   error.
//! - **Line breaks** are CR LF, LF, or `@@` (some banks write `@@` for the
//!   line break inside FinTS binary data, as in MT535).
//! - **`:86:`** continuation lines are joined without separator (banks wrap
//!   at 27 or 65 characters, and send more than 6 lines). The body is kept
//!   whole for `formats::tag86`; a `:86:` that does not follow a `:61:`
//!   (statement level, after `:62F:`) belongs to no booking and is dropped.
//! - **Dates** that do not exist in their month (29 February outside leap
//!   years, 30/31 February for month-end postings) become the month's last
//!   day, in the value and the entry date.
//! - **Characters** are the bank's: the data is decoded as Windows-1252.
//! - **Errors:** a malformed `:61:`, a balance without currency, a
//!   statement with bookings but no currency or a field before the first
//!   `:20:` fail the whole call with [`FinTSError::Mt940`], naming the line
//!   and the field.

use jiff::civil::Date;
use rust_decimal::Decimal;
use tracing::warn;

use crate::error::{FinTSError, Result};
use crate::formats::{self, tag86, tag86::Keyword};
use crate::types::*;

pub(crate) fn parse_mt940(data: &[u8], status: TransactionStatus) -> Result<Vec<Transaction>> {
    let mut transactions = Vec::new();
    for statement in decode_statements(data)? {
        push_transactions(&mut transactions, statement, &status);
    }
    Ok(transactions)
}

/// The pending part of an HKKAZ answer.
#[derive(Debug)]
pub(crate) struct Mt942 {
    /// The bookings, marked [`TransactionStatus::Pending`].
    pub transactions: Vec<Transaction>,
    /// The earliest creation time (`:13:`/`:13D:`) of the statements:
    /// joined touchdown pages can carry one statement each, and the list is
    /// as old as its oldest part. `None` when no statement had a readable
    /// one.
    pub created: Option<BankTime>,
}

/// Parse MT942 data (HBCI 2.2 IX.2.9). On top of [`parse_mt940`], each
/// statement's `:90D:`/`:90C:` (count and sum of the debit and credit
/// bookings, both optional) is checked against its `:61:` bookings: a count
/// that does not match fails the call, because the list would not be
/// complete; a sum or currency that does not match is a warning (see
/// [`Statement::check_totals`]).
pub(crate) fn parse_mt942(data: &[u8]) -> Result<Mt942> {
    let mut transactions = Vec::new();
    let mut created: Option<BankTime> = None;
    for statement in decode_statements(data)? {
        statement.check_totals()?;
        if let Some(t) = statement.created
            && created.is_none_or(|c| t.datetime < c.datetime)
        {
            created = Some(t);
        }
        push_transactions(&mut transactions, statement, &TransactionStatus::Pending);
    }
    Ok(Mt942 {
        transactions,
        created,
    })
}

/// MT942 data with each `:90D:`/`:90C:` sum set to the sum of its
/// statement's `:61:` amounts on that side, count and currency as they
/// were. For the replay of a shape-masked trace (`mask::shape::unshape`),
/// where the amounts are rebuilt as `1…` and no longer add up. A total whose
/// `:20:` is not in `data` (a statement split across touchdown pages) is
/// left as it is.
#[cfg_attr(not(feature = "trace"), allow(dead_code))]
pub(crate) fn recompute_totals(data: &[u8]) -> Vec<u8> {
    let data = &*at_line_breaks(data);
    let (text, _, _) = encoding_rs::WINDOWS_1252.decode(data);
    let mut lines: Vec<Vec<u8>> = data.split(|&b| b == b'\n').map(<[u8]>::to_vec).collect();
    let mut sums: Option<[Decimal; 2]> = None;
    for field in fields(&text) {
        match field.tag {
            "20" => sums = Some([Decimal::ZERO; 2]),
            "61" => {
                if let (Some(sums), Ok(line)) = (sums.as_mut(), parse_61(&field)) {
                    sums[usize::from(line.mark.credits_account())] += line.amount;
                }
            }
            "90D" | "90C" => {
                let tag = if field.tag == "90D" { "90D" } else { "90C" };
                let (Some(sums), Ok(t)) = (sums, total(&field, tag)) else {
                    continue;
                };
                let sum = sums[usize::from(tag == "90C")].normalize();
                let mut amount = sum.to_string().replace('.', ",");
                if !amount.contains(',') {
                    amount.push(',');
                }
                let Some(line) = lines.get_mut(field.line_no - 1) else {
                    continue;
                };
                let marker = format!(":{tag}:");
                let Some(at) = line
                    .windows(marker.len())
                    .position(|w| w == marker.as_bytes())
                else {
                    continue;
                };
                let cr = line.ends_with(b"\r");
                line.truncate(at);
                line.extend_from_slice(
                    format!("{marker}{}{}{amount}", t.count, t.currency).as_bytes(),
                );
                if cr {
                    line.push(b'\r');
                }
            }
            _ => {}
        }
    }
    lines.join(&b'\n')
}

fn decode_statements(data: &[u8]) -> Result<Vec<Statement>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let data = at_line_breaks(data);
    let (cow, _, had_errors) = encoding_rs::WINDOWS_1252.decode(&data);
    if had_errors {
        warn!("MT940 encoding errors");
    }
    parse_statements(&cow)
}

/// `data` with each `@@` as CR LF. Some banks write `@@` for the line
/// break of SWIFT data inside FinTS binary data (observed for MT535, see
/// `segments::mt535`, and common client behaviour for MT940); read as one
/// line, the whole statement would be a `:20:` without bookings. The
/// replacement has the same length, so byte positions stay.
fn at_line_breaks(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !data.windows(2).any(|w| w == b"@@") {
        return std::borrow::Cow::Borrowed(data);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut rest = data;
    while let Some((&b, tail)) = rest.split_first() {
        if b == b'@' && tail.first() == Some(&b'@') {
            out.extend_from_slice(b"\r\n");
            rest = &tail[1..];
        } else {
            out.push(b);
            rest = tail;
        }
    }
    std::borrow::Cow::Owned(out)
}

fn push_transactions(out: &mut Vec<Transaction>, statement: Statement, status: &TransactionStatus) {
    for entry in statement.entries {
        out.push(build_transaction(
            &entry.line,
            &statement.currency,
            entry.tag86.as_deref(),
            status.clone(),
        ));
    }
}

fn build_transaction(
    line: &StatementLine,
    currency: &str,
    tag86_body: Option<&str>,
    status: TransactionStatus,
) -> Transaction {
    // Sign by the effect on the account: see `Mark`.
    let amount = if line.mark.credits_account() {
        line.amount
    } else {
        -line.amount
    };
    let booking_date = booking_date(line.value_date, line.entry_date);
    let tag = tag86_body.map(tag86::parse).unwrap_or_default();

    // Decision 1: `purpose` is the SVWZ value, as CAMT's `Ustrd`; without
    // keywords it is the whole text (non-SEPA booking).
    let purpose = tag
        .value(Keyword::Svwz)
        .or(tag.plain_purpose.as_deref())
        .map(str::to_owned);
    let sepa: serde_json::Map<String, serde_json::Value> = tag
        .sepa
        .iter()
        .map(|v| (v.keyword.as_str().to_owned(), v.value.clone().into()))
        .collect();
    let other: Vec<serde_json::Value> = tag
        .other
        .iter()
        .map(|f| serde_json::json!({ "code": f.code, "value": f.value }))
        .collect();
    let value = |k| tag.value(k).map(str::to_owned);

    let raw = serde_json::json!({
        "value_date": line.value_date.to_string(),
        "entry_date": line.entry_date.map(|d| d.to_string()),
        "booking_date": booking_date.to_string(),
        "amount": amount.to_string(),
        "currency": currency,
        "funds_code": line.funds_code.map(String::from),
        "transaction_type": line.transaction_type,
        "customer_ref": line.customer_ref,
        "bank_ref": line.bank_ref,
        "supplementary_details": line.supplementary_details,
        "gvc": tag.gvc,
        "prima_nota": tag.primanota,
        "applicant_name": tag.name,
        "applicant_iban": tag.account,
        "applicant_bic": tag.bic,
        "posting_text": tag.posting_text,
        "text_key_extension": tag.text_key_extension,
        // The whole `:86:` after the line join: no text is lost.
        "tag86": tag86_body.map(|b| b.chars().filter(|c| !matches!(c, '\r' | '\n')).collect::<String>()),
        // Every SEPA keyword value as sent, placeholders included.
        "sepa": sepa,
        "other_subfields": other,
    });

    Transaction {
        date: booking_date,
        valuta_date: Some(line.value_date),
        amount,
        currency: Currency::new(currency),
        applicant_name: tag.name.clone(),
        applicant_iban: tag.account.clone().map(Iban::new),
        applicant_bic: tag.bic.clone().map(Bic::new),
        purpose,
        posting_text: tag.posting_text.clone(),
        // Decision 4: NONREF and the like are no reference.
        customer_reference: Some(line.customer_ref.clone()).filter(|r| !formats::absent(r)),
        bank_reference: line.bank_ref.clone(),
        // The `:61:` bank reference is often NONREF or repeated: never a
        // booking ID, whatever the bank profile says.
        booking_id: None,
        gvc: tag.gvc.clone(),
        prima_nota: tag.primanota.clone(),
        purpose_code: value(Keyword::Purp),
        mandate_id: value(Keyword::Mref),
        creditor_id: value(Keyword::Cred),
        end_to_end_id: value(Keyword::Eref),
        debtor_id: value(Keyword::Debt),
        ultimate_debtor: tag.ultimate_debtor().map(str::to_owned),
        ultimate_creditor: tag.ultimate_creditor().map(str::to_owned),
        reversal: line.mark.is_reversal(),
        original_amount: tag
            .amount(Keyword::Oamt)
            .map(|a| (a, Currency::new(currency))),
        fees: tag
            .amount(Keyword::Coam)
            .map(|a| (a, Currency::new(currency))),
        return_reason: tag.return_reason().map(str::to_owned),
        sequence_type: None,
        raw,
        status,
    }
}

/// One MT940/MT942 statement: the bookings and the account currency.
#[derive(Debug)]
struct Statement {
    /// Empty when the statement has neither a currency nor bookings.
    currency: String,
    entries: Vec<Entry>,
    /// MT942 `:13:`/`:13D:` Erstellungszeitpunkt.
    created: Option<BankTime>,
    /// MT942 `:90D:` (debit bookings) and `:90C:` (credit bookings).
    debit_total: Option<Total>,
    credit_total: Option<Total>,
}

/// An MT942 `:90D:`/`:90C:` field (HBCI 2.2 IX.2.9): Anzahl (n..5),
/// Währung (a3), Betrag (d..15).
#[derive(Debug, Clone, PartialEq)]
struct Total {
    line_no: usize,
    tag: &'static str,
    count: usize,
    currency: String,
    amount: Decimal,
}

impl Statement {
    /// The bookings against `:90D:` and `:90C:` (HBCI 2.2 IX.2.9 "Anzahl
    /// und Summe der Soll-/Haben-Buchungen").
    ///
    /// - **Count:** decides completeness. A side whose count differs from
    ///   its `:61:` bookings means the list is not complete: an error.
    /// - **Reversals:** HBCI 2.2 does not say which side RC and RD count
    ///   on. SWIFT counts them by their effect on the account (RC, a
    ///   reversed credit, is a debit entry); a bank may count them by the
    ///   letter of the original booking (RC on the credit side). The counts
    ///   pass when they match under either reading; the sums are compared
    ///   under the one that matched (the effect, when both do).
    /// - **Sum and currency:** a mismatch is a warning. With the count
    ///   right, every booking is listed; a wrong sum or currency says the
    ///   bank's totals are off, not that a booking is missing, as an MT940
    ///   closing balance that does not add up is not checked either.
    fn check_totals(&self) -> Result<()> {
        let totals = [(&self.debit_total, false), (&self.credit_total, true)];
        let totals = totals
            .iter()
            .filter_map(|(t, credits)| t.as_ref().map(|t| (t, *credits)));
        let side = |credit_side: fn(Mark) -> bool, credits: bool| {
            self.entries
                .iter()
                .filter(move |e| credit_side(e.line.mark) == credits)
        };
        let readings: [fn(Mark) -> bool; 2] = [Mark::credits_account, Mark::credit_letter];
        let reading = readings.into_iter().find(|&r| {
            totals
                .clone()
                .all(|(t, credits)| t.count == side(r, credits).count())
        });
        let Some(reading) = reading else {
            let (total, listed) = totals
                .map(|(t, credits)| (t, side(Mark::credits_account, credits).count()))
                .find(|(t, listed)| t.count != *listed)
                .expect("a count differs under every reading");
            return Err(mt940_error(
                total.line_no,
                total.tag,
                &format!(
                    "{} bookings announced, {listed} listed: the list is not complete",
                    total.count
                ),
            ));
        };
        for (total, credits) in totals {
            let sum: Decimal = side(reading, credits).map(|e| e.line.amount).sum();
            if total.amount != sum {
                warn!(
                    "[MT942] line {} (:{}:): sum {} announced, the bookings add up to {sum}",
                    total.line_no, total.tag, total.amount
                );
            }
            if total.count > 0 && total.currency != self.currency {
                warn!(
                    "[MT942] line {} (:{}:): currency {} is not the statement's {}",
                    total.line_no, total.tag, total.currency, self.currency
                );
            }
        }
        Ok(())
    }
}

/// A `:61:` booking with the `:86:` body that follows it.
#[derive(Debug)]
struct Entry {
    line: StatementLine,
    /// Continuation lines joined; `None` when the `:61:` has no `:86:`.
    tag86: Option<String>,
}

/// Soll/Haben-Kennung of a `:61:` (HBCI 2.2 IX.2.8): "C" = Haben, "D" =
/// Soll, "RC" = Storno Haben, "RD" = Storno Soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Credit,
    Debit,
    /// RC: a reversed credit, which debits the account.
    ReversedCredit,
    /// RD: a reversed debit, which credits the account.
    ReversedDebit,
}

impl Mark {
    fn credits_account(self) -> bool {
        matches!(self, Mark::Credit | Mark::ReversedDebit)
    }

    /// The letter of the booking read as the side: C and RC credit, D and
    /// RD debit.
    fn credit_letter(self) -> bool {
        matches!(self, Mark::Credit | Mark::ReversedCredit)
    }

    fn is_reversal(self) -> bool {
        matches!(self, Mark::ReversedCredit | Mark::ReversedDebit)
    }
}

/// The subfields of a `:61:` field (HBCI 2.2 IX.2.8 and IX.2.9).
#[derive(Debug, Clone, PartialEq)]
struct StatementLine {
    value_date: Date,
    /// The entry date in the value date's year; see [`booking_date`].
    entry_date: Option<Date>,
    mark: Mark,
    /// Währungsart: the third letter of the currency, when sent.
    funds_code: Option<char>,
    amount: Decimal,
    /// `N`/`F`/`S` and the three-character Buchungsschlüssel, e.g. `NTRF`.
    transaction_type: String,
    customer_ref: String,
    bank_ref: Option<String>,
    /// The line after the `:61:` (Weitere Informationen), when sent.
    supplementary_details: Option<String>,
}

/// One field: its tag, the value on the tag's line and the lines that
/// continue it.
struct Field<'a> {
    /// 1-based line number in the decoded text, for errors.
    line_no: usize,
    tag: &'a str,
    value: &'a str,
    more: Vec<&'a str>,
}

fn mt940_error(line_no: usize, tag: &str, what: &str) -> FinTSError {
    FinTSError::Mt940(format!("MT940 line {line_no} (:{tag}:): {what}"))
}

/// The fields of `text`, without separators and blank lines. Text before
/// the first field is skipped.
fn fields(text: &str) -> Vec<Field<'_>> {
    let mut out: Vec<Field<'_>> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let mut line = raw.trim_end_matches('\r');
        // A page that ends with the separator "-" and no line break runs into
        // the next page's ":20:".
        if let Some(rest) = line.strip_prefix('-')
            && is_mt940_tag(rest)
        {
            line = rest;
        }
        let t = line.trim();
        if t.is_empty() || t == "-" || t == "--" {
            continue;
        }
        if is_mt940_tag(line)
            && let Some((tag, value)) = line[1..].split_once(':')
        {
            out.push(Field {
                line_no: i + 1,
                tag,
                value,
                more: Vec::new(),
            });
        } else if let Some(field) = out.last_mut() {
            field.more.push(line);
        }
    }
    out
}

/// The statements of `text`, each started by `:20:`.
fn parse_statements(text: &str) -> Result<Vec<Statement>> {
    let mut statements = Vec::new();
    let mut current: Option<Draft> = None;
    for field in fields(text) {
        if field.tag == "20" {
            if let Some(draft) = current.replace(Draft::new(field.line_no)) {
                statements.push(draft.finish()?);
            }
            continue;
        }
        let Some(draft) = current.as_mut() else {
            return Err(mt940_error(
                field.line_no,
                field.tag,
                "field before the first :20:",
            ));
        };
        draft.add(&field)?;
    }
    if let Some(draft) = current {
        statements.push(draft.finish()?);
    }
    if statements.is_empty() && text.lines().any(|l| !matches!(l.trim(), "" | "-" | "--")) {
        return Err(FinTSError::Mt940("MT940: no :20: field".into()));
    }
    Ok(statements)
}

/// A statement being read.
struct Draft {
    /// Line of the `:20:`, for errors.
    line_no: usize,
    opening_currency: Option<String>,
    floor_currency: Option<String>,
    entries: Vec<Entry>,
    created: Option<BankTime>,
    debit_total: Option<Total>,
    credit_total: Option<Total>,
    /// The last field was the `:61:` of the last entry (or its `:86:`).
    after_61: bool,
}

impl Draft {
    fn new(line_no: usize) -> Self {
        Self {
            line_no,
            opening_currency: None,
            floor_currency: None,
            entries: Vec::new(),
            created: None,
            debit_total: None,
            credit_total: None,
            after_61: false,
        }
    }

    fn add(&mut self, field: &Field<'_>) -> Result<()> {
        match field.tag {
            "60F" | "60M" => {
                // Soll/Haben-Kennung (1), Datum (6), Währung (3), Betrag.
                self.opening_currency = Some(currency_at(field, 7)?);
                self.after_61 = false;
            }
            "34F" => {
                // MT942 Mindestbetrag (HBCI 2.2 IX.2.9): Währung (a3),
                // Soll/Haben-Kennung (empty or "D" in the first, "C" in the
                // second), Betrag. One :34F: when the smallest debit and
                // credit are the same, else two. HBCI 2.2 describes it as
                // the smallest amount reported, not a filter, so it says
                // nothing about completeness and is not checked; the
                // second :34F: has the same currency.
                if self.floor_currency.is_none() {
                    self.floor_currency = Some(currency_at(field, 0)?);
                }
                self.after_61 = false;
            }
            "61" => {
                self.entries.push(Entry {
                    line: parse_61(field)?,
                    tag86: None,
                });
                self.after_61 = true;
            }
            "86" => {
                // The first :86: after a :61: is its body; a second one, or
                // one after another field, belongs to no booking.
                if self.after_61
                    && let Some(entry) = self.entries.last_mut()
                    && entry.tag86.is_none()
                {
                    let mut body = field.value.to_string();
                    body.extend(field.more.iter().copied());
                    entry.tag86 = Some(body);
                }
            }
            "13" | "13D" => {
                self.created = creation_time(field);
                self.after_61 = false;
            }
            "90D" => {
                self.debit_total = Some(total(field, "90D")?);
                self.after_61 = false;
            }
            "90C" => {
                self.credit_total = Some(total(field, "90C")?);
                self.after_61 = false;
            }
            _ => self.after_61 = false,
        }
        Ok(())
    }

    fn finish(self) -> Result<Statement> {
        // The totals carry the currency too (ISO 4217, HBCI 2.2 IX.2.9): the
        // last source, for an MT942 without the mandatory :34F:.
        let totals_currency = || {
            [&self.debit_total, &self.credit_total]
                .into_iter()
                .flatten()
                .map(|t| t.currency.clone())
                .next()
        };
        let currency = match self
            .opening_currency
            .or(self.floor_currency)
            .or_else(totals_currency)
        {
            Some(c) => c,
            None if self.entries.is_empty() => String::new(),
            None => {
                return Err(mt940_error(
                    self.line_no,
                    "20",
                    "statement with :61: but no currency in :60F:, :60M:, :34F: or :90D:/:90C:",
                ));
            }
        };
        Ok(Statement {
            currency,
            entries: self.entries,
            created: self.created,
            debit_total: self.debit_total,
            credit_total: self.credit_total,
        })
    }
}

/// MT942 Erstellungszeitpunkt. `:13:` (HBCI 2.2 IX.2.9): Datum JJMMTT and
/// Uhrzeit hhmm, no offset. `:13D:` (SWIFT MT942 since the 2000s; not in
/// HBCI 2.2): the same and the offset from UTC, `+hhmm` or `-hhmm`. Either
/// tag is read in either form (an offset where one is sent, none where
/// not). The time is information, not part of the list: one that does not
/// read is `None` with a warning, and the bookings still count.
fn creation_time(field: &Field<'_>) -> Option<BankTime> {
    let v = field.value.trim();
    let parsed = (|| {
        let digits = |r: std::ops::Range<usize>| {
            v.get(r)
                .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<i32>().ok())
        };
        let date = ymd(
            2000 + digits(0..2)?,
            u32::try_from(digits(2..4)?).ok()?,
            u32::try_from(digits(4..6)?).ok()?,
        )?;
        let time = jiff::civil::Time::new(
            i8::try_from(digits(6..8)?).ok()?,
            i8::try_from(digits(8..10)?).ok()?,
            0,
            0,
        )
        .ok()?;
        let datetime = date.to_datetime(time);
        let offset = match v.len() {
            10 => None,
            15 => {
                let sign = match v.get(10..11)? {
                    "+" => 1,
                    "-" => -1,
                    _ => return None,
                };
                let seconds = (digits(11..13)? * 3600 + digits(13..15)? * 60) * sign;
                Some(jiff::tz::Offset::from_seconds(seconds).ok()?)
            }
            _ => return None,
        };
        Some(BankTime { datetime, offset })
    })();
    if parsed.is_none() {
        warn!(
            "[MT942] line {}: unreadable :{}: creation time, ignored",
            field.line_no, field.tag
        );
    }
    parsed
}

/// An MT942 `:90D:`/`:90C:`: Anzahl (n..5), Währung (a3), Betrag (d..15).
fn total(field: &Field<'_>, tag: &'static str) -> Result<Total> {
    let err = |what: &str| mt940_error(field.line_no, tag, what);
    let v = field.value.trim();
    let digits = v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len());
    let count = v[..digits]
        .parse::<usize>()
        .map_err(|_| err("no count of bookings"))?;
    let currency = v
        .get(digits..digits + 3)
        .filter(|c| c.bytes().all(|b| b.is_ascii_alphabetic()))
        .ok_or_else(|| err("no currency"))?;
    let amount = parse_amount(&v[digits + 3..]).ok_or_else(|| err("invalid amount"))?;
    Ok(Total {
        line_no: field.line_no,
        tag,
        count,
        currency: currency.to_owned(),
        amount,
    })
}

/// The ISO 4217 currency at byte `at` of a balance or floor field.
fn currency_at(field: &Field<'_>, at: usize) -> Result<String> {
    field
        .value
        .get(at..at + 3)
        .filter(|c| c.bytes().all(|b| b.is_ascii_alphabetic()))
        .map(str::to_owned)
        .ok_or_else(|| mt940_error(field.line_no, field.tag, "no currency"))
}

/// Parse a `:61:` field (HBCI 2.2 IX.2.8):
/// `Datum` YYMMDD, `Buchungsdatum` MMDD (optional), `Soll/Haben-Kennung`
/// C/D/RC/RD, `Währungsart` (optional letter), `Betrag` with decimal comma,
/// `N`/`F`/`S` + `Buchungsschlüssel` (3), `Referenz` up to `//`,
/// `Bankreferenz` after `//`, and on the next line `Weitere Informationen`.
/// Lengths are not checked: banks exceed them.
fn parse_61(field: &Field<'_>) -> Result<StatementLine> {
    let err = |what: &str| mt940_error(field.line_no, "61", what);
    let fixed = repair_61_dates(&format!(":61:{}", field.value.trim()));
    let mut s = &fixed[4..];

    let v = take_digits(&mut s, 6).ok_or_else(|| err("no value date YYMMDD"))?;
    let num = |r: std::ops::Range<usize>, d: &str| d[r].parse::<u32>().unwrap_or(0);
    let value_date = ymd(2000 + num(0..2, v) as i32, num(2..4, v), num(4..6, v))
        .ok_or_else(|| err("invalid value date"))?;
    let entry_date = match take_digits(&mut s, 4) {
        Some(e) => Some(
            ymd(i32::from(value_date.year()), num(0..2, e), num(2..4, e))
                .ok_or_else(|| err("invalid entry date"))?,
        ),
        None => None,
    };

    let marks = [
        ("RC", Mark::ReversedCredit),
        ("RD", Mark::ReversedDebit),
        ("C", Mark::Credit),
        ("D", Mark::Debit),
    ];
    let (mark, rest) = marks
        .iter()
        .find_map(|(p, m)| s.strip_prefix(p).map(|r| (*m, r)))
        .ok_or_else(|| err("no debit/credit mark C, D, RC or RD"))?;
    s = rest;

    let funds_code = s.chars().next().filter(char::is_ascii_alphabetic);
    if funds_code.is_some() {
        s = &s[1..];
    }

    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == ','))
        .unwrap_or(s.len());
    let amount = parse_amount(&s[..end]).ok_or_else(|| err("invalid amount"))?;
    s = &s[end..];

    let type_end = s.char_indices().nth(4).map_or(s.len(), |(i, _)| i);
    let transaction_type = &s[..type_end];
    if transaction_type.chars().count() < 4 || !matches!(s.as_bytes()[0], b'N' | b'F' | b'S') {
        return Err(err("no transaction type N, F or S plus three characters"));
    }
    s = &s[type_end..];

    let (customer_ref, bank_ref) = match s.split_once("//") {
        Some((c, b)) => (c, Some(b.trim_end()).filter(|b| !b.is_empty())),
        None => (s, None),
    };
    let supplementary_details =
        Some(field.more.join("\n").trim().to_string()).filter(|d| !d.is_empty());

    Ok(StatementLine {
        value_date,
        entry_date,
        mark,
        funds_code,
        amount,
        transaction_type: transaction_type.to_string(),
        customer_ref: customer_ref.trim_end().to_string(),
        bank_ref: bank_ref.map(str::to_owned),
        supplementary_details,
    })
}

/// Take `n` ASCII digits from the front of `s`.
fn take_digits<'a>(s: &mut &'a str, n: usize) -> Option<&'a str> {
    let head = s.get(..n)?;
    if !head.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    *s = &s[n..];
    Some(head)
}

/// An MT940 amount: digits with one decimal comma (`1000,` or `12,5`),
/// the scale as sent.
fn parse_amount(s: &str) -> Option<Decimal> {
    let (int, frac) = s.split_once(',')?;
    if frac.contains(',') || int.len() + frac.len() == 0 {
        return None;
    }
    let digits: i64 = format!("{int}{frac}").parse().ok()?;
    Decimal::try_new(digits, u32::try_from(frac.len()).ok()?).ok()
}

/// `:20:`, `:28C:`, `:60F:` … — a field tag at the start of the line.
fn is_mt940_tag(line: &str) -> bool {
    let Some(rest) = line.strip_prefix(':') else {
        return false;
    };
    let Some((tag, _)) = rest.split_once(':') else {
        return false;
    };
    (2..=3).contains(&tag.len())
        && tag.as_bytes()[..2].iter().all(u8::is_ascii_digit)
        && tag
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// Clamp invalid month-end dates in a `:61:` line: value date `YYMMDD` at
/// characters 4..10, optional entry date `MMDD` at 10..14.
fn repair_61_dates(line: &str) -> String {
    let digits = |r: std::ops::Range<usize>| {
        line.get(r)
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u32>().ok())
    };
    let (Some(yy), Some(mm), Some(dd)) = (digits(4..6), digits(6..8), digits(8..10)) else {
        return line.to_string();
    };
    let year = 2000 + yy as i32;
    let mut fixed = line.to_string();
    if let Some(day) = clamp_day(year, mm, dd) {
        warn!("[MT940] invalid value date {year}-{mm:02}-{dd:02} in :61:, using day {day}");
        fixed.replace_range(8..10, &format!("{day:02}"));
    }
    if let (Some(emm), Some(edd)) = (digits(10..12), digits(12..14))
        && let Some(day) = clamp_day(year, emm, edd)
    {
        warn!("[MT940] invalid entry date {emm:02}-{edd:02} in :61:, using day {day}");
        fixed.replace_range(12..14, &format!("{day:02}"));
    }
    fixed
}

/// A calendar date from wide integers; `None` when it does not exist.
fn ymd(year: i32, month: u32, day: u32) -> Option<Date> {
    Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()
}

/// The last day of the month when `day` does not exist in it (only for
/// days 29-31 of a valid month); `None` when the date is fine or not a
/// date this repair handles.
fn clamp_day(year: i32, month: u32, day: u32) -> Option<u32> {
    if !(1..=12).contains(&month) || !(29..=31).contains(&day) {
        return None;
    }
    if ymd(year, month, day).is_some() {
        return None;
    }
    (28..day).rev().find(|d| ymd(year, month, *d).is_some())
}

/// The booking date of an MT940 `:61:` line.
///
/// `:61:` carries the value date (YYMMDD) first and an optional entry date
/// (MMDD, no year) second. The entry date is the booking date. `parse_61`
/// gives it the value date's year, which is wrong across New Year (value
/// 2025-12-31, entry 0102 must be 2026-01-02), so shift by a year when the
/// two are more than half a year apart.
fn booking_date(value_date: Date, entry_date: Option<Date>) -> Date {
    let Some(entry) = entry_date else {
        return value_date;
    };
    let gap = (entry - value_date).get_days();
    let year = if gap > 183 {
        i32::from(entry.year()) - 1
    } else if gap < -183 {
        i32::from(entry.year()) + 1
    } else {
        return entry;
    };
    ymd(year, entry.month() as u32, entry.day() as u32).unwrap_or(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Date {
        ymd(y, m, day).unwrap()
    }

    /// A one-page statement of a synthetic account around `lines`, with
    /// opening and closing balance.
    fn statement(lines: &[&str]) -> Vec<u8> {
        let opening = [
            ":20:STARTUMS",
            ":25:12345678/0000004711",
            ":28C:1",
            ":60F:C251229EUR250,00",
        ];
        let closing = ":62F:C260105EUR237,55";
        let all: Vec<&str> = opening
            .iter()
            .chain(lines)
            .chain([&closing])
            .copied()
            .collect();
        all.join("\r\n").into_bytes()
    }

    /// The `:61:` line gives the value date first and the entry (booking)
    /// date as `MMDD` after it; `date` is the entry date.
    #[test]
    fn direct_debit_with_entry_date_and_sepa_fields() {
        // Value 2026-01-02, entered 01-05.
        let data = statement(&[
            ":61:2601020105D12,45NDDTNONREF//TB-0001",
            ":86:105?00LASTSCHRIFT?10123?20EREF+LS-0001MREF+MA-0007CRED+DE?2100ZZZ00000000001SVWZ+Stromabschlag?30TESTDEFFXXX?31DE00123456780000000815?32Teststrom GmbH?60 Januar",
        ]);
        let all = parse_mt940(&data, TransactionStatus::Booked).unwrap();
        let [debit] = all.as_slice() else {
            panic!("{all:?}");
        };
        assert_eq!(debit.date, d(2026, 1, 5));
        assert_eq!(debit.valuta_date, Some(d(2026, 1, 2)));
        assert_eq!(debit.amount.to_string(), "-12.45");
        assert_eq!(
            (debit.gvc.as_deref(), debit.prima_nota.as_deref()),
            (Some("105"), Some("123"))
        );
        assert_eq!(debit.posting_text.as_deref(), Some("LASTSCHRIFT"));
        assert_eq!(debit.bank_reference.as_deref(), Some("TB-0001"));
        // NONREF is the placeholder for "no reference"; raw keeps it.
        assert_eq!(debit.customer_reference, None);
        assert_eq!(debit.raw["customer_ref"], "NONREF");
        assert_eq!(debit.applicant_name.as_deref(), Some("Teststrom GmbH"));
        assert_eq!(
            debit.applicant_iban.as_ref().map(Iban::as_str),
            Some("DE00123456780000000815")
        );
        assert_eq!(
            debit.applicant_bic.as_ref().map(Bic::as_str),
            Some("TESTDEFFXXX")
        );
        // ?20 to ?29 and ?60 to ?63 are one text without separators: the
        // creditor ID cut between ?20 and ?21 is whole again, and the
        // purpose continues in ?60.
        assert_eq!(debit.creditor_id.as_deref(), Some("DE00ZZZ00000000001"));
        assert_eq!(debit.purpose.as_deref(), Some("Stromabschlag Januar"));
        assert_eq!(debit.end_to_end_id.as_deref(), Some("LS-0001"));
        assert_eq!(debit.mandate_id.as_deref(), Some("MA-0007"));
    }

    #[test]
    fn statements_split_across_pages_and_separators() {
        // Two statements, separated by "-", split by the bank in the middle
        // of a :86: line across two touchdown pages.
        let mut all = Mt940Data::new();
        let text = [
            ":20:STARTUMS",
            ":25:12345678/0000001234",
            ":28C:0",
            ":60F:C260101EUR100,00",
            ":61:2601020102D1,00NMSCNONREF",
            ":86:166?00GUTSCHRIFT?20Erster Teil",
            ":62F:C260102EUR99,00",
            "-",
            ":20:STARTUMS",
            ":25:12345678/0000001234",
            ":28C:0",
            ":60F:C260102EUR99,00",
            ":61:2601030103C2,00NMSCNONREF",
            ":86:166?00GUTSCHRIFT?20Zweiter Teil",
            ":62F:C260103EUR101,00",
            "-",
        ]
        .join("\r\n");
        // The page boundary falls inside a :61: line.
        let (a, b) = text.split_at(text.find("NMSCNONREF").unwrap() + 4);
        all.extend(a.as_bytes().to_vec());
        all.extend(b.as_bytes().to_vec());
        let txs = parse_mt940(all.as_bytes(), TransactionStatus::Booked).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].purpose.as_deref(), Some("Erster Teil"));
        assert_eq!(txs[0].customer_reference, None);
        assert_eq!(txs[1].purpose.as_deref(), Some("Zweiter Teil"));
    }

    #[test]
    fn long_86_field_over_six_lines_parses() {
        // DFU style, one subfield per line of at most 27 characters. The
        // grammar takes 6 x 65 characters where each line break counts as
        // one: 15 lines of 385 characters overflow, joined they fit.
        let mut body = vec![
            "105?00SEPA-BASISLASTSCHRIFT".to_string(),
            "?10931".to_string(),
        ];
        for (i, sub) in (20..=29).enumerate() {
            body.push(format!("?{sub}SVWZ-TEIL-{i:02}-ABCDEFGHIJ"));
        }
        body.push("?32Testversorger GmbH".to_string());
        body.push("?60ABWA+Testversorger".to_string());
        let used: usize = body.iter().map(String::len).sum();
        body.push(format!("?61{}", "x".repeat(385 - used - 3)));
        let chars: usize = body.iter().map(String::len).sum();
        assert_eq!(chars, 385);
        assert!(
            chars + body.len() - 1 > 6 * 65,
            "must overflow with line breaks"
        );

        let mut lines = vec![":61:2601020102D5,00NDDTNONREF".to_string()];
        lines.push(format!(":86:{}", body[0]));
        lines.extend(body[1..].iter().cloned());
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let txs = parse_mt940(&statement(&refs), TransactionStatus::Booked).unwrap();
        assert_eq!(txs.len(), 1);
        let p = txs[0].purpose.as_deref().unwrap();
        assert!(p.starts_with("SVWZ-TEIL-00-ABCDEFGHIJSVWZ-TEIL-01"), "{p}");
        assert!(!p.contains("ABWA+"), "{p}");
        assert!(
            txs[0].raw["sepa"]["ABWA"]
                .as_str()
                .unwrap()
                .starts_with("Testversorgerx")
        );
        assert_eq!(txs[0].applicant_name.as_deref(), Some("Testversorger GmbH"));
    }

    #[test]
    fn separator_glued_to_next_page() {
        let page = |amount: &str| {
            statement(&[
                &format!(":61:2601020102D{amount}NMSCNONREF"),
                ":86:166?00TEST?20x",
            ])
        };
        let mut all = Mt940Data::new();
        let mut first = page("1,00");
        first.extend_from_slice(b"\r\n-");
        all.extend(first);
        all.extend(page("2,00"));
        let txs = parse_mt940(all.as_bytes(), TransactionStatus::Booked).unwrap();
        assert_eq!(txs.len(), 2);
    }

    #[test]
    fn long_86_field_is_kept_whole() {
        let long = format!(":86:166?00GUTSCHRIFT?20{}", "x".repeat(500));
        let data = statement(&[":61:2601020102C1,00NMSCNONREF", &long]);
        let txs = parse_mt940(&data, TransactionStatus::Booked).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].purpose.as_deref(), Some("x".repeat(500).as_str()));
    }

    /// Booking with the given `:86:` lines, after a plain `:61:`.
    fn one(indicator: &str, tag86: &[&str]) -> Transaction {
        let mut lines = vec![format!(":61:2601020102{indicator}1,00NMSCNONREF")];
        lines.extend(tag86.iter().map(|s| s.to_string()));
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        parse_mt940(&statement(&refs), TransactionStatus::Booked)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn missing_and_statement_level_86_keep_alignment() {
        let mut data = statement(&[
            ":61:2601020102D1,00NMSCNONREF",
            ":61:2601030103D2,00NMSCNONREF",
            ":86:166?20SVWZ+Zweite",
        ]);
        // A statement-level :86: after :62F: belongs to no booking.
        data.extend_from_slice(b"\r\n:86:Hinweis zum Auszug");
        let txs = parse_mt940(&data, TransactionStatus::Booked).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].purpose, None);
        assert_eq!(txs[0].raw["tag86"], serde_json::Value::Null);
        assert_eq!(txs[1].purpose.as_deref(), Some("Zweite"));
    }

    #[test]
    fn sepa_credit_transfer_fields() {
        // The `:86:` value spans three lines; the purpose continues from
        // ?22 into ?23 in the middle of a number.
        let credit = one(
            "C",
            &[
                ":86:166?00SEPA-GUTSCHRIFT?10456?20EREF+UEB-0042?21KREF+NONREF?22SVWZ+Honorar Nr. 20",
                "?2326-017?24ABWA+Testverlag KG?30TESTDEFFXXX?31DE00123456780000009001",
                "?32Max Mustermann?34166",
            ],
        );
        assert_eq!(credit.gvc.as_deref(), Some("166"));
        assert_eq!(credit.prima_nota.as_deref(), Some("456"));
        assert_eq!(credit.posting_text.as_deref(), Some("SEPA-GUTSCHRIFT"));
        assert_eq!(credit.end_to_end_id.as_deref(), Some("UEB-0042"));
        assert_eq!(credit.purpose.as_deref(), Some("Honorar Nr. 2026-017"));
        // On a credit transfer ABWA names the payer's side: the ultimate
        // debtor.
        assert_eq!(credit.ultimate_debtor.as_deref(), Some("Testverlag KG"));
        assert_eq!(credit.ultimate_creditor, None);
        assert_eq!(credit.applicant_name.as_deref(), Some("Max Mustermann"));
        assert_eq!(
            credit.applicant_iban.as_ref().map(Iban::as_str),
            Some("DE00123456780000009001")
        );
        assert_eq!(
            credit.applicant_bic.as_ref().map(Bic::as_str),
            Some("TESTDEFFXXX")
        );
        assert!(!credit.reversal);
        assert_eq!(
            (credit.mandate_id.as_deref(), credit.creditor_id.as_deref()),
            (None, None)
        );
        // raw keeps the field as sent, the KREF placeholder included.
        assert_eq!(credit.raw["sepa"]["KREF"], "NONREF");
        let tag86 = credit.raw["tag86"].as_str().unwrap();
        assert!(tag86.starts_with("166?00SEPA-GUTSCHRIFT?10456"), "{tag86}");
    }

    #[test]
    fn sepa_direct_debit_fields() {
        let tx = one(
            "D",
            &[
                ":86:105?00SEPA-BASISLASTSCHRIFT?10931?20EREF+NOTPROVIDED?21MREF+M-9?22CRED+DE98ZZZ09999999999",
                "?23SVWZ+Beitrag?24ABWE+Dritte GmbH?32Versicherung AG",
            ],
        );
        assert_eq!(tx.end_to_end_id, None, "NOTPROVIDED is absent");
        assert_eq!(tx.raw["sepa"]["EREF"], "NOTPROVIDED");
        assert_eq!(tx.mandate_id.as_deref(), Some("M-9"));
        assert_eq!(tx.creditor_id.as_deref(), Some("DE98ZZZ09999999999"));
        assert_eq!(tx.purpose.as_deref(), Some("Beitrag"));
        // Direct debit: ABWA is the creditor's side, ABWE the ultimate
        // debtor; only ABWE is given.
        assert_eq!(tx.ultimate_creditor, None);
        assert_eq!(tx.ultimate_debtor.as_deref(), Some("Dritte GmbH"));
        assert_eq!(tx.raw["sepa"]["ABWE"], "Dritte GmbH");
    }

    #[test]
    fn unstructured_gvc_9xx_is_all_purpose() {
        let tx = one("D", &[":86:999Gebuehren fuer Kontofuehrung 01/26"]);
        assert_eq!(tx.gvc.as_deref(), Some("999"));
        assert_eq!(
            tx.purpose.as_deref(),
            Some("Gebuehren fuer Kontofuehrung 01/26")
        );
        assert_eq!(tx.applicant_name, None);
    }

    #[test]
    fn no_keywords_means_whole_text_is_purpose() {
        let tx = one(
            "D",
            &[":86:005?00UEBERWEISUNG?20Miete Januar?21 und Nebenkosten"],
        );
        assert_eq!(tx.purpose.as_deref(), Some("Miete Januar und Nebenkosten"));
        assert_eq!(tx.end_to_end_id, None);
    }

    #[test]
    fn umlauts_are_kept() {
        let mut data = statement(&[
            ":61:2601020102D1,00NMSCNONREF",
            ":86:166?00\u{FC}BERWEISUNG?20SVWZ+Gr\u{FC}\u{DF}e",
        ]);
        // Windows-1252: ü = 0xFC, ß = 0xDF.
        let text = String::from_utf8(std::mem::take(&mut data)).unwrap();
        let bytes: Vec<u8> = text
            .chars()
            .map(|c| match c {
                '\u{FC}' => 0xFC,
                '\u{DF}' => 0xDF,
                c => c as u8,
            })
            .collect();
        let tx = parse_mt940(&bytes, TransactionStatus::Booked)
            .unwrap()
            .remove(0);
        assert_eq!(tx.posting_text.as_deref(), Some("\u{FC}BERWEISUNG"));
        assert_eq!(tx.purpose.as_deref(), Some("Gr\u{FC}\u{DF}e"));
    }

    #[test]
    fn debit_credit_mark_sets_the_amount_sign() {
        // FinTS Finanzdatenformate B.8 `:61:`: C and RD credit the account,
        // D and RC debit it; the third currency letter is optional.
        for (mark, negative) in [
            ("C", false),
            ("D", true),
            ("RC", true),
            ("RD", false),
            ("CR", false),
            ("DR", true),
            ("RCR", true),
            ("RDR", false),
        ] {
            let tx = one(mark, &[":86:166?00TEST?20Zeichen"]);
            let expected = if negative { "-1.00" } else { "1.00" };
            assert_eq!(tx.amount.to_string(), expected, "mark {mark}");
        }
    }

    #[test]
    fn return_keeps_the_roles_of_the_original_payment() {
        // ABWA/ABWE of an R-transaction refer to the original payment
        // (footnote 33): a returned transfer has ABWA as ultimate debtor.
        let tx = one(
            "RD",
            &[":86:109?20SVWZ+RETURN AC04?21ABWA+Wir selbst?22ABWE+Original GmbH"],
        );
        assert_eq!(tx.purpose.as_deref(), Some("RETURN AC04"));
        assert_eq!(tx.ultimate_debtor.as_deref(), Some("Wir selbst"));
        assert_eq!(tx.ultimate_creditor.as_deref(), Some("Original GmbH"));
        assert_eq!(tx.return_reason.as_deref(), Some("AC04"));
    }

    #[test]
    fn reversal_marks_of_61() {
        for (mark, reversal) in [("C", false), ("D", false), ("RC", true), ("RD", true)] {
            let tx = one(mark, &[":86:166?00TEST?20Zeichen"]);
            assert_eq!(tx.reversal, reversal, "mark {mark}");
        }
    }

    #[test]
    fn debtor_id_and_amounts_from_keywords() {
        let tx = one(
            "RD",
            &[":86:159?20DEBT+DEBTORID-1?21SVWZ+RETURN MD06?22OAMT+12,50?23COAM+1,20"],
        );
        assert_eq!(tx.debtor_id.as_deref(), Some("DEBTORID-1"));
        assert_eq!(
            tx.original_amount,
            Some((Decimal::new(1250, 2), Currency::new("EUR")))
        );
        assert_eq!(tx.fees, Some((Decimal::new(120, 2), Currency::new("EUR"))));
        assert_eq!(tx.return_reason.as_deref(), Some("MD06"));
        assert_eq!(tx.sequence_type, None);
        let plain = one("C", &[":86:166?20SVWZ+Text?21DEBT+NONREF"]);
        assert_eq!(plain.debtor_id, None);
        assert_eq!(plain.original_amount, None);
        assert_eq!(plain.fees, None);
    }

    #[test]
    fn customer_reference_is_the_61_value() {
        let data = statement(&[
            ":61:2601020102D1,00NMSCKUNDENREF-1//BANKREF",
            ":86:166?20Text",
        ]);
        let tx = parse_mt940(&data, TransactionStatus::Booked)
            .unwrap()
            .remove(0);
        assert_eq!(tx.customer_reference.as_deref(), Some("KUNDENREF-1"));
        assert_eq!(tx.bank_reference.as_deref(), Some("BANKREF"));
    }

    #[test]
    fn invalid_february_dates_are_clamped() {
        // 2025 is no leap year; 30 February is used for month-end postings.
        let data = statement(&[
            ":61:2502290228D1,00NMSCNONREF",
            ":86:166?00TEST?20Feb 29",
            ":61:2502300230D1,00NMSCNONREF",
            ":86:166?00TEST?20Feb 30",
            ":61:2402290229D1,00NMSCNONREF",
            ":86:166?00TEST?20Leap year",
        ]);
        let txs = parse_mt940(&data, TransactionStatus::Booked).unwrap();
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].valuta_date, Some(d(2025, 2, 28)));
        assert_eq!(txs[1].valuta_date, Some(d(2025, 2, 28)));
        assert_eq!(txs[1].date, d(2025, 2, 28));
        assert_eq!(txs[2].valuta_date, Some(d(2024, 2, 29)));
        assert_eq!(txs[2].date, d(2024, 2, 29));
    }

    #[test]
    fn repairs_leave_odd_lines_alone() {
        assert_eq!(repair_61_dates(":61:"), ":61:");
        assert_eq!(repair_61_dates(":61:25ü"), ":61:25ü");
        assert_eq!(repair_61_dates(":61:2513310101"), ":61:2513310101");
        assert!(is_mt940_tag(":60F:C1") && is_mt940_tag(":86:x") && !is_mt940_tag(":x:"));
        assert!(!is_mt940_tag("?20:abc") && !is_mt940_tag(":2:"));
    }

    proptest::proptest! {
        #[test]
        fn mt940_never_panics_on_random_bytes(data in proptest::collection::vec(proptest::num::u8::ANY, 0..512)) {
            let _ = parse_mt940(&data, TransactionStatus::Booked);
        }

        #[test]
        fn mt940_never_panics_on_field_shaped_input(
            parts in proptest::collection::vec(
                proptest::prop_oneof![
                    proptest::strategy::Just(":20:STARTUMS\r\n".to_string()),
                    proptest::strategy::Just(":25:1/2\r\n:28C:0\r\n".to_string()),
                    proptest::strategy::Just(":60F:C250101EUR1,\r\n".to_string()),
                    proptest::strategy::Just(":61:".to_string()),
                    proptest::strategy::Just(":86:".to_string()),
                    proptest::strategy::Just("?20".to_string()),
                    proptest::strategy::Just(":62F:C250101EUR1,\r\n".to_string()),
                    proptest::strategy::Just("\r\n-\r\n".to_string()),
                    "[0-9CDN,]{0,14}",
                    "[ÄÖÜß€\\x00-\\x7f]{0,8}",
                ],
                0..24,
            )
        ) {
            let _ = parse_mt940(parts.concat().as_bytes(), TransactionStatus::Booked);
        }
    }

    /// The entry date is optional in `:61:`; without it the booking date
    /// is the value date.
    #[test]
    fn booking_without_entry_date() {
        let data = statement(&[
            ":61:260104C75,10NTRFNONREF",
            ":86:152?00DAUERAUFTRAG?20Taschengeld",
        ]);
        let all = parse_mt940(&data, TransactionStatus::Booked).unwrap();
        let standing_order = &all[0];
        assert_eq!(standing_order.date, d(2026, 1, 4));
        assert_eq!(standing_order.valuta_date, Some(d(2026, 1, 4)));
        assert_eq!(standing_order.amount.to_string(), "75.10");
        assert_eq!(standing_order.purpose.as_deref(), Some("Taschengeld"));
    }

    /// The entry date has no year: it is the one of the value date, or the
    /// year next to it when the two dates lie across a new year.
    #[test]
    fn entry_date_year() {
        for (value, entry_month_day, booked) in [
            (d(2026, 12, 30), d(2026, 1, 4), d(2027, 1, 4)),
            (d(2027, 1, 4), d(2027, 12, 30), d(2026, 12, 30)),
            (d(2026, 6, 15), d(2026, 6, 12), d(2026, 6, 12)),
            (d(2026, 6, 12), d(2026, 6, 15), d(2026, 6, 15)),
        ] {
            assert_eq!(
                booking_date(value, Some(entry_month_day)),
                booked,
                "{value}"
            );
        }
        assert_eq!(booking_date(d(2026, 6, 15), None), d(2026, 6, 15));
    }

    /// `:61:` with the value after the tag and the continuation lines.
    fn line61(value: &str, more: &[&str]) -> Result<StatementLine> {
        parse_61(&Field {
            line_no: 7,
            tag: "61",
            value,
            more: more.to_vec(),
        })
    }

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn field_61_every_subfield() {
        let l = line61("2601150114DR1234,56NTRFKREF-1//BANK-9", &["/OCMT/EUR1,00/"]).unwrap();
        assert_eq!(l.value_date, d(2026, 1, 15));
        assert_eq!(l.entry_date, Some(d(2026, 1, 14)));
        assert_eq!(l.mark, Mark::Debit);
        assert_eq!(l.funds_code, Some('R'));
        assert_eq!(l.amount, dec("1234.56"));
        assert_eq!(l.transaction_type, "NTRF");
        assert_eq!(l.customer_ref, "KREF-1");
        assert_eq!(l.bank_ref.as_deref(), Some("BANK-9"));
        assert_eq!(l.supplementary_details.as_deref(), Some("/OCMT/EUR1,00/"));
    }

    #[test]
    fn field_61_marks() {
        for (value, mark, funds) in [
            ("260115C1,00NMSCNONREF", Mark::Credit, None),
            ("260115D1,00NMSCNONREF", Mark::Debit, None),
            ("260115RC1,00NMSCNONREF", Mark::ReversedCredit, None),
            ("260115RD1,00NMSCNONREF", Mark::ReversedDebit, None),
            ("260115RCR1,00NMSCNONREF", Mark::ReversedCredit, Some('R')),
            ("260115RDM1,00NMSCNONREF", Mark::ReversedDebit, Some('M')),
        ] {
            let l = line61(value, &[]).unwrap();
            assert_eq!((l.mark, l.funds_code), (mark, funds), "{value}");
            assert_eq!(l.amount, dec("1.00"));
        }
        assert!(Mark::ReversedDebit.credits_account() && Mark::Credit.credits_account());
        assert!(!Mark::ReversedCredit.credits_account() && !Mark::Debit.credits_account());
    }

    #[test]
    fn field_61_optional_parts() {
        // No entry date, no bank reference, no supplementary line.
        let l = line61("251230C100,NMSCNONREF", &[]).unwrap();
        assert_eq!(l.value_date, d(2025, 12, 30));
        assert_eq!(l.entry_date, None);
        assert_eq!(l.funds_code, None);
        assert_eq!(l.amount, dec("100"));
        assert_eq!(l.customer_ref, "NONREF");
        assert_eq!(l.bank_ref, None);
        assert_eq!(l.supplementary_details, None);
        // An empty bank reference and an empty customer reference.
        let l = line61("251230C,5S999//", &[]).unwrap();
        assert_eq!(l.amount, dec("0.5"));
        assert_eq!(l.transaction_type, "S999");
        assert_eq!(l.customer_ref, "");
        assert_eq!(l.bank_ref, None);
    }

    #[test]
    fn field_61_entry_date_takes_the_value_year() {
        // Value 2025-12-31, entry 0102: the line keeps the value year, the
        // booking date moves to 2026.
        let l = line61("2512310102C1,00NMSCNONREF", &[]).unwrap();
        assert_eq!(l.entry_date, Some(d(2025, 1, 2)));
        let data = statement(&[":61:2512310102C1,00NMSCNONREF"]);
        let tx = &parse_mt940(&data, TransactionStatus::Booked).unwrap()[0];
        assert_eq!(tx.date, d(2026, 1, 2));
        assert_eq!(tx.valuta_date, Some(d(2025, 12, 31)));
        assert_eq!(tx.raw["entry_date"], "2025-01-02");
    }

    #[test]
    fn field_61_errors_name_the_line_and_the_field() {
        for (value, what) in [
            ("", "no value date"),
            ("26011", "no value date"),
            ("261301C1,00NMSC", "invalid value date"),
            ("2601151340C1,00NMSC", "invalid entry date"),
            ("260115X1,00NMSC", "no debit/credit mark"),
            ("260115C100NMSC", "invalid amount"),
            ("260115C1,0,0NMSC", "invalid amount"),
            ("260115C,NMSC", "invalid amount"),
            ("260115C1,00", "no transaction type"),
            ("260115C1,00XMSCNONREF", "no transaction type"),
            ("260115C1,00NMS", "no transaction type"),
        ] {
            let e = line61(value, &[]).unwrap_err().to_string();
            assert!(
                e.contains("line 7 (:61:)") && e.contains(what),
                "{value}: {e}"
            );
        }
        // A malformed :61: fails the whole call.
        let data = statement(&[":61:2601020102C1,00NMSCNONREF", ":61:260102C1,00"]);
        let e = parse_mt940(&data, TransactionStatus::Booked).unwrap_err();
        assert!(e.to_string().contains("line 6 (:61:)"), "{e}");
    }

    #[test]
    fn mt942_without_opening_balance_takes_the_floor_currency() {
        let data = [
            ":20:STARTDISPE",
            ":25:12345678/0000001234",
            ":28C:0",
            ":34F:EURD0,",
            ":34F:EURC0,",
            ":13D:2601021230+0100",
            ":61:2601020102D3,00NDDTNONREF",
            ":86:105?00LASTSCHRIFT?20SVWZ+Vorgemerkt",
            ":90D:1EUR3,00",
            ":90C:0EUR0,",
            "-",
        ]
        .join("\r\n");
        let txs = parse_mt940(data.as_bytes(), TransactionStatus::Pending).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].currency.as_str(), "EUR");
        assert_eq!(txs[0].amount, dec("-3.00"));
        assert_eq!(txs[0].purpose.as_deref(), Some("Vorgemerkt"));
        assert_eq!(txs[0].status, TransactionStatus::Pending);

        // An MT942 without bookings needs no currency.
        let empty =
            ":20:STARTDISPE\r\n:25:12345678/0000001234\r\n:28C:0\r\n:13D:2601021230+0100\r\n-";
        assert!(
            parse_mt940(empty.as_bytes(), TransactionStatus::Pending)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn statement_with_bookings_needs_a_currency() {
        let data = ":20:STARTUMS\r\n:25:12345678/0000001234\r\n:61:260102C1,00NMSCNONREF";
        let e = parse_mt940(data.as_bytes(), TransactionStatus::Booked).unwrap_err();
        assert!(e.to_string().contains("line 1 (:20:)"), "{e}");
        let data = statement(&[]);
        let bad = String::from_utf8(data)
            .unwrap()
            .replace(":60F:C251229EUR", ":60F:C2512291,0");
        let e = parse_mt940(bad.as_bytes(), TransactionStatus::Booked).unwrap_err();
        assert!(e.to_string().contains("(:60F:): no currency"), "{e}");
    }

    #[test]
    fn multiple_statements_keep_their_currency() {
        let mut data = statement(&[":61:2601020102C1,00NMSCNONREF"]);
        data.extend_from_slice(b"\r\n-\r\n");
        let second = String::from_utf8(statement(&[
            ":61:2601020102D2,00NMSCNONREF",
            ":61:2601030103C3,00NMSCNONREF",
        ]))
        .unwrap()
        .replace("EUR", "USD")
        .replace(":60F:", ":60M:");
        data.extend_from_slice(second.as_bytes());
        let txs = parse_mt940(&data, TransactionStatus::Booked).unwrap();
        let got: Vec<_> = txs
            .iter()
            .map(|t| (t.currency.as_str().to_string(), t.amount.to_string()))
            .collect();
        assert_eq!(
            got,
            [("EUR", "1.00"), ("USD", "-2.00"), ("USD", "3.00")]
                .map(|(c, a)| (c.to_string(), a.to_string()))
        );
    }

    #[test]
    fn separator_variants() {
        let one = String::from_utf8(statement(&[":61:2601020102C1,00NMSCNONREF"])).unwrap();
        for sep in [
            "\r\n-\r\n",
            "\r\n--\r\n",
            "\r\n\r\n",
            "\n-\n",
            "\r\n-",
            "\r\n- \r\n\r\n",
        ] {
            let data = format!("{one}{sep}{one}{sep}");
            let txs = parse_mt940(data.as_bytes(), TransactionStatus::Booked).unwrap();
            assert_eq!(txs.len(), 2, "{sep:?}");
        }
        // Text before the first field is skipped; a field before :20: fails.
        let data = format!("\r\n{one}");
        assert_eq!(
            parse_mt940(data.as_bytes(), TransactionStatus::Booked)
                .unwrap()
                .len(),
            1
        );
        let data = format!(":25:12345678/0000001234\r\n{one}");
        let e = parse_mt940(data.as_bytes(), TransactionStatus::Booked).unwrap_err();
        assert!(
            e.to_string()
                .contains("(:25:): field before the first :20:"),
            "{e}"
        );
        // Separators and blank lines only: no statement; other text
        // without a field is an error.
        for empty in [&b"-\r\n"[..], b"\r\n \r\n", b"\r\n-", b"--\n-\n"] {
            let txs = parse_mt940(empty, TransactionStatus::Booked).unwrap();
            assert!(txs.is_empty(), "{empty:?}");
            assert!(parse_mt942(empty).unwrap().transactions.is_empty());
        }
        assert!(parse_mt940(b"STARTUMS\r\n-", TransactionStatus::Booked).is_err());
    }

    #[test]
    fn spec_style_mt942_with_funds_code_and_supplementary_line() {
        // Shape of the HBCI 2.2 IX.2.9 example, synthetic values.
        let data = [
            ":20:1234567",
            ":21:9876543210",
            ":25:12345678/0000001234",
            ":28C:4/1",
            ":34F:EURD10,",
            ":34F:EURC20,",
            ":13:2601031245",
            ":61:2601020102DR10,NSTONONREF//55555",
            "/OCMT/EUR10,00/",
            ":86:008?00DAUERAUFTRAG?100599?20Miete Jan",
            "uar?32TESTER",
            ":61:2601020102CR20,NTRFNONREF//55555",
            ":86:051?00UEBERWEISUNG?20Lohn",
            ":90D:1EUR10,",
            ":90C:1EUR20,",
            "-",
        ]
        .join("\r\n");
        let txs = parse_mt940(data.as_bytes(), TransactionStatus::Pending).unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount, dec("-10"));
        assert_eq!(txs[0].raw["funds_code"], "R");
        assert_eq!(txs[0].raw["transaction_type"], "NSTO");
        assert_eq!(txs[0].raw["supplementary_details"], "/OCMT/EUR10,00/");
        assert_eq!(txs[0].bank_reference.as_deref(), Some("55555"));
        assert_eq!(txs[0].purpose.as_deref(), Some("Miete Januar"));
        assert_eq!(txs[0].applicant_name.as_deref(), Some("TESTER"));
        assert_eq!(txs[1].amount, dec("20"));
        assert_eq!(txs[1].raw["supplementary_details"], serde_json::Value::Null);
    }

    /// An MT942 with `lines` between the header and the separator.
    fn mt942(lines: &[&str]) -> String {
        let mut text = vec![":20:STARTDISPE", ":25:12345678/0000001234", ":28C:0"];
        text.extend_from_slice(lines);
        text.push("-");
        text.join("\r\n")
    }

    #[test]
    fn mt942_creation_time_with_and_without_offset() {
        let at = |data: &str| parse_mt942(data.as_bytes()).unwrap().created;
        let t = at(&mt942(&[":34F:EUR0,", ":13D:2601021230+0100"])).unwrap();
        assert_eq!(t.datetime, jiff::civil::date(2026, 1, 2).at(12, 30, 0, 0));
        assert_eq!(t.offset, Some(jiff::tz::offset(1)));
        assert_eq!(
            t.timestamp(),
            Some("2026-01-02T11:30:00Z".parse::<jiff::Timestamp>().unwrap())
        );
        let t = at(&mt942(&[":13D:2601021230-0230"])).unwrap();
        assert_eq!(t.offset.unwrap().seconds(), -9000);

        // HBCI 2.2 IX.2.9 `:13:`: no offset, so no instant.
        let t = at(&mt942(&[":13:2601031245"])).unwrap();
        assert_eq!(t.datetime, jiff::civil::date(2026, 1, 3).at(12, 45, 0, 0));
        assert_eq!((t.offset, t.timestamp()), (None, None));

        // Unreadable times are no time; the bookings still parse.
        // Either tag in either form.
        let t = at(&mt942(&[":13:2601021230+0100"])).unwrap();
        assert_eq!(t.offset, Some(jiff::tz::offset(1)));
        let t = at(&mt942(&[":13D:2601021230"])).unwrap();
        assert_eq!(t.datetime, jiff::civil::date(2026, 1, 2).at(12, 30, 0, 0));
        assert_eq!(t.offset, None);

        for bad in [
            ":13:26010312",
            ":13:2601032460",
            ":13D:2601021230*0100",
            ":13D:2601021230+01",
            ":13D:2613021230+0100",
        ] {
            let data = mt942(&[":34F:EUR0,", bad, ":61:2601020102C1,00NMSCNONREF"]);
            let p = parse_mt942(data.as_bytes()).unwrap();
            assert_eq!((p.created, p.transactions.len()), (None, 1), "{bad}");
        }
        assert_eq!(at(&mt942(&[])), None);

        // Joined pages: the oldest statement's time.
        let data = [
            mt942(&[":13D:2601021230+0100"]),
            mt942(&[":13D:2601021100+0100"]),
            mt942(&[":13D:2601021300+0100"]),
        ]
        .concat();
        assert_eq!(
            at(&data).unwrap().datetime,
            jiff::civil::date(2026, 1, 2).at(11, 0, 0, 0)
        );
    }

    #[test]
    fn mt942_total_counts_must_match_the_bookings() {
        let parse = |totals: &[&str]| {
            let mut lines = vec![
                ":34F:EURD0,",
                ":34F:EURC0,",
                ":13D:2601021230+0100",
                ":61:2601020102D3,00NDDTNONREF",
                ":86:105?00LASTSCHRIFT?20SVWZ+Eins",
                ":61:2601020102D4,50NDDTNONREF",
                ":61:2601020102C10,NTRFNONREF",
                // A reversed credit debits the account: a debit entry.
                ":61:2601020102RC1,00NTRFNONREF",
                // A reversed debit credits it: a credit entry.
                ":61:2601020102RD2,00NDDTNONREF",
            ];
            lines.extend_from_slice(totals);
            parse_mt942(mt942(&lines).as_bytes()).map(|p| p.transactions.len())
        };
        assert_eq!(parse(&[":90D:3EUR8,50", ":90C:2EUR12,00"]).unwrap(), 5);
        // Both totals are optional (K).
        assert_eq!(parse(&[]).unwrap(), 5);
        assert_eq!(parse(&[":90C:2EUR12,"]).unwrap(), 5);

        // A sum or currency that does not match is a warning: the count
        // says every booking is listed.
        assert_eq!(parse(&[":90D:3EUR8,49", ":90C:2EUR12,01"]).unwrap(), 5);
        assert_eq!(parse(&[":90D:3USD8,50"]).unwrap(), 5);

        for (totals, what) in [
            (
                &[":90D:4EUR8,50"][..],
                "(:90D:): 4 bookings announced, 3 listed: the list is not complete",
            ),
            (&[":90D:3EUR8,50", ":90C:1EUR10,"], "(:90C:): 1 bookings"),
            (
                &[":90D:2EUR7,50"],
                "(:90D:): 2 bookings announced, 3 listed",
            ),
            (&[":90D:EUR8,50"], "(:90D:): no count"),
            (&[":90D:3E8,50"], "(:90D:): no currency"),
            (&[":90D:3EUR8.50"], "(:90D:): invalid amount"),
        ] {
            let e = parse(totals).unwrap_err().to_string();
            assert!(e.contains(what), "{totals:?}: {e}");
        }

        // An empty list with zero totals is complete.
        let data = mt942(&[":13D:2601021230+0100", ":90D:0EUR0,", ":90C:0EUR0,"]);
        assert!(
            parse_mt942(data.as_bytes())
                .unwrap()
                .transactions
                .is_empty()
        );
        // Each statement is checked against its own totals.
        let page = |amount: &str| {
            mt942(&[
                ":34F:EUR0,",
                &format!(":61:2601020102D{amount}NMSCNONREF"),
                &format!(":90D:1EUR{amount}"),
            ])
        };
        let data = [page("1,00"), page("2,00")].concat();
        assert_eq!(parse_mt942(data.as_bytes()).unwrap().transactions.len(), 2);
    }

    #[test]
    fn mt942_reversals_count_on_either_side() {
        let parse = |totals: &[&str]| {
            let mut lines = vec![
                ":34F:EUR0,",
                ":61:2601020102D3,00NDDTNONREF",
                ":61:2601020102C10,NTRFNONREF",
                ":61:2601020102RC1,00NTRFNONREF",
            ];
            lines.extend_from_slice(totals);
            parse_mt942(mt942(&lines).as_bytes()).map(|p| p.transactions)
        };
        // By the effect on the account (SWIFT): RC is a debit entry.
        let txs = parse(&[":90D:2EUR4,00", ":90C:1EUR10,"]).unwrap();
        assert_eq!(txs[2].amount, dec("-1"));
        assert!(txs[2].reversal);
        // By the letter of the original booking: RC on the credit side.
        assert_eq!(parse(&[":90D:1EUR3,00", ":90C:2EUR11,"]).unwrap().len(), 3);
        assert_eq!(parse(&[":90C:2EUR11,"]).unwrap().len(), 3);
        // The two readings are not mixed: a list the counts fit under
        // neither reading is not complete.
        let e = parse(&[":90D:2EUR4,00", ":90C:2EUR11,"]).unwrap_err();
        assert!(
            e.to_string()
                .contains("(:90C:): 2 bookings announced, 1 listed"),
            "{e}"
        );
    }

    #[test]
    fn mt942_without_floor_takes_the_totals_currency() {
        let data = mt942(&[
            ":13:2601031245",
            ":61:2601020102C5,NMSCNONREF",
            ":90D:0CHF0,",
            ":90C:1CHF5,",
        ]);
        let p = parse_mt942(data.as_bytes()).unwrap();
        assert_eq!(p.transactions[0].currency.as_str(), "CHF");
        // Without any currency the statement fails.
        let data = mt942(&[":61:2601020102C5,NMSCNONREF"]);
        let e = parse_mt942(data.as_bytes()).unwrap_err();
        assert!(e.to_string().contains("no currency"), "{e}");
    }

    #[test]
    fn at_signs_are_line_breaks() {
        let data = mt942(&[
            ":34F:EUR0,",
            ":13D:2601021230+0100",
            ":61:2601020102D3,00NDDTNONREF",
            ":86:105?00LASTSCHRIFT?20SVWZ+Eins",
            ":90D:1EUR3,00",
        ])
        .replace("\r\n", "@@");
        let p = parse_mt942(data.as_bytes()).unwrap();
        assert_eq!(p.transactions.len(), 1);
        assert_eq!(p.transactions[0].purpose.as_deref(), Some("Eins"));
        assert!(p.created.is_some());
        // A miscount behind `@@` is still caught.
        let bad = data.replace(":90D:1EUR", ":90D:2EUR");
        assert!(parse_mt942(bad.as_bytes()).is_err());
        // MT940 too; recompute_totals reads the same lines.
        let booked = String::from_utf8(statement(&[":61:2601020102C1,00NMSCNONREF"]))
            .unwrap()
            .replace("\r\n", "@@")
            .replace('\n', "@@");
        let txs = parse_mt940(booked.as_bytes(), TransactionStatus::Booked).unwrap();
        assert_eq!(txs.len(), 1);
        let shaped = data.replace(":90D:1EUR3,00", ":90D:1EUR9,00");
        let fixed = recompute_totals(shaped.as_bytes());
        assert!(parse_mt942(&fixed).is_ok());
        assert!(String::from_utf8(fixed).unwrap().contains(":90D:1EUR3"));
    }

    #[test]
    fn mt942_single_floor_line_gives_the_currency() {
        // One :34F: without Soll/Haben-Kennung: the floor for both sides.
        let data = mt942(&[
            ":34F:USD0,",
            ":13:2601031245",
            ":61:2601020102C5,NMSCNONREF",
            ":90C:1USD5,",
        ]);
        let p = parse_mt942(data.as_bytes()).unwrap();
        assert_eq!(p.transactions[0].currency.as_str(), "USD");
        assert_eq!(p.transactions[0].status, TransactionStatus::Pending);
    }
}
