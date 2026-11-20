//! HIIPS, SEPA-Instant Payment Status rückmelden. Messages 2022, C.10.2.9.2.

use super::common::Kti1;
use crate::codec::{An, Bin, Code, Segment};

/// HIIPS version 1, bank answer to HKIPS 1. Messages 2022, C.10.2.9.2.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Kontoverbindung international DEG kti  #     M     1
///  3  SEPA Descriptor            DE   an    ..256  O     1
///  4  SEPA pain message          DE   bin    ..    O     1
///  5  Auftragsidentifikation     DE   an    ..99   M     1
///  6  SEPA-C-Code                DE   code    #    O     1     1, 2, 3, 4
///  7  Status SEPA-Auftrag        DE   code    #    O     1     1 to 9
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIIPS", version = 1)]
pub struct HiIps1 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 SEPA Descriptor.
    #[de(max = 256)]
    pub descriptor: Option<An>,
    /// 4 SEPA pain message.
    pub message: Option<Bin>,
    /// 5 Auftragsidentifikation.
    #[de(max = 99)]
    pub order_id: An,
    /// 6 SEPA-C-Code.
    pub sepa_c_code: Option<Code>,
    /// 7 Status SEPA-Auftrag.
    pub status: Option<Code>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIIPS:4:1:3+DE00120300009900012345:BYLADEM1001+urn?:x?:pain.002+@6@<a/>  +ORDER-0001+4+5'",
    "HIIPS:4:1:3+DE00120300009900012345:BYLADEM1001+++ORDER-0001'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiIps1 = round_trip(SAMPLES[0]);
        assert_eq!(v.message.as_ref().map(|b| b.0.len()), Some(6));
        assert_eq!(v.status.as_ref().map(Code::as_str), Some("5"));
        let v: HiIps1 = round_trip(SAMPLES[1]);
        assert!(v.descriptor.is_none() && v.message.is_none());
        assert_eq!(v.order_id.as_str(), "ORDER-0001");
    }

    #[test]
    fn the_order_id_is_mandatory() {
        let e =
            from_wire::<HiIps1>(b"HIIPS:4:1:3+DE00120300009900012345:BYLADEM1001'").unwrap_err();
        assert_eq!(e.field, "order_id");
    }
}
