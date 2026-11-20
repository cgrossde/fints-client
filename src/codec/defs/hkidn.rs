//! HKIDN, Identifikation (identification). Formals 2017, C.3.1.2.

use super::common::Kik1;
use crate::codec::{Code, Id, Segment};

/// HKIDN version 2, customer segment that identifies the user in the dialog
/// initialisation.
///
/// ```text
/// Nr. Name                   Typ  Format Länge Status Anzahl
///  1  Segmentkopf            DEG              M     1
///  2  Kreditinstitutskennung DEG  kik    #     M     1
///  3  Kunden-ID              DE   id     #     M     1
///  4  Kundensystem-ID        DE   id     #     M     1
///  5  Kundensystem-Status    DE   code   1     M     1    0, 1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKIDN", version = 2)]
pub struct HkIdn2 {
    /// 2 Kreditinstitutskennung: the bank the customer wants to reach.
    pub bank: Kik1,
    /// 3 Kunden-ID.
    pub customer_id: Id,
    /// 4 Kundensystem-ID: required with RAH/RDH and PIN/TAN (`0` until the
    /// bank assigned one by synchronisation); `0` with DDV.
    pub system_id: Id,
    /// 5 Kundensystem-Status: `0` system ID not needed, `1` needed (PIN/TAN).
    #[de(len = 1)]
    pub system_status: Code,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKIDN:3:2+280:12345678+CUST0001+SYS12345+1'",
    "HKIDN:3:2+280:12345678+CUST0001+0+1'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{Dig, from_wire, to_wire};

    #[test]
    fn wire_round_trips() {
        let v: HkIdn2 = round_trip(SAMPLES[0]);
        assert_eq!(v.bank.country, Dig::new("280").unwrap());
        assert_eq!(v.bank.bank_code.as_ref().unwrap().as_str(), "12345678");
        assert_eq!(v.customer_id.as_str(), "CUST0001");
        assert_eq!(v.system_status.as_str(), "1");
        round_trip::<HkIdn2>(SAMPLES[1]);
    }

    #[test]
    fn builds_the_expected_wire() {
        let built = to_wire(
            &crate::segments::request::hkidn("12345678", "CUST0001", "SYS12345").unwrap(),
            3,
            None,
        )
        .unwrap();
        // The expected wire string for these values.
        assert_eq!(built, b"HKIDN:3:2+280:12345678+CUST0001+SYS12345+1'");
        let (_, derived) = from_wire::<HkIdn2>(SAMPLES[0].as_bytes()).unwrap();
        assert_eq!(to_wire(&derived, 3, None).unwrap(), built);
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HkIdn2>(b"HKIDN:3:2+280:12345678+CUST0001+SYS12345+10'").unwrap_err();
        assert_eq!((e.de, e.field.as_str()), (Some(5), "system_status"));
        let e = from_wire::<HkIdn2>(b"HKIDN:3:2+280:12345678+CUST0001'").unwrap_err();
        assert_eq!(e.field, "system_id");
    }
}
