//! HIWPDS, Depotaufstellung Parameter. HBCI 2.2, VII.4.3.1 (v5);
//! Messages 2022, C.4.3.1 (v6).

use crate::codec::{Code, DataElementGroup, Jn, Num, Segment};

/// HIWPDS version 5. HBCI 2.2, VII.4.3.1 / IV.6. No Sicherheitsklasse.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Maximale Anzahl Aufträge      DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens  DE   num     1    M     1
///  4  Parameter Depotaufstellung    DEG         #    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIWPDS", version = 5)]
pub struct HiWpds5 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Parameter Depotaufstellung.
    pub parameters: DepotListParameters2,
}

/// HIWPDS version 6.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Maximale Anzahl Aufträge      DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens  DE   num     1    M     1
///  4  Sicherheitsklasse             DE   code    1    M     1
///  5  Parameter Depotaufstellung    DEG  v2      #    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIWPDS", version = 6)]
pub struct HiWpds6 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Depotaufstellung.
    pub parameters: DepotListParameters2,
}

/// Parameter Depotaufstellung, version 2.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct DepotListParameters2 {
    /// 1 Eingabe Anzahl Einträge erlaubt.
    pub max_entries_allowed: Jn,
    /// 2 Währung der Depotaufstellung wählbar.
    pub currency_selectable: Jn,
    /// 3 Kursqualität wählbar.
    pub quality_selectable: Jn,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIWPDS:11:6:4+1+1+1+J:J:N'",
    "HIWPDS:11:6:4+1+0+1+N:N:J'",
    "HIWPDS:11:5:4+1+2+J:N:J'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiWpds6 = round_trip(SAMPLES[0]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(1));
        assert_eq!(v.security_class.as_str(), "1");
        assert_eq!(v.parameters.max_entries_allowed, Jn::YES);
        assert_eq!(v.parameters.currency_selectable, Jn::YES);
        assert_eq!(v.parameters.quality_selectable, Jn::NO);
        let v: HiWpds6 = round_trip(SAMPLES[1]);
        assert_eq!(v.min_signatures, Num::new(0));
        assert_eq!(v.parameters.max_entries_allowed, Jn::NO);
        assert_eq!(v.parameters.currency_selectable, Jn::NO);
        assert_eq!(v.parameters.quality_selectable, Jn::YES);
        let v: HiWpds5 = round_trip(SAMPLES[2]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(2));
        assert_eq!(v.parameters.max_entries_allowed, Jn::YES);
        assert_eq!(v.parameters.currency_selectable, Jn::NO);
        assert_eq!(v.parameters.quality_selectable, Jn::YES);
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiWpds6>(b"HIWPDS:11:6:4+1+1+1+J:J:X'").unwrap_err();
        assert_eq!(e.field, "parameters.quality_selectable");
        assert_eq!(e.kind, crate::codec::ErrorKind::InvalidYesNo);
        let e = from_wire::<HiWpds5>(b"HIWPDS:11:5:4+1+1+J:J:X'").unwrap_err();
        assert_eq!(e.field, "parameters.quality_selectable");
        assert_eq!(e.kind, crate::codec::ErrorKind::InvalidYesNo);
    }
}
