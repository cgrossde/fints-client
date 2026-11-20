//! HIIPSS, SEPA-Instant Payment Status Parameter. Messages 2022, C.10.2.9.2
//! and the data dictionary, "PSD2 Management".

use crate::codec::{An, Code, DataElementGroup, Num, Segment};

/// HIIPSS version 1, bank parameter segment. Reference segment HKVVB.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
///  5  Parameter SEPA-Instant Payment Status DEG v1    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIIPSS", version = 1)]
pub struct HiIpss1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter SEPA-Instant Payment Status.
    pub parameters: InstantStatusParameters1,
}

/// Parameter SEPA-Instant Payment Status, version 1. Messages 2022, data
/// dictionary.
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Mindestwartezeit                    DE   num     ..2   M     1
///  2  Unterstützte SEPA-Datenformate      DE   an    ..256   O     ..9
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct InstantStatusParameters1 {
    /// 1 Mindestwartezeit (seconds between the order and the first status
    /// request).
    #[de(max = 2)]
    pub min_wait: Num,
    /// 2 Unterstützte SEPA-Datenformate (schema URNs).
    #[de(max = 256, max_count = 9)]
    pub formats: Vec<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIIPSS:43:1:4+1+1+0+5'",
    "HIIPSS:23:1:4+999+1+0+99:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09.xsd'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiIpss1 = round_trip(SAMPLES[0]);
        assert_eq!(v.parameters.min_wait, Num::new(5));
        assert!(v.parameters.formats.is_empty());
        let v: HiIpss1 = round_trip(SAMPLES[1]);
        assert_eq!(v.parameters.min_wait, Num::new(99));
        assert_eq!(v.parameters.formats.len(), 2);
    }

    #[test]
    fn the_minimum_wait_is_mandatory() {
        let e = from_wire::<HiIpss1>(b"HIIPSS:43:1:4+1+1+0'").unwrap_err();
        assert_eq!(e.field, "parameters.min_wait");
    }
}
