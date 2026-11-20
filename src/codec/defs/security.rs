//! Groups shared by the security envelope segments HNSHK, HNSHA, HNVSK
//! (written from the HBCI security document, "Security - Sicherheitsverfahren
//! HBCI" 2024, chapter D "Data Dictionary"; the values PIN/TAN uses come from
//! the PIN/TAN document 2020, B.9 "Besondere Belegungsrichtlinien").
//!
//! The general layout is RDH/RAH-oriented; every PIN/TAN rule that narrows it
//! is in the doc comment of the field it narrows.

use super::common::Kik1;
use crate::codec::{An, Bin, Code, Dat, DataElementGroup, Id, Num, Tim};

/// Sicherheitsprofil, version 1. HBCI security, D "Sicherheitsprofil".
///
/// PIN/TAN (PIN/TAN B.9.1): method `PIN`; version `1` in a one-step dialog,
/// `2` in a two-step dialog, in all messages.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SecurityProfile1 {
    /// 1 Sicherheitsverfahren, Code, `code 3`, M: `RAH` or `PIN`.
    #[de(len = 3)]
    pub method: Code,
    /// 2 Version des Sicherheitsverfahrens, `num ..3`, M: `1`, 7, 9, 10.
    #[de(max = 3)]
    pub version: Num,
}

/// Sicherheitsidentifikation, Details, version 2. HBCI security, D
/// "Sicherheitsidentifikation, Details".
///
/// PIN/TAN (B.9.3): `cid` must stay empty; `party_id` carries the customer
/// system ID obtained earlier from the bank (`0` while synchronising).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SecurityIdentification2 {
    /// 1 Bezeichner für Sicherheitspartei, `code ..3`, M: `1` message
    /// sender (customer to bank), `2` message receiver (bank to customer).
    #[de(max = 3)]
    pub party: Code,
    /// 2 CID, `bin ..256`, C: M for chip cards, N otherwise.
    #[de(max = 256)]
    pub cid: Option<Bin>,
    /// 3 Identifizierung der Partei, `id`, C: O, ignored by the bank.
    pub party_id: Option<Id>,
}

/// Sicherheitsdatum und -uhrzeit, version 2. HBCI security, D
/// "Sicherheitsdatum und -uhrzeit".
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SecurityTimestamp2 {
    /// 1 Datum- und Zeitbezeichner, kodiert, `code ..3`, M: `1` security
    /// time stamp (the value both HNSHK and HNVSK use), `6` revocation time.
    #[de(max = 3)]
    pub kind: Code,
    /// 2 Datum, `dat`, O.
    pub date: Option<Dat>,
    /// 3 Uhrzeit, `tim`, C: O when the date is given, N otherwise.
    pub time: Option<Tim>,
}

/// Hashalgorithmus, version 2. HBCI security, D "Hashalgorithmus".
///
/// PIN/TAN (B.9.5): `parameter_value` must stay empty; the other elements
/// are filler values (the HBCI document lists `1`, `3..6`, `1`).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct HashAlgorithm2 {
    /// 1 Verwendung des Hashalgorithmus, kodiert, `code ..3`, M: `1` owner hashing.
    #[de(max = 3)]
    pub usage: Code,
    /// 2 Hashalgorithmus, kodiert, `code ..3`, M: 3 SHA-256, 4 SHA-384,
    /// 5 SHA-512, 6 SHA-256/SHA-256; 1 and 999 are not allowed.
    #[de(max = 3)]
    pub algorithm: Code,
    /// 3 Bezeichner für Hashalgorithmusparameter, `code ..3`, M: `1`.
    #[de(max = 3)]
    pub parameter_name: Code,
    /// 4 Wert des Hashalgorithmusparameters, `bin ..512`, O.
    #[de(max = 512)]
    pub parameter_value: Option<Bin>,
}

