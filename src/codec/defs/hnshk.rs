//! HNSHK, Signaturkopf (signature header). HBCI security 2024, B.5.1.
//!
//! The document is RDH/RAH-oriented; the values PIN/TAN uses are in the doc
//! comments, from PIN/TAN 2020, B.9.

use super::security::{
    Certificate2, HashAlgorithm2, KeyName3, SecurityIdentification2, SecurityProfile1,
    SecurityTimestamp2, SignatureAlgorithm2,
};
use crate::codec::{An, Code, Num, Segment};

/// HNSHK version 4.
///
/// ```text
/// Nr. Name                                   Typ  Format Länge Status Anzahl
///  1  Segmentkopf                            DEG               M     1
///  2  Sicherheitsprofil                      DEG               M     1
///  3  Sicherheitsfunktion, kodiert           DE   code   ..3   M     1    1, 2
///  4  Sicherheitskontrollreferenz            DE   an     ..14  M     1    <>0
///  5  Bereich der Sicherheitsapplikation     DE   code   ..3   M     1    1
///  6  Rolle des Sicherheitslieferanten       DE   code   ..3   M     1    1, 3, 4
///  7  Sicherheitsidentifikation, Details     DEG               M     1
///  8  Sicherheitsreferenznummer              DE   num    ..16  M     1
///  9  Sicherheitsdatum und -uhrzeit          DEG               M     1
/// 10  Hashalgorithmus                        DEG               M     1
/// 11  Signaturalgorithmus                    DEG               M     1
/// 12  Schlüsselname                          DEG               M     1
/// 13  Zertifikat                             DEG               C     1    N for RAH-10 and PIN/TAN
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HNSHK", version = 4)]
pub struct HnShk4 {
    /// 2 Sicherheitsprofil: PIN/TAN uses `PIN:1` (one-step) or `PIN:2`
    /// (two-step) (PIN/TAN B.9.1).
    pub profile: SecurityProfile1,
    /// 3 Sicherheitsfunktion, kodiert: the HBCI document lists 1 and 2 for
    /// RAH; PIN/TAN uses `999` in the one-step method and the BPD's
    /// `900`..`997` in a two-step method (PIN/TAN B.9.4).
    #[de(max = 3)]
    pub security_function: Code,
    /// 4 Sicherheitskontrollreferenz: any value other than 0, repeated in
    /// HNSHA.
    #[de(max = 14)]
    pub control_reference: An,
    /// 5 Bereich der Sicherheitsapplikation, kodiert: only `1` (SHM).
    #[de(max = 3)]
    pub application_area: Code,
    /// 6 Rolle des Sicherheitslieferanten, kodiert: 1 issuer, 3 supporter,
    /// 4 witness; a bank need not evaluate it.
    #[de(max = 3)]
    pub role: Code,
    /// 7 Sicherheitsidentifikation, Details.
    pub identification: SecurityIdentification2,
    /// 8 Sicherheitsreferenznummer (signature ID).
    #[de(max = 16)]
    pub reference_number: Num,
    /// 9 Sicherheitsdatum und -uhrzeit: kind `1`.
    pub timestamp: SecurityTimestamp2,
    /// 10 Hashalgorithmus.
    pub hash_algorithm: HashAlgorithm2,
    /// 11 Signaturalgorithmus.
    pub signature_algorithm: SignatureAlgorithm2,
    /// 12 Schlüsselname.
    pub key_name: KeyName3,
    /// 13 Zertifikat: mandatory for RAH-7 with security class 2-4, optional
    /// for RAH-9, never with RAH-10; PIN/TAN must not fill it (PIN/TAN B.9.4).
    pub certificate: Option<Certificate2>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HNSHK:2:4+PIN:2+921+4711+1+1+2::SYS12345+1+1:20260115:101500+1:999:1+6:10:16+280:12345678:USER0001:S:0:0'",
    "HNSHK:2:4+PIN:1+999+4711+1+1+1::0+1+1:20260115:101500+1:999:1+6:10:16+280:12345678:USER0001:S:0:0'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{from_wire, to_wire};

    #[test]
    fn wire_round_trips() {
        let v: HnShk4 = round_trip(SAMPLES[0]);
        assert_eq!(v.profile.method.as_str(), "PIN");
        assert_eq!(v.profile.version, Num::new(2));
        assert_eq!(v.security_function.as_str(), "921");
        assert_eq!(
            v.identification.party_id.as_ref().unwrap().as_str(),
            "SYS12345"
        );
        assert!(v.identification.cid.is_none());
        assert_eq!(v.key_name.user_id.as_str(), "USER0001");
        assert!(v.certificate.is_none());
        let first: HnShk4 = round_trip(SAMPLES[1]);
        assert_eq!(first.identification.party_id.unwrap().as_str(), "0");
    }

    #[test]
    fn certificate_round_trips_when_present() {
        let wire = format!("{}+1:@3@abc'", SAMPLES[0].trim_end_matches('\''));
        let v: HnShk4 = round_trip(&wire);
        assert_eq!(v.certificate.unwrap().content.as_bytes(), b"abc");
    }

    /// Deliberate: the Bezeichner für
    /// Sicherheitspartei (first element of DE 7) is `1`, message sender, in
    /// customer messages (HBCI security D; PIN/TAN example F.2.2 shows
    /// `1::...`). Not `2`. HNVSK DE 5 likewise
    /// (`hnvsk::tests::pin_tan_security_function_is_998`).
    #[test]
    fn customer_security_party_is_1() {
        let ctx = crate::message::SecurityContext {
            dialog_id: "0",
            message_number: 1,
            blz: "12345678",
            user_id: "USER0001",
            system_id: "SYS12345",
            pin: "12345",
            tan: None,
            security_function: "921",
        };
        let at = jiff::civil::date(2026, 1, 15).at(10, 15, 0, 0);
        let built = to_wire(&crate::message::hnshk(&ctx, 4711, at).unwrap(), 2, None).unwrap();
        let expected = SAMPLES[0].replace("+2::SYS12345+", "+1::SYS12345+");
        assert_eq!(built, expected.as_bytes());
        let v = from_wire::<HnShk4>(&built).unwrap().1;
        assert_eq!(v.profile.version, Num::new(2));
        assert_eq!(v.control_reference.as_str(), "4711");
        assert_eq!(v.identification.party.as_str(), "1");
        assert_eq!(
            v.key_name.bank.bank_code.as_ref().unwrap().as_str(),
            "12345678"
        );
    }

    #[test]
    fn rules_are_enforced() {
        // Sicherheitskontrollreferenz is an ..14.
        let wire = SAMPLES[0].replace("4711", "123456789012345");
        let e = from_wire::<HnShk4>(wire.as_bytes()).unwrap_err();
        assert_eq!((e.de, e.field.as_str()), (Some(4), "control_reference"));
        // The timestamp's date must be a date.
        let wire = SAMPLES[0].replace("20260115", "20261315");
        let e = from_wire::<HnShk4>(wire.as_bytes()).unwrap_err();
        assert_eq!(e.field, "timestamp.date");
    }
}
