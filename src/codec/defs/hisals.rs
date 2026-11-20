//! HISALS, Saldenabfrage Parameter (balance-request parameters in the BPD).
//! HBCI 2.2, VII.2.2 (version 5); Messages 2022, C.2.1.2.1 (version 6),
//! C.2.1.2.2 (version 7) and C.2.1.2.3 (version 8).
//!
//! Versions 5-7 have no order-specific parameters; version 8 adds
//! "Parameter Saldenabfrage" (Messages 2022, D). Version 5 is
//! HBCI 2.2 IV.6 "Geschäftsvorfall ohne Parameter" (max orders, min
//! signatures). Version 6+ adds Sicherheitsklasse (Messages 2022).

use crate::codec::{Code, DataElementGroup, Jn, Num, Segment};

/// HISALS version 5. HBCI 2.2, VII.2.2 / IV.6 (ohne Parameter).
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Maximale Anzahl Aufträge      DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens  DE   num     1    M     1   0, 1, 2, 3
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISALS", version = 5)]
pub struct HiSals5 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
}

/// HISALS version 6.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Maximale Anzahl Aufträge      DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens  DE   num     1    M     1   0, 1, 2, 3
///  4  Sicherheitsklasse             DE   code    1    M     1   0, 1, 2, 3, 4
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISALS", version = 6)]
pub struct HiSals6 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
}

/// HISALS version 7. Same layout as version 6.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISALS", version = 7)]
pub struct HiSals7 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
}

/// HISALS version 8. Messages 2022, C.2.1.2.3.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Maximale Anzahl Aufträge      DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens  DE   num     1    M     1   0, 1, 2, 3
///  4  Sicherheitsklasse             DE   code    1    M     1   0, 1, 2, 3, 4
///  5  Parameter Saldenabfrage       DEG               M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISALS", version = 8)]
pub struct HiSals8 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Saldenabfrage.
    pub parameters: BalanceParameters1,
}

/// Parameter Saldenabfrage, version 1. Messages 2022, D (Data Dictionary).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct BalanceParameters1 {
    /// 1 Eingabe Anzahl Einträge erlaubt, `jn`, M.
    pub max_entries_allowed: Jn,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HISALS:19:6:4+1+1+1'",
    "HISALS:19:7:4+1+1+0'",
    "HISALS:19:5:4+1+2'",
    "HISALS:19:8:4+1+1+1+J'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiSals6 = round_trip(SAMPLES[0]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(1));
        assert_eq!(v.security_class.as_str(), "1");
        let v: HiSals7 = round_trip(SAMPLES[1]);
        assert_eq!(v.security_class.as_str(), "0");
        let v: HiSals5 = round_trip(SAMPLES[2]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.min_signatures, Num::new(2));
        let v: HiSals8 = round_trip(SAMPLES[3]);
        assert_eq!(v.parameters.max_entries_allowed, Jn::YES);
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiSals6>(b"HISALS:19:6:4+1+12+1'").unwrap_err();
        assert_eq!(e.field, "min_signatures");
        let e = from_wire::<HiSals7>(b"HISALS:19:7:4+1234+1+1'").unwrap_err();
        assert_eq!(e.field, "max_orders");
        let e = from_wire::<HiSals5>(b"HISALS:19:5:4+1+12'").unwrap_err();
        assert_eq!(e.field, "min_signatures");
        let e = from_wire::<HiSals8>(b"HISALS:19:8:4+1+1+1'").unwrap_err();
        assert_eq!(e.field, "parameters.max_entries_allowed");
    }
}
