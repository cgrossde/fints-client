//! HKIPZ, SEPA-Instant Payment Zahlung. Messages 2022, C.10.2.9.1.

use super::common::Kti1;
use crate::codec::{An, Bin, Jn, Segment};

/// HKIPZ version 1, customer order. Messages 2022, C.10.2.9.1.1.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Kontoverbindung international DEG kti  #     M     1
///  3  SEPA Descriptor            DE   an    ..256  M     1
///  4  SEPA pain message          DE   bin    ..    M     1
/// ```
///
/// The requested execution date in the pain message must be `1999-01-01`
/// for a non-dated order (bank answer 9150 otherwise).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKIPZ", version = 1)]
pub struct HkIpz1 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 SEPA Descriptor.
    #[de(max = 256)]
    pub descriptor: An,
    /// 4 SEPA pain message.
    pub message: Bin,
}

/// HKIPZ version 2, customer order. Messages 2022, C.10.2.9.1.2. Adds the
/// permission to convert an undeliverable instant payment into a standard
/// transfer (answer code 3270).
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Kontoverbindung international DEG kti  #     M     1
///  3  SEPA Descriptor            DE   an    ..256  M     1
///  4  SEPA pain message          DE   bin    ..    M     1
///  5  Umwandlung nach SEPA-Überweisung zulässig DE jn # C  1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKIPZ", version = 2)]
pub struct HkIpz2 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 SEPA Descriptor.
    #[de(max = 256)]
    pub descriptor: An,
    /// 4 SEPA pain message.
    pub message: Bin,
    /// 5 Umwandlung nach SEPA-Überweisung zulässig. M when the BPD
    /// "Umwandlung nach SEPA-Überweisung zulässig erlaubt" is `J`, else N.
    pub convert_to_sepa_allowed: Option<Jn>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKIPZ:3:1+DE00120300009900012345:BYLADEM1001+urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09+@5@<a/> '",
    "HKIPZ:3:2+DE00120300009900012345:BYLADEM1001+urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09+@5@<a/> +J'",
    "HKIPZ:3:2+DE00120300009900012345:BYLADEM1001+urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09+@5@<a/> '",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HkIpz1 = round_trip(SAMPLES[0]);
        assert_eq!(v.message.0, b"<a/> ");
        let v: HkIpz2 = round_trip(SAMPLES[1]);
        assert_eq!(v.convert_to_sepa_allowed, Some(Jn::YES));
        let v: HkIpz2 = round_trip(SAMPLES[2]);
        assert_eq!(v.convert_to_sepa_allowed, None);
    }

    #[test]
    fn version_1_has_no_conversion_element() {
        let e = from_wire::<HkIpz1>(SAMPLES[1].replace(":2+", ":1+").as_bytes()).unwrap_err();
        assert!(matches!(e.kind, crate::codec::ErrorKind::UnexpectedData));
    }
}
