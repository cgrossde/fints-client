//! HICAZS, Parameter Kontoumsätze/Zeitraum camt. Messages 2022, C.2.3.1.x.

use crate::codec::{An, Code, DataElementGroup, Jn, Num, Segment};

/// HICAZS version 1, bank parameter segment.
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Segmentkopf                         DEG               M     1
///  2  Maximale Anzahl Aufträge            DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens        DE   num     1    M     1
///  4  Sicherheitsklasse                   DE   code    1    M     1
///  5  Parameter Kontoumsätze/Zeitraum camt DEG               M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HICAZS", version = 1)]
pub struct HiCazs1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Kontoumsätze/Zeitraum camt.
    pub parameters: CamtTurnoverParameters1,
}

/// Parameter Kontoumsätze/Zeitraum camt, version 1.
///
/// ```text
/// Nr. Name                              Typ  Format Länge Status Anzahl
///  1  Speicherzeitraum                  DE   num    ..4   M     1
///  2  Eingabe Anzahl Einträge erlaubt   DE   jn      #    M     1
///  3  Alle Konten erlaubt               DE   jn      #    M     1
///  4  Unterstützte camt-Datenformate    DE   an     ..256 M     1..99
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct CamtTurnoverParameters1 {
    /// 1 Speicherzeitraum.
    #[de(max = 4)]
    pub storage_period: Num,
    /// 2 Eingabe Anzahl Einträge erlaubt.
    pub entry_count_allowed: Jn,
    /// 3 Alle Konten erlaubt.
    pub all_accounts_allowed: Jn,
    /// 4 Unterstützte camt-Datenformate.
    #[de(max = 256, min_count = 1, max_count = 99)]
    pub formats: Vec<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HICAZS:20:1:4+1+1+1+450:N:N:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02'",
    "HICAZS:20:1:4+1+1+1+450:N:N:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.08'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    const CAMT: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02";
    const CAMT08: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.08";

    #[test]
    fn wire_round_trips() {
        let v: HiCazs1 = round_trip(SAMPLES[0]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(1));
        assert_eq!(v.security_class.as_str(), "1");
        assert_eq!(v.parameters.storage_period, Num::new(450));
        assert_eq!(v.parameters.entry_count_allowed, Jn::NO);
        assert_eq!(v.parameters.all_accounts_allowed, Jn::NO);
        assert_eq!(v.parameters.formats, vec![An::new(CAMT).unwrap()]);

        let v: HiCazs1 = round_trip(SAMPLES[1]);
        assert_eq!(
            v.parameters.formats,
            vec![An::new(CAMT).unwrap(), An::new(CAMT08).unwrap()]
        );
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiCazs1>(b"HICAZS:20:1:4+1+12+1+450:N:N:x'").unwrap_err();
        assert_eq!(
            e.kind,
            ErrorKind::WrongLength {
                expected: 1,
                actual: 2
            }
        );
        assert_eq!(e.field, "min_signatures");
    }
}
