//! HISPAS, SEPA-Kontoverbindung anfordern, Parameter (SEPA account request
//! parameters in the BPD). Messages 2022, C.10.1.3 (v1), C.10.1.4 (v2),
//! C.10.1.5 (v3); parameter groups in the data dictionary, "PSD2 Management".

use crate::codec::{An, Code, DataElementGroup, Jn, Num, Segment};

/// HISPAS version 1, bank parameter segment. Reference segment HKVVB.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
///  5  Parameter SEPA-Kontoverbindung anfordern DEG v1  M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISPAS", version = 1)]
pub struct HiSpas1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter SEPA-Kontoverbindung anfordern.
    pub parameters: SepaAccountParameters1,
}

/// HISPAS version 2. Same header as version 1, parameter group version 2.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
///  5  Parameter SEPA-Kontoverbindung anfordern DEG v2  M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISPAS", version = 2)]
pub struct HiSpas2 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter SEPA-Kontoverbindung anfordern.
    pub parameters: SepaAccountParameters2,
}

/// HISPAS version 3. Same header as version 1, parameter group version 3.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
///  5  Parameter SEPA-Kontoverbindung anfordern DEG v3  M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISPAS", version = 3)]
pub struct HiSpas3 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter SEPA-Kontoverbindung anfordern.
    pub parameters: SepaAccountParameters3,
}

/// Parameter SEPA-Kontoverbindung anfordern, version 1. Messages 2022,
/// data dictionary.
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Einzelkontenabruf erlaubt           DE   jn      #    M     1
///  2  Nationale Kontoverbindung erlaubt   DE   jn      #    M     1
///  3  Strukturierter Verwendungszweck erlaubt DE jn     #    M     1
///  4  Unterstützte SEPA-Datenformate      DE   an    ..256  O     ..99
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SepaAccountParameters1 {
    /// 1 Einzelkontenabruf erlaubt.
    pub single_account_allowed: Jn,
    /// 2 Nationale Kontoverbindung erlaubt.
    pub national_account_allowed: Jn,
    /// 3 Strukturierter Verwendungszweck erlaubt.
    pub structured_purpose_allowed: Jn,
    /// 4 Unterstützte SEPA-Datenformate (schema URNs).
    #[de(max = 256, max_count = 99)]
    pub formats: Vec<An>,
}

/// Parameter SEPA-Kontoverbindung anfordern, version 2. Adds "Eingabe Anzahl
/// Einträge erlaubt".
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Einzelkontenabruf erlaubt           DE   jn      #    M     1
///  2  Nationale Kontoverbindung erlaubt   DE   jn      #    M     1
///  3  Strukturierter Verwendungszweck erlaubt DE jn     #    M     1
///  4  Eingabe Anzahl Einträge erlaubt     DE   jn      #    M     1
///  5  Unterstützte SEPA-Datenformate      DE   an    ..256  O     ..99
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SepaAccountParameters2 {
    /// 1 Einzelkontenabruf erlaubt.
    pub single_account_allowed: Jn,
    /// 2 Nationale Kontoverbindung erlaubt.
    pub national_account_allowed: Jn,
    /// 3 Strukturierter Verwendungszweck erlaubt.
    pub structured_purpose_allowed: Jn,
    /// 4 Eingabe Anzahl Einträge erlaubt.
    pub max_entries_allowed: Jn,
    /// 5 Unterstützte SEPA-Datenformate (schema URNs).
    #[de(max = 256, max_count = 99)]
    pub formats: Vec<An>,
}

