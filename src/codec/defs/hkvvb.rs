//! HKVVB, Verarbeitungsvorbereitung (processing preparation). Formals 2017,
//! C.3.1.3.

use crate::codec::{An, Code, Num, Segment};

/// HKVVB version 3: tells the bank which BPD/UPD the client holds and which
/// product it is.
///
/// ```text
/// Nr. Name               Typ  Format Länge Status Anzahl
///  1  Segmentkopf        DEG              M     1
///  2  BPD-Version        DE   num    ..3   M     1
///  3  UPD-Version        DE   num    ..3   M     1
///  4  Dialogsprache      DE   code   ..3   M     1    0, 1, 2, 3
///  5  Produktbezeichnung DE   an     ..25  M     1
///  6  Produktversion     DE   an     ..5   M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKVVB", version = 3)]
pub struct HkVvb3 {
    /// 2 BPD-Version: the version the client holds, `0` if none.
    #[de(max = 3)]
    pub bpd_version: Num,
    /// 3 UPD-Version: the version the client holds, `0` if none.
    #[de(max = 3)]
    pub upd_version: Num,
    /// 4 Dialogsprache: 0 default, 1 German, 2 English, 3 French; only a
    /// language the bank announced in the BPD.
    #[de(max = 3)]
    pub language: Code,
    /// 5 Produktbezeichnung: the product's registration number or a
    /// meaningful name of the customer product, not of a FinTS library.
    #[de(max = 25)]
    pub product_name: An,
    /// 6 Produktversion: version of the customer product.
    #[de(max = 5)]
    pub product_version: An,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKVVB:4:3+0+0+1+Testprodukt+1.0'",
    "HKVVB:4:3+42+7+0+Testprodukt+1.0'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{from_wire, to_wire};

    #[test]
    fn wire_round_trips() {
        let v: HkVvb3 = round_trip(SAMPLES[0]);
        assert_eq!(v.language.as_str(), "1");
        assert_eq!(v.product_name.as_str(), "Testprodukt");
        let v: HkVvb3 = round_trip(SAMPLES[1]);
        assert_eq!((v.bpd_version, v.upd_version), (Num::new(42), Num::new(7)));
    }

    #[test]
    fn builds_the_expected_wire() {
        let built = to_wire(
            &crate::segments::request::hkvvb(0, 0, "Testprodukt").unwrap(),
            4,
            None,
        )
        .unwrap();
        // The expected wire string for these values.
        assert_eq!(built, b"HKVVB:4:3+0+0+1+Testprodukt+1.0'");
        let (_, derived) = from_wire::<HkVvb3>(SAMPLES[0].as_bytes()).unwrap();
        assert_eq!(to_wire(&derived, 4, None).unwrap(), built);
    }

    #[test]
    fn lengths_are_enforced() {
        let e =
            from_wire::<HkVvb3>(b"HKVVB:4:3+0+0+1+Ein Produktname mit 26 Zeich+1.0'").unwrap_err();
        assert_eq!((e.de, e.field.as_str()), (Some(5), "product_name"));
        let e = from_wire::<HkVvb3>(b"HKVVB:4:3+0+0+1+Testprodukt+1.0.0.1'").unwrap_err();
        assert_eq!(e.field, "product_version");
        let e = from_wire::<HkVvb3>(b"HKVVB:4:3+1000+0+1+Testprodukt+1.0'").unwrap_err();
        assert_eq!(e.field, "bpd_version");
    }
}
