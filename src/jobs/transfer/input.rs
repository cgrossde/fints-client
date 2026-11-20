//! A SEPA credit transfer as the caller describes it, and its checks
//! (docs/payments-design.md section 2).
//!
//! [`Transfer::check`] runs before anything is sent and returns a
//! [`CheckedTransfer`]: the parsed IBANs and BICs, the resolved debtor name
//! and the final end-to-end ID. It rejects; it never repairs text
//! ([`crate::formats::sepa::transliterate`] is there for a caller that wants
//! to offer a cleaned text).

use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::formats::pain001::{Pain001Input, generate_reference};
use crate::formats::sepa::{Invalid, check_reference, first_invalid_char};
use crate::protocol::Account;
use crate::types::{Bic, Iban};

/// Longest name of a party (Anlage 3 2.1 "Namen für beteiligte Parteien").
const MAX_NAME: usize = 70;
/// Longest unstructured remittance information (Anlage 3 2.2.1.7).
const MAX_PURPOSE: usize = 140;
/// Largest amount of an SCT / SCT Inst (Anlage 3 2.3.3).
fn max_amount() -> Decimal {
    Decimal::new(99_999_999_999, 2)
}

/// One SEPA credit transfer in EUR from an own account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    /// The debtor account; must have an IBAN.
    pub from: Account,
    /// The debtor name; `None`: the UPD account holder name.
    pub debtor_name: Option<String>,
    /// At most 70 characters of the SEPA text set.
    pub payee_name: String,
    /// Checked by [`Iban::parse`]; spaces and lower case are accepted.
    pub payee_iban: String,
    /// 8 or 11 characters; `None`: IBAN-only.
    pub payee_bic: Option<String>,
    /// EUR, greater than zero, at most 999 999 999.99, at most two decimals.
    pub amount: Decimal,
    /// Unstructured remittance information, at most 140 characters of the
    /// SEPA text set; may be empty.
    pub purpose: String,
    /// At most 35 characters of the SEPA reference set; `None`: the library
    /// generates a unique one in [`Transfer::check`] (read it from
    /// [`CheckedTransfer::end_to_end_id`] before sending).
    pub end_to_end_id: Option<String>,
    /// Standard or instant transfer.
    pub instant: Instant,
}

/// Whether to send the transfer as an instant transfer (SCT Inst, HKIPZ).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Instant {
    /// Standard transfer (HKCCS).
    No,
    /// Instant when the bank offers it, else standard; decided before sending.
    Prefer,
    /// Instant or nothing.
    Require,
}

/// Which field of a [`Transfer`] is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferField {
    From,
    DebtorName,
    PayeeName,
    PayeeIban,
    PayeeBic,
    Amount,
    Purpose,
    EndToEndId,
}

impl fmt::Display for TransferField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::From => "from",
            Self::DebtorName => "debtor_name",
            Self::PayeeName => "payee_name",
            Self::PayeeIban => "payee_iban",
            Self::PayeeBic => "payee_bic",
            Self::Amount => "amount",
            Self::Purpose => "purpose",
            Self::EndToEndId => "end_to_end_id",
        })
    }
}

/// Why a field of a [`Transfer`] is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidReason {
    /// A text, reference, IBAN or BIC that breaks the SEPA rules.
    Sepa(Invalid),
    /// The amount is zero or negative.
    NotPositive,
    /// The amount exceeds 999 999 999.99.
    TooLarge,
    /// The amount has more than two decimals.
    Precision,
    /// The debtor account has no IBAN (addressed nationally only).
    NoIban,
    /// No debtor name given and none in the UPD.
    NoName,
}

impl From<Invalid> for InvalidReason {
    fn from(e: Invalid) -> Self {
        Self::Sepa(e)
    }
}

impl fmt::Display for InvalidReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sepa(e) => e.fmt(f),
            Self::NotPositive => f.write_str("must be greater than zero"),
            Self::TooLarge => f.write_str("must not exceed 999999999.99"),
            Self::Precision => f.write_str("at most two decimals"),
            Self::NoIban => f.write_str("the account has no IBAN"),
            Self::NoName => f.write_str("no account holder name known; give one"),
        }
    }
}

/// A [`Transfer`] that cannot be sent: the caller's data, nothing was sent.
/// Becomes [`FinTSError::InvalidTransfer`](crate::FinTSError::InvalidTransfer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransfer {
    pub field: TransferField,
    pub reason: InvalidReason,
}

impl fmt::Display for InvalidTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for InvalidTransfer {}

