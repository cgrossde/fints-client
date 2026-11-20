//! FinTS segments as the protocol uses them.
//!
//! Outgoing segments are built from the codec's declarative definitions
//! (`codec::defs`); incoming segments are read with `codec::registry`.
//! `mt535` / `mt536` parse the SWIFT payloads inside HIWPD / HIWDU, with
//! the field reading they share in `swift`.

pub mod mt535;
pub mod mt536;
pub(crate) mod read;
pub(crate) mod request;
pub mod response;
mod swift;

use crate::codec::{self, DataElement, RawSegment, Scalar};
use crate::error::{FinTSError, Result};
use crate::types::SegmentType;

/// A codec value from caller-supplied text, kept exactly as given.
///
/// The `An::new` family trims spaces (Formals B.4.1, a rule for senders);
/// values that come from the caller or the bank (PIN, user ID, touchdown
/// point) are taken verbatim. The format's character rules still apply; lengths are checked
/// when the segment is encoded.
pub(crate) fn verbatim<T: Scalar>(value: &str, what: &str) -> Result<T> {
    T::from_wire(&DataElement::Text(value.to_owned()))
        .map_err(|e| FinTSError::Serialize(format!("{what}: {e}")))
}

/// An order whose version is negotiated with the bank's BPD (Formals C.10):
/// the client sends the highest version both the BPD and
/// [`Order::implemented`] list, see
/// `BankParams::order_version`.
///
/// Administrative segments (HKIDN, HKVVB, HKSYN, HKEND) have one fixed
/// version each and are not orders; HKTAN follows the selected TAN method's
/// HITANS version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Order {
    /// HKSAL, balance.
    Balance,
    /// HKKAZ, transactions as MT940.
    Transactions,
    /// HKCAZ, transactions as camt.052.
    Camt,
    /// HKWPD, depot holdings as MT535.
    Holdings,
    /// HKWDU, depot transactions as MT536.
    DepotTransactions,
    /// HKCCS, SEPA credit transfer.
    Transfer,
    /// HKIPZ, SEPA instant credit transfer.
    InstantTransfer,
    /// HKVPP, payee check, Verification of Payee.
    VopCheck,
    /// HKVPA, release after the payee check.
    VopExecute,
    /// HKIPS, instant payment status.
    InstantStatus,
}

impl Order {
    /// Every order, in declaration order.
    #[cfg(test)]
    pub const ALL: &'static [Order] = &[
        Order::Balance,
        Order::Transactions,
        Order::Camt,
        Order::Holdings,
        Order::DepotTransactions,
        Order::Transfer,
        Order::InstantTransfer,
        Order::VopCheck,
        Order::VopExecute,
        Order::InstantStatus,
    ];

    /// The order's segment code, e.g. `HKSAL`; its BPD parameter segment is
    /// the same code with `I` and `S` (`HISALS`).
    pub fn code(self) -> &'static str {
        match self {
            Order::Balance => "HKSAL",
            Order::Transactions => "HKKAZ",
            Order::Camt => "HKCAZ",
            Order::Holdings => "HKWPD",
            Order::DepotTransactions => "HKWDU",
            Order::Transfer => "HKCCS",
            Order::InstantTransfer => "HKIPZ",
            Order::VopCheck => "HKVPP",
            Order::VopExecute => "HKVPA",
            Order::InstantStatus => "HKIPS",
        }
    }

    /// The versions this client builds, ascending: the arms of the builder
    /// in `segments::request`. A test builds every one of them.
    /// `BankParams::order_version` reports a mismatch when the bank offers
    /// none of them.
    pub fn implemented(self) -> &'static [u16] {
        match self {
            Order::Balance => &[5, 6, 7, 8],
            Order::Transactions => &[5, 6, 7],
            Order::Camt => &[1],
            Order::Holdings => &[5, 6],
            Order::DepotTransactions => &[5],
            Order::Transfer => &[1],
            Order::InstantTransfer => &[1, 2],
            Order::VopCheck => &[1],
            Order::VopExecute => &[1],
            Order::InstantStatus => &[1],
        }
    }
}

