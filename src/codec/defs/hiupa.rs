//! HIUPA, Userparameter allgemein (general user parameters). Formals 2017, E.2.

use crate::codec::{An, Code, Id, Num, Segment};

/// HIUPA version 4.
///
/// ```text
/// Nr. Name                  Typ  Format Länge  Status Anzahl
///  1  Segmentkopf           DEG               M     1
///  2  Benutzerkennung       DE   id     #      M     1
///  3  UPD-Version           DE   num    ..3    M     1
///  4  UPD-Verwendung        DE   code   1      M     1    0, 1
///  5  Benutzername          DE   an     ..35   O     1
///  6  Erweiterung, allgemein DE  an     ..2048 O     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIUPA", version = 4)]
pub struct HiUpa4 {
    /// 2 Benutzerkennung: the user the UPD belong to.
    pub user_id: Id,
    /// 3 UPD-Version. A bank that answers an UPD version 0 in HKVVB with
    /// version 0 means the UPD are valid for this dialog only.
    #[de(max = 3)]
    pub upd_version: Num,
    /// 4 UPD-Verwendung: `0` orders not listed in HIUPD are blocked, `1`
    /// nothing can be said about them from the UPD.
    #[de(len = 1)]
    pub upd_usage: Code,
    /// 5 Benutzername.
    #[de(max = 35)]
    pub user_name: Option<An>,
    /// 6 Erweiterung, allgemein: bilateral, no defined structure.
    #[de(max = 2048)]
    pub extension: Option<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIUPA:6:4:4+USER0001+7+0'",
    "HIUPA:6:4:4+USER0001+7+1+Erika Beispiel+bilateral'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiUpa4 = round_trip(SAMPLES[0]);
        assert_eq!(
            (v.user_id.as_str(), v.upd_version),
            ("USER0001", Num::new(7))
        );
        let v: HiUpa4 = round_trip(SAMPLES[1]);
        assert_eq!(v.user_name.unwrap().as_str(), "Erika Beispiel");
    }

    /// The string the hand-written parser's regression test reads: user
    /// ID first, the UPD version in the second element (the old bug).
    #[test]
    fn reads_the_regression_fixture() {
        let (_, v) = from_wire::<HiUpa4>(b"HIUPA:6:4:4+test1+7+0'").unwrap();
        assert_eq!((v.user_id.as_str(), v.upd_version), ("test1", Num::new(7)));
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiUpa4>(b"HIUPA:6:4:4+USER0001+7+2x'").unwrap_err();
        assert_eq!(e.field, "upd_usage");
        let e = from_wire::<HiUpa4>(b"HIUPA:6:4:4+USER0001+1234+0'").unwrap_err();
        assert_eq!(e.field, "upd_version");
    }
}
