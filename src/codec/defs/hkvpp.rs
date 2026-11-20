//! HKVPP, Namensabgleich Prüfauftrag (Verification of Payee, check order).
//! VoP 1.02, C.10.7.1.1.

use crate::codec::{An, Bin, DataElementGroup, Num, Segment};

/// Unterstützte Payment Status Reports, version 1. VoP 1.02, C.10.7.1.1: one
/// descriptor (pain.002 schema) per data element. The data dictionary has no
/// entry of its own; the layout follows the pain message groups of the
/// SEPA orders (descriptor `an ..256`, up to 99).
///
/// ```text
/// Nr. Name                  Typ  Format Länge Status Anzahl
///  1  Payment Status Report Descriptor DE an ..256  M    ..99
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SupportedPaymentStatusReports1 {
    /// 1 Descriptor (schema URNs, 1 to 99).
    #[de(max = 256, min_count = 1, max_count = 99)]
    pub descriptors: Vec<An>,
}

/// HKVPP version 1, customer order. VoP 1.02, C.10.7.1.1. Sent together
/// with the payment order it checks; not TAN-protected.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Unterstützte Payment Status Reports DEG      M     1
///  3  Polling-ID                 DE   bin    ..    C     1
///  4  Maximale Anzahl Einträge   DE   num    ..4   C     1
///  5  Aufsetzpunkt               DE   an    ..35   C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKVPP", version = 1)]
pub struct HkVpp1 {
    /// 2 Unterstützte Payment Status Reports.
    pub reports: SupportedPaymentStatusReports1,
    /// 3 Polling-ID. M when a touchdown point is set and the bank sent a
    /// polling ID; N otherwise.
    pub polling_id: Option<Bin>,
    /// 4 Maximale Anzahl Einträge (>0). O when the BPD "Eingabe Anzahl
    /// Einträge erlaubt" is `J`; N otherwise.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 5 Aufsetzpunkt. M when the bank returned a touchdown point.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKVPP:4:1+urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.002.001.10'",
    "HKVPP:4:1+sepade.pain.002.001.10.xsd+@4@poll+20+TD1'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HkVpp1 = round_trip(SAMPLES[0]);
        assert_eq!(v.reports.descriptors.len(), 1);
        assert!(v.polling_id.is_none() && v.touchdown.is_none());
        let v: HkVpp1 = round_trip(SAMPLES[1]);
        assert_eq!(
            v.polling_id.as_ref().map(|b| b.0.as_slice()),
            Some(&b"poll"[..])
        );
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown.as_ref().map(An::as_str), Some("TD1"));
    }

    #[test]
    fn a_descriptor_is_required() {
        assert_eq!(
            from_wire::<HkVpp1>(b"HKVPP:4:1'").unwrap_err().field,
            "reports.descriptors"
        );
    }
}