/// Signaturalgorithmus, version 2. HBCI security, D "Signaturalgorithmus".
///
/// PIN/TAN (B.9.6): `algorithm` and `mode` are filler values (the PIN/TAN
/// document suggests `10` and `16`).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SignatureAlgorithm2 {
    /// 1 Verwendung des Signaturalgorithmus, kodiert, `code ..3`, M: `6`.
    #[de(max = 3)]
    pub usage: Code,
    /// 2 Signaturalgorithmus, kodiert, `code ..3`, M: `10` RSA (RAH).
    #[de(max = 3)]
    pub algorithm: Code,
    /// 3 Operationsmodus, kodiert, `code ..3`, M: `19`.
    #[de(max = 3)]
    pub mode: Code,
}

/// Verschlüsselungsalgorithmus, version 2. HBCI security, D
/// "Verschlüsselungsalgorithmus".
///
/// PIN/TAN (B.9.9): `key_value` is a filler (for example eight zero bytes),
/// `key_parameter_name` a filler (for example `5`), `iv_value` must stay
/// empty.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct EncryptionAlgorithm2 {
    /// 1 Verwendung des Verschlüsselungsalgorithmus, kodiert, `code ..3`, M: `2`.
    #[de(max = 3)]
    pub usage: Code,
    /// 2 Operationsmodus, kodiert, `code ..3`, M: 2, 18, 19.
    #[de(max = 3)]
    pub mode: Code,
    /// 3 Verschlüsselungsalgorithmus, kodiert, `code ..3`, M: 13 (2-key
    /// triple DES, not allowed), 14 (AES-256).
    #[de(max = 3)]
    pub algorithm: Code,
    /// 4 Wert des Algorithmusparameters, Schlüssel, `bin ..512`, M.
    #[de(max = 512)]
    pub key_value: Bin,
    /// 5 Bezeichner für Algorithmusparameter, Schlüssel, `code ..3`, M: `6`.
    #[de(max = 3)]
    pub key_parameter_name: Code,
    /// 6 Bezeichner für Algorithmusparameter, IV, `code ..3`, M: `1`.
    #[de(max = 3)]
    pub iv_parameter_name: Code,
    /// 7 Wert des Algorithmusparameters, IV, `bin ..512`, O.
    #[de(max = 512)]
    pub iv_value: Option<Bin>,
}

/// Schlüsselname, version 3. HBCI security, D "Schlüsselname".
///
/// PIN/TAN (B.9.2): `key_number` and `key_version` are filler values, for
/// example `0`.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct KeyName3 {
    /// 1 Kreditinstitutskennung, `kik`, M.
    pub bank: Kik1,
    /// 2 Benutzerkennung, `id`, M.
    pub user_id: Id,
    /// 3 Schlüsselart, `code 1`, M: `D`, `S`, `V`.
    #[de(len = 1)]
    pub key_type: Code,
    /// 4 Schlüsselnummer, `num ..3`, M.
    #[de(max = 3)]
    pub key_number: Num,
    /// 5 Schlüsselversion, `num ..3`, M.
    #[de(max = 3)]
    pub key_version: Num,
}

/// Zertifikat, version 2. HBCI security, D "Zertifikat". PIN/TAN never
/// fills it (B.9.4, B.9.8).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Certificate2 {
    /// 1 Zertifikatstyp, `code 1`, M: 1, 2, 3.
    #[de(len = 1)]
    pub certificate_type: Code,
    /// 2 Zertifikatsinhalt, `bin ..4096`, M.
    #[de(max = 4096)]
    pub content: Bin,
}

/// Benutzerdefinierte Signatur for PIN/TAN, version 1. PIN/TAN 2020, D
/// "Benutzerdefinierte Signatur" (the HBCI document only refers to it).
///
/// `Debug` hides PIN and TAN: they must not reach a log.
#[derive(Clone, PartialEq, Eq, DataElementGroup)]
pub struct UserDefinedSignature1 {
    /// PIN, `an ..99`, M (the table numbers it 6).
    #[de(max = 99)]
    pub pin: An,
    /// TAN, `an ..99`, O (the table numbers it 7). Present only for a
    /// one-step TAN or after a decoupled confirmation; with process
    /// variant 2 a TAN travels in HKTAN, not here (PIN/TAN B.9.7).
    #[de(max = 99)]
    pub tan: Option<An>,
}

impl std::fmt::Debug for UserDefinedSignature1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserDefinedSignature1")
            .field("pin", &"<hidden>")
            .field("tan", &self.tan.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}
