//! HIKAZS, Kontoumsätze/Zeitraum Parameter (statement-request parameters
//! in the BPD). HBCI 2.2, VII.2.1.1 (v5); Messages 2022, C.2.1.1.1.1 (v6, v7).

use crate::codec::{Code, DataElementGroup, Jn, Num, Segment};

/// HIKAZS version 5, bank parameter segment. HBCI 2.2, VII.2.1.1 / IV.6.
/// No Sicherheitsklasse (added in FinTS 3.0 Messages).
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Maximale Anzahl Aufträge      DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens  DE   num     1    M     1
///  4  Parameter Kontoumsätze/Zeitraum DEG           M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIKAZS", version = 5)]
pub struct HiKazs5 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Parameter Kontoumsätze/Zeitraum.
    pub parameters: TurnoverParameters2,
}

/// HIKAZS version 6, bank parameter segment.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Maximale Anzahl Aufträge      DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens  DE   num     1    M     1
///  4  Sicherheitsklasse             DE   code    1    M     1
///  5  Parameter Kontoumsätze/Zeitraum DEG v2          M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIKAZS", version = 6)]
pub struct HiKazs6 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Kontoumsätze/Zeitraum.
    pub parameters: TurnoverParameters2,
}

/// HIKAZS version 7. Same layout as version 6.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIKAZS", version = 7)]
pub struct HiKazs7 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Kontoumsätze/Zeitraum.
    pub parameters: TurnoverParameters2,
}

/// Parameter Kontoumsätze/Zeitraum, version 2. Messages 2022, data dictionary.
///
/// ```text
/// Nr. Name                            Typ  Format Länge Status Anzahl
///  1  Speicherzeitraum                DE   num    ..4   M     1   >0
///  2  Eingabe Anzahl Einträge erlaubt DE   jn      #    M     1
///  3  Alle Konten erlaubt             DE   jn      #    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TurnoverParameters2 {
    /// 1 Speicherzeitraum.
    #[de(max = 4)]
    pub storage_period: Num,
    /// 2 Eingabe Anzahl Einträge erlaubt.
    pub max_entries_allowed: Jn,
    /// 3 Alle Konten erlaubt.
    pub all_accounts_allowed: Jn,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIKAZS:10:7:4+1+1+1+365:J:N'",
    "HIKAZS:10:6:4+1+1+1+365:J:N'",
    "HIKAZS:10:5:4+1+2+60:J:J'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    #[test]
    fn wire_round_trips() {
        let v: HiKazs7 = round_trip(SAMPLES[0]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(1));
        assert_eq!(v.security_class.as_str(), "1");
        assert_eq!(v.parameters.storage_period, Num::new(365));
        assert_eq!(v.parameters.max_entries_allowed, Jn::YES);
        assert_eq!(v.parameters.all_accounts_allowed, Jn::NO);

        let v: HiKazs6 = round_trip(SAMPLES[1]);
        assert_eq!(v.parameters.storage_period, Num::new(365));
        assert_eq!(v.parameters.max_entries_allowed, Jn::YES);
        assert_eq!(v.parameters.all_accounts_allowed, Jn::NO);

        let v: HiKazs5 = round_trip(SAMPLES[2]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(2));
        assert_eq!(v.parameters.storage_period, Num::new(60));
        assert_eq!(v.parameters.max_entries_allowed, Jn::YES);
        assert_eq!(v.parameters.all_accounts_allowed, Jn::YES);
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiKazs7>(b"HIKAZS:10:7:4+1+12+1+365:J:N'").unwrap_err();
        assert_eq!(e.field, "min_signatures");
        assert_eq!(
            e.kind,
            ErrorKind::WrongLength {
                expected: 1,
                actual: 2
            }
        );
        let e = from_wire::<HiKazs5>(b"HIKAZS:10:5:4+1+12+60:J:J'").unwrap_err();
        assert_eq!(e.field, "min_signatures");
    }
}
