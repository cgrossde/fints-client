//! HKIPS, SEPA-Instant Payment Status anfordern. Messages 2022, C.10.2.9.2.

use super::common::Kti1;
use crate::codec::{An, DataElementGroup, Segment};

/// Unterstützte SEPA pain messages, version 1. Messages 2022, data
/// dictionary: one URN per data element.
///
/// ```text
/// Nr. Name                  Typ  Format Länge Status Anzahl
///  1  SEPA Descriptor       DE   an    ..256   M     ..99
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SupportedPainMessages1 {
    /// 1 SEPA Descriptor (schema URNs, 1 to 99).
    #[de(max = 256, min_count = 1, max_count = 99)]
    pub descriptors: Vec<An>,
}

/// HKIPS version 1, customer order. Messages 2022, C.10.2.9.2.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Kontoverbindung international DEG kti  #     M     1
///  3  Unterstützte SEPA pain messages DEG          M     1
///  4  Auftragsidentifikation     DE   an    ..99   M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKIPS", version = 1)]
pub struct HkIps1 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 Unterstützte SEPA pain messages (formats the client understands in
    /// the answer).
    pub formats: SupportedPainMessages1,
    /// 4 Auftragsidentifikation (from HIIPZ).
    #[de(max = 99)]
    pub order_id: An,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKIPS:3:1+DE00120300009900012345:BYLADEM1001+urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.003.01+ORDER-0001'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HkIps1 = round_trip(SAMPLES[0]);
        assert_eq!(v.formats.descriptors.len(), 2);
        assert_eq!(v.order_id.as_str(), "ORDER-0001");
        let v: HkIps1 = round_trip(
            "HKIPS:3:1+DE00120300009900012345:BYLADEM1001+urn?:x?:pain.001.001.09+ORDER-0001'",
        );
        assert_eq!(v.formats.descriptors.len(), 1);
    }

    #[test]
    fn at_least_one_format_is_required() {
        let e = from_wire::<HkIps1>(b"HKIPS:3:1+DE00120300009900012345:BYLADEM1001++ORDER-0001'")
            .unwrap_err();
        assert_eq!(e.field, "formats.descriptors");
    }
}
