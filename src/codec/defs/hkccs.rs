//! HKCCS, SEPA-Einzelüberweisung (SEPA credit transfer). Messages 2022,
//! C.10.2.1.

use super::common::Kti1;
use crate::codec::{An, Bin, Segment};

/// HKCCS version 1, customer order. Messages 2022, C.10.2.1.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Kontoverbindung international DEG kti  #     M     1
///  3  SEPA Descriptor            DE   an    ..256  M     1
///  4  SEPA pain message          DE   bin    ..    M     1
/// ```
///
/// The account is the debtor account; IBAN and BIC must match the pain
/// message. The descriptor names the pain.001 schema (a URN allowed by
/// HISPAS or the BPD of the order).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKCCS", version = 1)]
pub struct HkCcs1 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 SEPA Descriptor.
    #[de(max = 256)]
    pub descriptor: An,
    /// 4 SEPA pain message.
    pub message: Bin,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKCCS:3:1+DE00120300009900012345:BYLADEM1001+urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09+@11@<Document/>'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HkCcs1 = round_trip(SAMPLES[0]);
        assert_eq!(
            v.account.iban.as_ref().map(An::as_str),
            Some("DE00120300009900012345")
        );
        assert_eq!(
            v.descriptor.as_str(),
            "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09"
        );
        assert_eq!(v.message.0, b"<Document/>");
    }

    #[test]
    fn a_missing_message_is_an_error() {
        let e =
            from_wire::<HkCcs1>(b"HKCCS:3:1+DE00120300009900012345:BYLADEM1001+pain'").unwrap_err();
        assert_eq!(e.field, "message");
    }
}
