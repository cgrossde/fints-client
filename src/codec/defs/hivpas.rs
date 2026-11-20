//! HIVPAS, Namensabgleich Ausführungsauftrag Parameter. VoP 1.02,
//! C.10.7.1.2.

use crate::codec::{Code, Num, Segment};

/// HIVPAS version 1, bank parameter segment. Reference segment HKVVB. The
/// order has no parameters of its own.
///
/// ```text
/// Nr. Name                           Typ  Format Länge Status Anzahl
///  1  Segmentkopf                    DEG               M     1
///  2  Maximale Anzahl Aufträge       DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens   DE   num     1    M     1
///  4  Sicherheitsklasse              DE   code    1    M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIVPAS", version = 1)]
pub struct HiVpas1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens (0, 1, 2, 3).
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse (0 to 4).
    #[de(len = 1)]
    pub security_class: Code,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HIVPAS:9:1:4+1+1+0'", "HIVPAS:10:1:4+1+1+0'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiVpas1 = round_trip(SAMPLES[0]);
        assert_eq!(v.max_orders, Num::new(1));
        assert_eq!(v.security_class.as_str(), "0");
        let _: HiVpas1 = round_trip(SAMPLES[1]);
    }

    #[test]
    fn the_security_class_is_mandatory() {
        let e = from_wire::<HiVpas1>(b"HIVPAS:9:1:4+1+1'").unwrap_err();
        assert_eq!(e.field, "security_class");
    }
}