/// Parameter SEPA-Kontoverbindung anfordern, version 3. Adds "Anzahl
/// reservierter Verwendungszweckstellen".
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Einzelkontenabruf erlaubt           DE   jn      #    M     1
///  2  Nationale Kontoverbindung erlaubt   DE   jn      #    M     1
///  3  Strukturierter Verwendungszweck erlaubt DE jn     #    M     1
///  4  Eingabe Anzahl Einträge erlaubt     DE   jn      #    M     1
///  5  Anzahl reservierter Verwendungszweckstellen DE num ..2 M   1
///  6  Unterstützte SEPA-Datenformate      DE   an    ..256  O     ..99
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SepaAccountParameters3 {
    /// 1 Einzelkontenabruf erlaubt.
    pub single_account_allowed: Jn,
    /// 2 Nationale Kontoverbindung erlaubt.
    pub national_account_allowed: Jn,
    /// 3 Strukturierter Verwendungszweck erlaubt.
    pub structured_purpose_allowed: Jn,
    /// 4 Eingabe Anzahl Einträge erlaubt.
    pub max_entries_allowed: Jn,
    /// 5 Anzahl reservierter Verwendungszweckstellen.
    #[de(max = 2)]
    pub reserved_purpose_places: Num,
    /// 6 Unterstützte SEPA-Datenformate (schema URNs).
    #[de(max = 256, max_count = 99)]
    pub formats: Vec<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HISPAS:8:1:4+1+1+0+J:J:J:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.03:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.008.001.02'",
    "HISPAS:8:2:4+1+1+0+N:J:N:J:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.03'",
    "HISPAS:8:3:4+1+2+1+J:N:J:N:27:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    const URN_001_03: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.001.03";

    #[test]
    fn wire_round_trips() {
        let v: HiSpas1 = round_trip(SAMPLES[0]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(1));
        assert_eq!(v.security_class.as_str(), "0");
        assert_eq!(v.parameters.single_account_allowed, Jn::YES);
        assert_eq!(v.parameters.national_account_allowed, Jn::YES);
        assert_eq!(v.parameters.structured_purpose_allowed, Jn::YES);
        assert_eq!(v.parameters.formats.len(), 2);
        assert_eq!(v.parameters.formats[0].as_str(), URN_001_03);

        let v: HiSpas2 = round_trip(SAMPLES[1]);
        assert_eq!(v.parameters.single_account_allowed, Jn::NO);
        assert_eq!(v.parameters.national_account_allowed, Jn::YES);
        assert_eq!(v.parameters.structured_purpose_allowed, Jn::NO);
        assert_eq!(v.parameters.max_entries_allowed, Jn::YES);
        assert_eq!(v.parameters.formats.len(), 1);

        let v: HiSpas3 = round_trip(SAMPLES[2]);
        assert_eq!(v.min_signatures, Num::new(2));
        assert_eq!(v.security_class.as_str(), "1");
        assert_eq!(v.parameters.national_account_allowed, Jn::NO);
        assert_eq!(v.parameters.max_entries_allowed, Jn::NO);
        assert_eq!(v.parameters.reserved_purpose_places, Num::new(27));
        assert_eq!(v.parameters.formats.len(), 1);
    }

    #[test]
    fn formats_are_optional() {
        let v: HiSpas1 = round_trip("HISPAS:8:1:4+1+1+0+J:N:J'");
        assert!(v.parameters.formats.is_empty());
        let v: HiSpas3 = round_trip("HISPAS:8:3:4+1+1+0+J:N:J:J:0'");
        assert!(v.parameters.formats.is_empty());
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiSpas1>(b"HISPAS:8:1:4+1+12+0+J:J:J'").unwrap_err();
        assert_eq!(e.field, "min_signatures");
        assert_eq!(
            e.kind,
            ErrorKind::WrongLength {
                expected: 1,
                actual: 2
            }
        );
        // Version 3 needs the reserved-places element; a version 1 body fails.
        assert!(from_wire::<HiSpas3>(b"HISPAS:8:3:4+1+1+0+J:J:J'").is_err());
    }

    /// The Consorsbank BPD (committed trace): HISPAS 1 with five formats,
    /// the last one with a literal `$` that the bank sent unescaped.
    #[test]
    fn consorsbank_hispas_reads() {
        let wire = "HISPAS:8:1:4+1+1+0+J:J:J:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.03:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.003.03:urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.002.03:urn?:swift?:xsd?:$pain.001.002.02:sepade.pain.001.001.02.xsd'";
        let v: HiSpas1 = round_trip(wire);
        assert_eq!(v.parameters.national_account_allowed, Jn::YES);
        assert_eq!(v.parameters.formats.len(), 6);
        assert_eq!(
            v.parameters.formats[4].as_str(),
            "urn:swift:xsd:$pain.001.002.02"
        );
    }
}