/// A [`Transfer`] that passed [`Transfer::check`]: what goes into the
/// pain.001, normalised.
#[derive(Debug, Clone)]
pub struct CheckedTransfer {
    from: Account,
    debtor_name: String,
    debtor_iban: Iban,
    debtor_bic: Option<Bic>,
    payee_name: String,
    payee_iban: Iban,
    payee_bic: Option<Bic>,
    amount: Decimal,
    purpose: String,
    end_to_end_id: String,
    instant: Instant,
}

impl CheckedTransfer {
    pub fn from(&self) -> &Account {
        &self.from
    }
    /// The given name, else the UPD holder name.
    pub fn debtor_name(&self) -> &str {
        &self.debtor_name
    }
    pub fn debtor_iban(&self) -> &Iban {
        &self.debtor_iban
    }
    /// `None` when the account's BIC is not a BIC (sent as `NOTPROVIDED`).
    pub fn debtor_bic(&self) -> Option<&Bic> {
        self.debtor_bic.as_ref()
    }
    pub fn payee_name(&self) -> &str {
        &self.payee_name
    }
    pub fn payee_iban(&self) -> &Iban {
        &self.payee_iban
    }
    pub fn payee_bic(&self) -> Option<&Bic> {
        self.payee_bic.as_ref()
    }
    pub fn amount(&self) -> Decimal {
        self.amount
    }
    pub fn purpose(&self) -> &str {
        &self.purpose
    }
    /// The caller's, or the one generated by the check (`FC` + UTC
    /// timestamp + 8 random characters). Store it before sending: it matches
    /// a later booking to the order.
    pub fn end_to_end_id(&self) -> &str {
        &self.end_to_end_id
    }
    pub fn instant(&self) -> Instant {
        self.instant
    }

    /// The flat pain.001 input; `instant` sets `LclInstrm` `INST`.
    pub(crate) fn pain_input(&self, instant: bool) -> Pain001Input<'_> {
        Pain001Input {
            debtor_name: &self.debtor_name,
            debtor_iban: &self.debtor_iban,
            debtor_bic: self.debtor_bic.as_ref(),
            payee_name: &self.payee_name,
            payee_iban: &self.payee_iban,
            payee_bic: self.payee_bic.as_ref(),
            amount: self.amount,
            purpose: &self.purpose,
            end_to_end_id: &self.end_to_end_id,
            instant,
        }
    }
}

fn invalid(field: TransferField, reason: impl Into<InvalidReason>) -> InvalidTransfer {
    InvalidTransfer {
        field,
        reason: reason.into(),
    }
}

/// A name or text field: not blank (when `required`), at most `max`
/// characters, SEPA text set (Anlage 3 2.1).
fn check_text(value: &str, max: usize, required: bool) -> Result<(), Invalid> {
    if value.trim().is_empty() {
        return if required || !value.is_empty() {
            Err(Invalid::Empty)
        } else {
            Ok(())
        };
    }
    if value.chars().count() > max {
        return Err(Invalid::TooLong { max });
    }
    if let Some(ch) = first_invalid_char(value) {
        return Err(Invalid::Character { ch });
    }
    Ok(())
}

fn check_amount(amount: Decimal) -> Result<(), InvalidReason> {
    if amount <= Decimal::ZERO {
        return Err(InvalidReason::NotPositive);
    }
    if amount > max_amount() {
        return Err(InvalidReason::TooLarge);
    }
    if amount.normalize().scale() > 2 {
        return Err(InvalidReason::Precision);
    }
    Ok(())
}

impl Transfer {
    /// Check every field before contact (payments-design section 2) and
    /// resolve the debtor name: the given one, else `upd_holder` (the UPD
    /// account holder; [`crate::jobs::transfer`] passes it). The first
    /// broken field is the error.
    pub fn check(&self, upd_holder: Option<&str>) -> Result<CheckedTransfer, InvalidTransfer> {
        self.check_with(upd_holder, || {
            generate_reference(jiff::Timestamp::now(), &mut rand::rng())
        })
    }

