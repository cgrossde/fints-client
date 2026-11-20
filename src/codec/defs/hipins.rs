//! HIPINS, PIN/TAN-spezifische Informationen (PIN/TAN-specific bank
//! parameters). PIN/TAN 2020, B.8.1, in the generic layout of a parameter
//! segment (Formals 2017, D.6).

use crate::codec::{An, Code, DataElementGroup, Jn, Num, Segment};

/// HIPINS version 1.
///
/// ```text
/// Nr. Name                            Typ  Format Länge Status Anzahl
///  1  Segmentkopf                     DEG              M     1
///  2  Maximale Anzahl Aufträge        DE   num    ..3   M     1
///  3  Anzahl Signaturen mindestens    DE   num    1     M     1   0, 1, 2, 3
///  4  Sicherheitsklasse               DE   code   1     M     1   0 for PIN/TAN
///  5  Parameter PIN/TAN-spezifische Informationen DEG   M     1
/// ```
///
/// A segment with a HIPINS in the BPD means the bank offers PIN/TAN.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIPINS", version = 1)]
pub struct HiPins1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse: a filler `0` for PIN/TAN.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter PIN/TAN-spezifische Informationen.
    pub parameters: PinTanParameters1,
}

/// Parameter PIN/TAN-spezifische Informationen. PIN/TAN 2020, B.8.1 (the
/// table under "Erläuterungen"; the data dictionary has no entry for it).
/// On the wire it is one group: the five elements, then the order entries,
/// colon-separated (`5:6:6:Text:::HKCCS:J:HKKAN:N`).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct PinTanParameters1 {
    /// 1 Minimale PIN-Länge, `num ..2`, O.
    #[de(max = 2)]
    pub min_pin_length: Option<Num>,
    /// 2 Maximale PIN-Länge, `num ..2`, O.
    #[de(max = 2)]
    pub max_pin_length: Option<Num>,
    /// 3 Maximale TAN-Länge, `num ..2`, O.
    #[de(max = 2)]
    pub max_tan_length: Option<Num>,
    /// 4 Text zur Belegung der Benutzerkennung, `an ..30`, O.
    #[de(max = 30)]
    pub user_id_text: Option<An>,
    /// 5 Text zur Belegung der Kunden-ID, `an ..30`, O.
    #[de(max = 30)]
    pub customer_id_text: Option<An>,
    /// 6 Geschäftsvorfallspezifische PIN/TAN-Informationen, up to 999: one
    /// entry per order that may be secured with PIN/TAN.
    #[de(max_count = 999)]
    pub orders: Vec<PinTanOrder1>,
}

/// Geschäftsvorfallspezifische PIN/TAN-Informationen, version 1. PIN/TAN
/// 2020, D: whether one order needs a TAN. An order without an entry may
/// not be secured with PIN/TAN.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct PinTanOrder1 {
    /// 1 Segmentkennung, `an ..6`, M: the code of the order segment.
    #[de(max = 6)]
    pub segment_code: An,
    /// 2 TAN erforderlich, `jn`, M.
    pub tan_required: Jn,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIPINS:4:1:5+1+1+0+5:6:6:Kunden-Nr aus dem TAN-Brief:x:HKCCS:J:HKKAN:N:HKSAL:J:HKPAE:J'",
    "HIPINS:4:1:5+1+1+0+:::::HKSAL:N'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiPins1 = round_trip(SAMPLES[0]);
        let p = &v.parameters;
        assert_eq!(p.min_pin_length, Some(Num::new(5)));
        assert_eq!(p.max_pin_length, Some(Num::new(6)));
        assert_eq!(
            p.user_id_text.as_ref().unwrap().as_str(),
            "Kunden-Nr aus dem TAN-Brief"
        );
        assert_eq!(p.customer_id_text.as_ref().unwrap().as_str(), "x");
        let orders: Vec<_> = p
            .orders
            .iter()
            .map(|o| (o.segment_code.as_str(), o.tan_required.value()))
            .collect();
        assert_eq!(
            orders,
            [
                ("HKCCS", true),
                ("HKKAN", false),
                ("HKSAL", true),
                ("HKPAE", true)
            ]
        );
        let v: HiPins1 = round_trip(SAMPLES[1]);
        assert_eq!(v.parameters.orders.len(), 1);
    }

    /// The hand-written parser's fixture is labelled "simplified": it puts
    /// each order entry in its own data element group (`+HKSAL:N+HKKAZ:N`),
    /// not in the one parameter group the spec shows (B.8.1, example). The
    /// definition therefore keeps it raw, naming the element.
    #[test]
    fn simplified_fixture_of_the_hand_written_parser_stays_raw() {
        let e = from_wire::<HiPins1>(b"HIPINS:6:1:4+1+1+0+HKSAL:N+HKKAZ:N+HKCCS:J+HKTAN:N'")
            .unwrap_err();
        assert_eq!(e.field, "parameters.min_pin_length");
    }

    #[test]
    fn at_most_999_orders_and_two_elements_each() {
        let e = from_wire::<HiPins1>(b"HIPINS:4:1:5+1+1+0+:::::HKSAL:X'").unwrap_err();
        assert_eq!(e.field, "parameters.orders[0].tan_required");
    }
}