/// A builder was asked for a version [`Order::implemented`] does not list:
/// a bug, since every version comes from [`Order::implemented`].
pub(crate) fn unbuildable(code: &str, version: u16) -> FinTSError {
    FinTSError::Internal(format!("{code} v{version} has no builder"))
}

/// Version `version` of order `code` cannot address the account.
pub(crate) fn not_addressable(
    code: &str,
    version: u16,
    reason: crate::error::AddressingReason,
) -> FinTSError {
    FinTSError::AccountNotAddressable {
        code: SegmentType::new(code),
        version,
        reason,
    }
}

/// Encodes a segment definition with the given segment number.
pub(crate) fn encode<T: codec::Segment>(value: &T, number: u16) -> Result<RawSegment> {
    codec::encode_segment(value, number, None).map_err(|e| FinTSError::Serialize(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Account;
    use crate::types::Blz;
    use jiff::civil::Date;

    /// Builds a sample of `order` in `version` with its builder, as raw
    /// segment header `(code, version)`.
    fn build(order: Order, version: u16) -> Result<(String, String)> {
        let giro = Account::new("DE00120300009900012345", "BYLADEM1001")?;
        let depot = Account::national("0123456789", None, &Blz::new("12030000"), "BYLADEM1001")?;
        let day = Date::new(2026, 1, 1).unwrap();
        let degs = match order {
            Order::Balance => request::hksal(3, version, &giro, None)?.degs,
            Order::Transactions => request::hkkaz(3, version, &giro, day, day, None)?.degs,
            Order::Camt => {
                request::hkcaz(3, version, &giro, request::CAMT052_DEFAULT, day, day, None)?.degs
            }
            Order::Holdings => request::hkwpd(3, version, &depot, None, None)?.degs,
            Order::DepotTransactions => request::hkwdu(3, version, &depot, None, None, None)?.degs,
            Order::Transfer => request::hkccs(3, version, &giro, "pain", b"<x/>")?.degs,
            Order::InstantTransfer => {
                request::hkipz(3, version, &giro, "pain", b"<x/>", Some(true))?.degs
            }
            Order::VopCheck => request::hkvpp(3, version, &["pain.002".into()], None, None)?.degs,
            Order::VopExecute => request::hkvpa(3, version, b"VOPID")?.degs,
            Order::InstantStatus => {
                request::hkips(3, version, &giro, &["pain".into()], "ORDER-1")?.degs
            }
        };
        Ok((
            degs[0].get_str(0).to_string(),
            degs[0].get_str(2).to_string(),
        ))
    }

    /// Every version an order claims has a builder, and the builder refuses
    /// the next one; codec-backed versions are registered definitions.
    #[test]
    fn implemented_versions_match_the_builders() {
        for &order in Order::ALL.iter() {
            let implemented = order.implemented();
            assert!(implemented.is_sorted());
            let registered = codec::versions(order.code());
            assert!(
                registered.is_empty() || implemented.iter().all(|v| registered.contains(v)),
                "{order:?}: {implemented:?} not all in the codec registry {registered:?}"
            );
            for &v in implemented {
                let (code, version) =
                    build(order, v).unwrap_or_else(|e| panic!("{order:?} v{v}: {e}"));
                assert_eq!((code.as_str(), version), (order.code(), v.to_string()));
            }
            let next = implemented.last().unwrap() + 1;
            assert!(
                matches!(build(order, next), Err(FinTSError::Internal(_))),
                "{order:?} v{next}"
            );
        }
    }

    /// A registered definition without a builder is not claimed: HKSPA 1 is
    /// in the codec registry, no order sends it.
    #[test]
    fn registry_versions_without_builder_are_not_claimed() {
        assert_eq!(codec::versions("HKSPA"), [1]);
        assert!(Order::ALL.iter().all(|o| o.code() != "HKSPA"));
    }
}