    /// [`check`](Self::check) with the generator of the end-to-end ID given.
    fn check_with(
        &self,
        upd_holder: Option<&str>,
        generate_id: impl FnOnce() -> String,
    ) -> Result<CheckedTransfer, InvalidTransfer> {
        use TransferField as F;
        let iban = self.from.iban();
        if iban.is_empty() {
            return Err(invalid(F::From, InvalidReason::NoIban));
        }
        let debtor_iban = Iban::parse(iban).map_err(|r| invalid(F::From, r))?;
        let debtor_name = self
            .debtor_name
            .as_deref()
            .or(upd_holder.filter(|h| !h.trim().is_empty()))
            .ok_or(invalid(F::DebtorName, InvalidReason::NoName))?;
        check_text(debtor_name, MAX_NAME, true).map_err(|r| invalid(F::DebtorName, r))?;
        check_text(&self.payee_name, MAX_NAME, true).map_err(|r| invalid(F::PayeeName, r))?;
        let payee_iban = Iban::parse(&self.payee_iban).map_err(|r| invalid(F::PayeeIban, r))?;
        let payee_bic = match &self.payee_bic {
            Some(bic) => Some(Bic::parse(bic).map_err(|r| invalid(F::PayeeBic, r))?),
            None => None,
        };
        check_amount(self.amount).map_err(|r| invalid(F::Amount, r))?;
        check_text(&self.purpose, MAX_PURPOSE, false).map_err(|r| invalid(F::Purpose, r))?;
        if let Some(id) = &self.end_to_end_id {
            check_reference(id).map_err(|r| invalid(F::EndToEndId, r))?;
        }
        Ok(CheckedTransfer {
            from: self.from.clone(),
            debtor_name: debtor_name.to_string(),
            debtor_iban,
            // The account's BIC when it is one; else NOTPROVIDED (5.4).
            debtor_bic: Bic::parse(self.from.bic()).ok(),
            payee_name: self.payee_name.clone(),
            payee_iban,
            payee_bic,
            amount: self.amount,
            purpose: self.purpose.clone(),
            end_to_end_id: self.end_to_end_id.clone().unwrap_or_else(generate_id),
            instant: self.instant,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn base() -> Transfer {
        Transfer {
            from: Account::new("DE02120300000000202051", "BYLADEM1001").unwrap(),
            debtor_name: Some("Erika Beispiel".into()),
            payee_name: "Max Muster".into(),
            payee_iban: "DE89 3704 0044 0532 0130 00".into(),
            payee_bic: Some("COBADEFFXXX".into()),
            amount: Decimal::from_str("1.00").unwrap(),
            purpose: "Test".into(),
            end_to_end_id: None,
            instant: Instant::No,
        }
    }

    #[test]
    fn end_to_end_id_is_generated_unique_or_kept() {
        let a = base().check(None).unwrap();
        let b = base().check(None).unwrap();
        assert_ne!(a.end_to_end_id(), b.end_to_end_id());
        for id in [a.end_to_end_id(), b.end_to_end_id()] {
            assert!(id.chars().count() <= 35);
            assert!(crate::formats::sepa::is_sepa_reference(id), "{id}");
            assert_ne!(id, "NOTPROVIDED");
        }
        let mut t = base();
        t.end_to_end_id = Some("MINE-1".into());
        assert_eq!(t.check(None).unwrap().end_to_end_id(), "MINE-1");
        let fixed = base().check_with(None, || "FIXED-ID".to_string()).unwrap();
        assert_eq!(fixed.end_to_end_id(), "FIXED-ID");
    }

    #[test]
    fn valid_transfer() {
        let c = base().check(None).unwrap();
        assert_eq!(c.payee_iban().as_str(), "DE89370400440532013000");
        assert!(crate::formats::sepa::is_sepa_reference(c.end_to_end_id()));
        assert_eq!(c.debtor_bic().map(Bic::as_str), Some("BYLADEM1001"));
        let mut t = base();
        t.payee_bic = None;
        t.purpose = String::new();
        t.debtor_name = None;
        t.amount = Decimal::from_str("999999999.99").unwrap();
        t.end_to_end_id = Some("RE-2026/11".into());
        t.payee_name = "Müller & Söhne".into();
        t.payee_iban = "de89 3704 0044 0532 0130 00".into();
        let c = t.check(Some("UPD Name")).unwrap();
        assert_eq!(c.payee_iban().as_str(), "DE89370400440532013000");
        assert_eq!(c.end_to_end_id(), "RE-2026/11");
        assert_eq!(c.payee_bic(), None);
        // Trailing zeros beyond two decimals are no precision.
        t.amount = Decimal::from_str("1.500").unwrap();
        t.check(Some("UPD Name")).unwrap();
        // A debtor account whose BIC is no BIC goes IBAN-only.
        t.from = Account::new("DE02120300000000202051", "-").unwrap();
        assert_eq!(t.check(Some("UPD Name")).unwrap().debtor_bic(), None);
    }

    /// The validation table: one broken field each, nothing else.
    #[test]
    fn validation_table() {
        use InvalidReason as R;
        use TransferField as F;
        fn d(s: &str) -> Decimal {
            Decimal::from_str(s).unwrap()
        }
        let s = R::Sepa;
        type Breaker = Box<dyn Fn(&mut Transfer)>;
        let cases: Vec<(Breaker, F, R)> = vec![
            (
                Box::new(|t| {
                    t.from =
                        Account::national("1234", None, &crate::Blz::new("12030000"), "X").unwrap()
                }),
                F::From,
                R::NoIban,
            ),
            (
                Box::new(|t| t.from = Account::new("DE00120300000000202051", "B").unwrap()),
                F::From,
                s(Invalid::Checksum),
            ),
            (
                Box::new(|t| t.debtor_name = Some(" ".into())),
                F::DebtorName,
                s(Invalid::Empty),
            ),
            (
                Box::new(|t| t.payee_name = String::new()),
                F::PayeeName,
                s(Invalid::Empty),
            ),
            (
                Box::new(|t| t.payee_name = "a".repeat(71)),
                F::PayeeName,
                s(Invalid::TooLong { max: 70 }),
            ),
            (
                Box::new(|t| t.payee_name = "Café".into()),
                F::PayeeName,
                s(Invalid::Character { ch: 'é' }),
            ),
            (
                Box::new(|t| t.payee_iban = "DE88370400440532013000".into()),
                F::PayeeIban,
                s(Invalid::Checksum),
            ),
            (
                Box::new(|t| t.payee_bic = Some("COBADE".into())),
                F::PayeeBic,
                s(Invalid::Length),
            ),
            (Box::new(|t| t.amount = d("0")), F::Amount, R::NotPositive),
            (Box::new(|t| t.amount = d("-1")), F::Amount, R::NotPositive),
            (Box::new(|t| t.amount = d("0.001")), F::Amount, R::Precision),
            (
                Box::new(|t| t.amount = d("1000000000.00")),
                F::Amount,
                R::TooLarge,
            ),
            (
                Box::new(|t| t.purpose = "x".repeat(141)),
                F::Purpose,
                s(Invalid::TooLong { max: 140 }),
            ),
            (
                Box::new(|t| t.purpose = "   ".into()),
                F::Purpose,
                s(Invalid::Empty),
            ),
            (
                Box::new(|t| t.purpose = "50 €".into()),
                F::Purpose,
                s(Invalid::Character { ch: '€' }),
            ),
            (
                Box::new(|t| t.end_to_end_id = Some("A//B".into())),
                F::EndToEndId,
                s(Invalid::Slash),
            ),
            (
                Box::new(|t| t.end_to_end_id = Some("Müller".into())),
                F::EndToEndId,
                s(Invalid::Character { ch: 'ü' }),
            ),
            (
                Box::new(|t| t.end_to_end_id = Some(String::new())),
                F::EndToEndId,
                s(Invalid::Empty),
            ),
        ];
        for (i, (break_it, field, reason)) in cases.into_iter().enumerate() {
            let mut t = base();
            break_it(&mut t);
            assert_eq!(
                t.check(Some("UPD Name")).unwrap_err(),
                InvalidTransfer { field, reason },
                "case {i}"
            );
        }
    }

    #[test]
    fn debtor_name_resolution() {
        let mut t = base();
        let name = |t: &Transfer, h| t.check(h).map(|c| c.debtor_name().to_string());
        assert_eq!(name(&t, Some("UPD Name")).unwrap(), "Erika Beispiel");
        t.debtor_name = None;
        assert_eq!(name(&t, Some("UPD Name")).unwrap(), "UPD Name");
        assert_eq!(
            name(&t, None).unwrap_err(),
            InvalidTransfer {
                field: TransferField::DebtorName,
                reason: InvalidReason::NoName
            }
        );
        assert_eq!(
            name(&t, Some("  ")).unwrap_err().reason,
            InvalidReason::NoName
        );
        assert_eq!(
            name(&t, Some("Zoë")).unwrap_err().reason,
            InvalidReason::Sepa(Invalid::Character { ch: 'ë' })
        );
    }

    #[test]
    fn error_category() {
        let mut t = base();
        t.amount = Decimal::ZERO;
        let e: crate::FinTSError = t.check(None).unwrap_err().into();
        assert_eq!(e.kind(), crate::Category::InvalidInput);
        assert_eq!(
            e.to_string(),
            "invalid transfer: amount: must be greater than zero"
        );
    }
}
