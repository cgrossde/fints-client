//! HIIPZS, SEPA-Instant Payment Zahlung Parameter. Messages 2022,
//! C.10.2.9.1 and the data dictionary, "PSD2 Management".

use crate::codec::{An, Code, DataElementGroup, Jn, Num, Segment};

/// HIIPZS version 1, bank parameter segment. Reference segment HKVVB.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
///  5  Parameter SEPA-Instant Payment Zahlung DEG v1  M   1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIIPZS", version = 1)]
pub struct HiIpzs1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter SEPA-Instant Payment Zahlung.
    pub parameters: InstantPaymentParameters1,
}

/// HIIPZS version 2. Same header as version 1, parameter group version 2.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
///  5  Parameter SEPA-Instant Payment Zahlung DEG v2  M   1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIIPZS", version = 2)]
pub struct HiIpzs2 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter SEPA-Instant Payment Zahlung.
    pub parameters: InstantPaymentParameters2,
}

/// Parameter SEPA-Instant Payment Zahlung, version 1. Messages 2022, data
/// dictionary.
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Zulässige Purpose codes             DE   an    ..4096  O     1
///  2  Unterstützte SEPA-Datenformate      DE   an    ..256   O     ..9
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct InstantPaymentParameters1 {
    /// 1 Zulässige Purpose codes (comma-separated ISO external purpose
    /// codes; absent or empty: every code).
    #[de(max = 4096)]
    pub purpose_codes: Option<An>,
    /// 2 Unterstützte SEPA-Datenformate (schema URNs).
    #[de(max = 256, max_count = 9)]
    pub formats: Vec<An>,
}

/// Parameter SEPA-Instant Payment Zahlung, version 2. Adds "Umwandlung nach
/// SEPA-Überweisung zulässig erlaubt" in front.
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Umwandlung nach SEPA-Überweisung zulässig erlaubt DE jn # M  1
///  2  Zulässige Purpose codes             DE   an    ..4096  O     1
///  3  Unterstützte SEPA-Datenformate      DE   an    ..256   O     ..9
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct InstantPaymentParameters2 {
    /// 1 Umwandlung nach SEPA-Überweisung zulässig erlaubt.
    pub conversion_allowed: Jn,
    /// 2 Zulässige Purpose codes.
    #[de(max = 4096)]
    pub purpose_codes: Option<An>,
    /// 3 Unterstützte SEPA-Datenformate (schema URNs).
    #[de(max = 256, max_count = 9)]
    pub formats: Vec<An>,
}

// Synthetic: the shapes banks send (empty purpose list with one format;
// a restricted list with several; version 2 with the conversion flag).
#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIIPZS:45:1:4+1+1+0+:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09'",
    "HIIPZS:22:1:4+999+1+0+BENE,BONU,SALA:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09.xsd:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.003.01'",
    "HIIPZS:46:2:4+1+1+0+N::urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09'",
    "HIIPZS:46:2:4+1+1+0+J:BENE:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09:sepade.pain.001.001.09.xsd'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiIpzs1 = round_trip(SAMPLES[0]);
        assert_eq!(v.max_orders, Num::new(1));
        assert!(
            v.parameters
                .purpose_codes
                .as_ref()
                .is_none_or(|p| p.as_str().is_empty())
        );
        assert_eq!(v.parameters.formats.len(), 1);

        let v: HiIpzs1 = round_trip(SAMPLES[1]);
        assert_eq!(v.max_orders, Num::new(999));
        assert_eq!(
            v.parameters.purpose_codes.as_ref().map(An::as_str),
            Some("BENE,BONU,SALA")
        );
        assert_eq!(v.parameters.formats.len(), 3);

        let v: HiIpzs2 = round_trip(SAMPLES[2]);
        assert_eq!(v.parameters.conversion_allowed, Jn::NO);
        assert_eq!(v.parameters.formats.len(), 1);

        let v: HiIpzs2 = round_trip(SAMPLES[3]);
        assert_eq!(v.parameters.conversion_allowed, Jn::YES);
        assert_eq!(
            v.parameters.purpose_codes.as_ref().map(An::as_str),
            Some("BENE")
        );
        assert_eq!(v.parameters.formats.len(), 2);
    }

    #[test]
    fn formats_are_optional() {
        let v: HiIpzs1 = round_trip("HIIPZS:45:1:4+1+1+0+BENE'");
        assert!(v.parameters.formats.is_empty());
    }

    #[test]
    fn version_2_needs_the_conversion_flag() {
        let e = from_wire::<HiIpzs2>(b"HIIPZS:46:2:4+1+1+0+BENE:urn'").unwrap_err();
        assert_eq!(e.field, "parameters.conversion_allowed");
    }
}
