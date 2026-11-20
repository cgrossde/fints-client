//! HNVSK, Verschlüsselungskopf (encryption header). HBCI security 2024,
//! B.5.3. PIN/TAN sends a plain-text header with filler values (PIN/TAN
//! 2020, B.9.8, B.9.9).

use super::security::{
    Certificate2, EncryptionAlgorithm2, KeyName3, SecurityIdentification2, SecurityProfile1,
    SecurityTimestamp2,
};
use crate::codec::{Code, Segment};

/// HNVSK version 3.
///
/// ```text
/// Nr. Name                                   Typ  Format Länge Status Anzahl
///  1  Segmentkopf                            DEG               M     1
///  2  Sicherheitsprofil                      DEG               M     1
///  3  Sicherheitsfunktion, kodiert           DE   code   ..3   M     1    4
///  4  Rolle des Sicherheitslieferanten       DE   code   ..3   M     1    1, 4
///  5  Sicherheitsidentifikation, Details     DEG               M     1
///  6  Sicherheitsdatum und -uhrzeit          DEG               M     1
///  7  Verschlüsselungsalgorithmus            DEG               M     1
///  8  Schlüsselname                          DEG               M     1
///  9  Komprimierungsfunktion                 DE   code   ..3   M     1
/// 10  Zertifikat                             DEG               C     1    N for PIN/TAN
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HNVSK", version = 3)]
pub struct HnVsk3 {
    /// 2 Sicherheitsprofil: `PIN:1` or `PIN:2` for PIN/TAN.
    pub profile: SecurityProfile1,
    /// 3 Sicherheitsfunktion, kodiert: the HBCI document says `4`; PIN/TAN
    /// uses `998` (clear text) (PIN/TAN B.9.8).
    #[de(max = 3)]
    pub security_function: Code,
    /// 4 Rolle des Sicherheitslieferanten, kodiert: 1 or 4.
    #[de(max = 3)]
    pub role: Code,
    /// 5 Sicherheitsidentifikation, Details.
    pub identification: SecurityIdentification2,
    /// 6 Sicherheitsdatum und -uhrzeit: kind `1`.
    pub timestamp: SecurityTimestamp2,
    /// 7 Verschlüsselungsalgorithmus: filler values for PIN/TAN, the
    /// transport encryption does the work.
    pub encryption_algorithm: EncryptionAlgorithm2,
    /// 8 Schlüsselname.
    pub key_name: KeyName3,
    /// 9 Komprimierungsfunktion: `0` (none).
    #[de(max = 3)]
    pub compression: Code,
    /// 10 Zertifikat: optional for the bank with RAH-7/9, for the customer
    /// with RAH-7; PIN/TAN must not fill it.
    pub certificate: Option<Certificate2>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HNVSK:998:3+PIN:2+998+1+2::SYS12345+1:20260115:101500+2:2:13:@8@\0\0\0\0\0\0\0\0:5:1+280:12345678:USER0001:V:0:0+0'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{from_wire, to_wire};

    #[test]
    fn wire_round_trips() {
        let v: HnVsk3 = round_trip(SAMPLES[0]);
        assert_eq!(v.security_function.as_str(), "998");
        assert_eq!(v.encryption_algorithm.key_value.as_bytes(), &[0u8; 8]);
        assert_eq!(v.encryption_algorithm.key_parameter_name.as_str(), "5");
        assert!(v.encryption_algorithm.iv_value.is_none());
        assert_eq!(v.key_name.key_type.as_str(), "V");
        assert_eq!(v.compression.as_str(), "0");
    }

    /// Deliberate: DE 3 Sicherheitsfunktion is
    /// always `998` for PIN/TAN (PIN/TAN B.9.8), not the dialog's TAN
    /// function (`921`). The Sicherheitsprofil version
    /// still follows the dialog's function (`PIN:2`, PIN/TAN B.9.1). The
    /// security party is `1`, see `hnshk::tests::customer_security_party_is_1`.
    #[test]
    fn pin_tan_security_function_is_998() {
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
        let built = to_wire(&crate::message::hnvsk(&ctx, at).unwrap(), 998, None).unwrap();
        let expected = SAMPLES[0].replace("+2::SYS12345+", "+1::SYS12345+");
        assert_eq!(built, expected.as_bytes());
        let v = from_wire::<HnVsk3>(&built).unwrap().1;
        assert_eq!(v.key_name.user_id.as_str(), "USER0001");
        assert_eq!(v.profile.version, crate::codec::Num::new(2));
        assert_eq!(v.security_function.as_str(), "998");
        assert_eq!(v.identification.party.as_str(), "1");
    }

    #[test]
    fn rules_are_enforced() {
        // Schlüsselart is one character.
        let wire = SAMPLES[0].replace(":V:", ":VV:");
        let e = from_wire::<HnVsk3>(wire.as_bytes()).unwrap_err();
        assert_eq!(e.field, "key_name.key_type");
        // The key value is binary, not text.
        let wire = SAMPLES[0].replace("@8@\0\0\0\0\0\0\0\0", "00000000");
        let e = from_wire::<HnVsk3>(wire.as_bytes()).unwrap_err();
        assert_eq!(e.field, "encryption_algorithm.key_value");
    }
}
