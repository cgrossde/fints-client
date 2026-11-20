//! FinTS message construction.
//!
//! Assembles a complete customer message from typed [`Segment`] values. The
//! security envelope (HNVSK/HNVSD/HNSHK/HNSHA) and the framing (HNHBK/HNHBS)
//! are built here with the codec's declarative definitions
//! (`codec::defs`), never exposed as raw data elements.
//!
//! PIN/TAN values (PIN/TAN 2020, B.9; HBCI security 2024, B.5):
//! - Sicherheitsprofil `PIN:1` for the one-step method (security function
//!   998/999), `PIN:2` for a two-step method (B.9.1), in HNSHK and HNVSK.
//! - HNSHK DE 3 is the dialog's security function; HNVSK DE 3 is always
//!   `998` (B.9.8).
//! - Bezeichner für Sicherheitspartei is `1` (message sender) in customer
//!   messages (HBCI security D; PIN/TAN example F.2.2).
//! - HNSHA carries `PIN[:TAN]`; without a TAN the TAN element is left out
//!   (Formals H.1.5).

use jiff::civil::DateTime;

use crate::codec::defs::common::Kik1;
use crate::codec::defs::security::{
    EncryptionAlgorithm2, HashAlgorithm2, KeyName3, SecurityIdentification2, SecurityProfile1,
    SecurityTimestamp2, SignatureAlgorithm2, UserDefinedSignature1,
};
use crate::codec::defs::{HnHbk3, HnHbs1, HnSha2, HnShk4, HnVsd1, HnVsk3};
use crate::codec::{self, An, Bin, Code, Dat, Dig, Id, Num, RawSegment, Segment as _, Tim};
use crate::error::FinTSError;
use crate::protocol::{BankParams, Segment};
use crate::segments::verbatim;

/// Segment number of HNVSK (PIN/TAN B.9.8; Formals B.5.2: fixed numbers
/// 998 and 999 for the encryption envelope).
const HNVSK_NUMBER: u16 = 998;
/// Segment number of HNVSD.
const HNVSD_NUMBER: u16 = 999;
/// Number of the first segment inside HNVSD (HNSHK); HNHBK is 1.
const FIRST_INNER_NUMBER: u16 = 2;

/// Security context for message construction.
pub(crate) struct SecurityContext<'a> {
    pub dialog_id: &'a str,
    pub message_number: u16,
    pub blz: &'a str,
    pub user_id: &'a str,
    pub system_id: &'a str,
    pub pin: &'a str,
    pub tan: Option<&'a str>,
    pub security_function: &'a str,
}

/// Sicherheitsprofil version: 1 for the one-step method (998/999), 2 for
/// a two-step method (PIN/TAN B.9.1). A function that is not a number is
/// a two-step method.
fn profile_version(security_function: &str) -> u64 {
    match security_function.parse::<u16>() {
        Ok(sf) if sf >= 998 => 1,
        _ => 2,
    }
}

fn profile(security_function: &str) -> SecurityProfile1 {
    SecurityProfile1 {
        method: Code::new("PIN").expect("constant"),
        version: Num::new(profile_version(security_function)),
    }
}

fn code(value: &str) -> Code {
    Code::new(value).expect("constant code")
}

fn serialize_error(segment: &str) -> impl Fn(codec::CodecError) -> FinTSError + '_ {
    move |e| FinTSError::Serialize(format!("{segment}: {e}"))
}

/// Sicherheitsidentifikation, Details: party `1` (message sender, HBCI
/// security D) and the customer system ID.
fn identification(system_id: &str) -> Result<SecurityIdentification2, FinTSError> {
    Ok(SecurityIdentification2 {
        party: code("1"),
        cid: None,
        party_id: if system_id.is_empty() {
            None
        } else {
            Some(verbatim::<Id>(system_id, "system ID")?)
        },
    })
}

/// Sicherheitsdatum und -uhrzeit: kind `1` (STS), local date and time.
fn timestamp(at: DateTime) -> Result<SecurityTimestamp2, FinTSError> {
    let invalid = |_| FinTSError::Serialize("security timestamp out of range".into());
    Ok(SecurityTimestamp2 {
        kind: code("1"),
        date: Some(Dat::new(at.date()).map_err(invalid)?),
        time: Some(Tim::new(at.time()).map_err(invalid)?),
    })
}

/// Schlüsselname: `280:<blz>:<user>:<key type>:0:0`.
fn key_name(ctx: &SecurityContext<'_>, key_type: &str) -> Result<KeyName3, FinTSError> {
    Ok(KeyName3 {
        bank: Kik1 {
            country: Dig::new("280").expect("constant"),
            bank_code: if ctx.blz.is_empty() {
                None
            } else {
                Some(verbatim::<An>(ctx.blz, "bank code")?)
            },
        },
        user_id: verbatim::<Id>(ctx.user_id, "user ID")?,
        key_type: code(key_type),
        key_number: Num::new(0),
        key_version: Num::new(0),
    })
}

/// HNSHK v4, signature header.
pub(crate) fn hnshk(
    ctx: &SecurityContext<'_>,
    control_reference: u32,
    at: DateTime,
) -> Result<HnShk4, FinTSError> {
    Ok(HnShk4 {
        profile: profile(ctx.security_function),
        security_function: verbatim::<Code>(ctx.security_function, "security function")?,
        control_reference: An::new(control_reference.to_string()).expect("digits"),
        // Bereich der Sicherheitsapplikation: 1 = SHM.
        application_area: code("1"),
        // Rolle des Sicherheitslieferanten: 1 = issuer.
        role: code("1"),
        identification: identification(ctx.system_id)?,
        reference_number: Num::new(1),
        timestamp: timestamp(at)?,
        // PIN/TAN B.9.4: placeholders, the bank does not evaluate them.
        hash_algorithm: HashAlgorithm2 {
            usage: code("1"),
            algorithm: code("999"),
            parameter_name: code("1"),
            parameter_value: None,
        },
        signature_algorithm: SignatureAlgorithm2 {
            usage: code("6"),
            algorithm: code("10"),
            mode: code("16"),
        },
        key_name: key_name(ctx, "S")?,
        certificate: None,
    })
}

/// HNVSK v3, encryption header. DE 3 is `998` for PIN/TAN (B.9.8); the
/// profile version follows the dialog's security function.
pub(crate) fn hnvsk(ctx: &SecurityContext<'_>, at: DateTime) -> Result<HnVsk3, FinTSError> {
    Ok(HnVsk3 {
        profile: profile(ctx.security_function),
        security_function: code("998"),
        role: code("1"),
        identification: identification(ctx.system_id)?,
        timestamp: timestamp(at)?,
        // PIN/TAN B.9.8: dummy 2-key triple DES, key of eight zero bytes.
        encryption_algorithm: EncryptionAlgorithm2 {
            usage: code("2"),
            mode: code("2"),
            algorithm: code("13"),
            key_value: Bin(vec![0u8; 8]),
            key_parameter_name: code("5"),
            iv_parameter_name: code("1"),
            iv_value: None,
        },
        key_name: key_name(ctx, "V")?,
        compression: code("0"),
        certificate: None,
    })
}

/// HNSHA v2, signature trailer with `PIN[:TAN]`. An empty TAN is no TAN.
pub(crate) fn hnsha(
    ctx: &SecurityContext<'_>,
    control_reference: u32,
) -> Result<HnSha2, FinTSError> {
    // The error must not quote the credential.
    let credential = |what: &str| {
        let what = what.to_string();
        move |_| {
            FinTSError::Serialize(format!(
                "HNSHA: {what} cannot be sent (ISO 8859-1 text, no line breaks)"
            ))
        }
    };
    Ok(HnSha2 {
        control_reference: An::new(control_reference.to_string()).expect("digits"),
        validation_result: None,
        user_signature: Some(UserDefinedSignature1 {
            pin: verbatim::<An>(ctx.pin, "PIN").map_err(credential("PIN"))?,
            tan: ctx
                .tan
                .filter(|t| !t.is_empty())
                .map(|t| verbatim::<An>(t, "TAN").map_err(credential("TAN")))
                .transpose()?,
        }),
    })
}

/// A whole message carrying `segments`, numbered from 3 on.
///
/// The message header HNHBK (with the final size) and trailer HNHBS
/// enclose the PIN/TAN encryption pair: the encryption header HNVSK (998)
/// and HNVSD (999), whose one binary element holds the signed part. That
/// part starts with the signature header HNSHK, then the segments, and
/// closes with HNSHA, which carries PIN and TAN (Security B.5 and B.9).
pub(crate) fn build_from_segments(
    ctx: &SecurityContext<'_>,
    segments: &[Segment],
    params: &BankParams,
    at: DateTime,
) -> Result<Vec<u8>, FinTSError> {
    let business: Vec<RawSegment> = segments
        .iter()
        .zip(FIRST_INNER_NUMBER + 1..)
        .map(|(s, number)| s.encode(params, number))
        .collect::<Result<_, _>>()?;
    let control_reference = rand::random_range(1_000_000u32..10_000_000u32);
    assemble(ctx, &business, at, control_reference)
}

/// Wrap business segments (already numbered from 3) in the envelope.
/// `at` stamps HNSHK and HNVSK; `control_reference` links HNSHK and HNSHA.
pub(crate) fn assemble(
    ctx: &SecurityContext<'_>,
    business: &[RawSegment],
    at: DateTime,
    control_reference: u32,
) -> Result<Vec<u8>, FinTSError> {
    let count = u16::try_from(business.len())
        .map_err(|_| FinTSError::Serialize("too many segments in one message".into()))?;
    let hnsha_number = FIRST_INNER_NUMBER + 1 + count;
    let trailer_number = hnsha_number + 1;

    let mut inner = Vec::new();
    inner.extend(
        codec::to_wire(
            &hnshk(ctx, control_reference, at)?,
            FIRST_INNER_NUMBER,
            None,
        )
        .map_err(serialize_error("HNSHK"))?,
    );
    for seg in business {
        inner.extend(codec::wire::serialize_segment(seg).map_err(serialize_error("segment"))?);
    }
    inner.extend(
        codec::to_wire(&hnsha(ctx, control_reference)?, hnsha_number, None)
            .map_err(|_| FinTSError::Serialize("HNSHA cannot be encoded".into()))?,
    );

    let mut body = Vec::new();
    body.extend(
        codec::to_wire(&hnvsk(ctx, at)?, HNVSK_NUMBER, None).map_err(serialize_error("HNVSK"))?,
    );
    body.extend(
        codec::to_wire(&HnVsd1 { data: Bin(inner) }, HNVSD_NUMBER, None)
            .map_err(serialize_error("HNVSD"))?,
    );
    body.extend(
        codec::to_wire(
            &HnHbs1 {
                message_number: Num::new(u64::from(ctx.message_number)),
            },
            trailer_number,
            None,
        )
        .map_err(serialize_error("HNHBS"))?,
    );

    let header = |size: u64| -> Result<Vec<u8>, FinTSError> {
        let hnhbk = HnHbk3 {
            message_size: Dig::zero_padded(size, 12),
            hbci_version: Num::new(300),
            dialog_id: verbatim::<Id>(ctx.dialog_id, "dialog ID")?,
            message_number: Num::new(u64::from(ctx.message_number)),
            reference_message: None,
        };
        codec::to_wire(&hnhbk, 1, None).map_err(serialize_error(HnHbk3::CODE))
    };
    // The size field has a fixed width, so the header length does not
    // depend on the size written into it.
    let total = header(0)?.len() + body.len();
    let mut message = header(total as u64)?;
    if message.len() + body.len() != total {
        return Err(FinTSError::Serialize(
            "HNHBK size changed after size update".into(),
        ));
    }
    message.extend(body);
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(security_function: &'a str, tan: Option<&'a str>) -> SecurityContext<'a> {
        SecurityContext {
            dialog_id: "0",
            message_number: 1,
            blz: "12030000",
            user_id: "testuser",
            system_id: "0",
            pin: "12345",
            tan,
            security_function,
        }
    }

    fn at() -> DateTime {
        jiff::civil::date(2026, 1, 15).at(10, 15, 0, 0)
    }

    fn hkidn() -> RawSegment {
        codec::wire::tokenize(b"HKIDN:3:2+280:12030000+testuser+0+1'")
            .unwrap()
            .remove(0)
    }

    fn text(bytes: &[u8]) -> String {
        bytes.iter().map(|&b| char::from(b)).collect()
    }

    /// The whole message, byte for byte. PIN/TAN values per the module
    /// documentation: HNVSK DE 3 is `998` (PIN/TAN B.9.8), the security
    /// party is `1` (HBCI security D, PIN/TAN F.2.2), no TAN element
    /// without a TAN (Formals H.1.5).
    #[test]
    fn two_step_message_wire_format() {
        let msg = assemble(&ctx("921", None), &[hkidn()], at(), 4711).unwrap();
        let inner = "HNSHK:2:4+PIN:2+921+4711+1+1+1::0+1+1:20260115:101500+1:999:1+6:10:16+280:12030000:testuser:S:0:0'\
                     HKIDN:3:2+280:12030000+testuser+0+1'\
                     HNSHA:4:2+4711++12345'";
        let body = format!(
            "HNVSK:998:3+PIN:2+998+1+1::0+1:20260115:101500+2:2:13:@8@{}:5:1+280:12030000:testuser:V:0:0+0'\
             HNVSD:999:1+@{}@{inner}'HNHBS:5:1+1'",
            "\0".repeat(8),
            inner.len()
        );
        let header = format!("HNHBK:1:3+{:012}+300+0+1'", 31 + body.len());
        assert_eq!(text(&msg), format!("{header}{body}"));
    }

    #[test]
    fn one_step_message_uses_profile_1_and_tan_follows_the_pin() {
        let msg = text(&assemble(&ctx("999", Some("654321")), &[hkidn()], at(), 4711).unwrap());
        assert!(msg.contains("HNVSK:998:3+PIN:1+998+1+1::0+"), "{msg}");
        assert!(msg.contains("HNSHK:2:4+PIN:1+999+4711+"), "{msg}");
        assert!(msg.contains("HNSHA:4:2+4711++12345:654321'"), "{msg}");
        // An empty TAN is no TAN.
        let msg = text(&assemble(&ctx("999", Some("")), &[hkidn()], at(), 4711).unwrap());
        assert!(msg.contains("HNSHA:4:2+4711++12345'"), "{msg}");
    }

    /// The TAN for an order's challenge (PIN/TAN B.4.2): HKTAN process 2
    /// alone with the order reference, the TAN after the PIN in HNSHA
    /// (`PIN:TAN`, Formals "Benutzerdefinierte Signatur"), two-step
    /// profile (PIN:2) with the selected security function.
    #[test]
    fn order_tan_message_wire_format() {
        let hktan = codec::wire::tokenize(b"HKTAN:3:6+2++++TASK0001+N'")
            .unwrap()
            .remove(0);
        let msg = text(&assemble(&ctx("921", Some("123456")), &[hktan], at(), 4711).unwrap());
        assert!(msg.contains("HNSHK:2:4+PIN:2+921+4711+"), "{msg}");
        assert!(
            msg.contains("'HKTAN:3:6+2++++TASK0001+N'HNSHA:4:2+4711++12345:123456'"),
            "{msg}"
        );
    }

    #[test]
    /// Outside the encryption pair the message has four segments; the
    /// signed part sits inside HNVSD.
    fn outer_segments_and_numbers() {
        let msg = assemble(&ctx("999", None), &[hkidn()], at(), 4711).unwrap();
        let outer: Vec<String> = codec::wire::tokenize(&msg)
            .unwrap()
            .iter()
            .map(|s| format!("{}:{}", s.segment_type(), s.segment_number()))
            .collect();
        // Inside: HNSHK 2, HKIDN 3, HNSHA 4; the trailer follows as 5.
        assert_eq!(outer, ["HNHBK:1", "HNVSK:998", "HNVSD:999", "HNHBS:5"]);
    }

    #[test]
    fn test_message_size_in_header() {
        let msg = assemble(&ctx("999", None), &[hkidn()], at(), 4711).unwrap();
        let (_, hnhbk) =
            codec::decode_segment::<HnHbk3>(&codec::wire::tokenize(&msg).unwrap()[0]).unwrap();
        assert_eq!(hnhbk.message_size.value(), Some(msg.len() as u64));
    }

    #[test]
    fn credentials_stay_out_of_errors() {
        let mut c = ctx("999", Some("12\n34"));
        let err = assemble(&c, &[hkidn()], at(), 4711).unwrap_err();
        assert!(!err.to_string().contains("12\n34"), "{err}");
        c.tan = None;
        c.pin = "pin\u{20ac}";
        let err = assemble(&c, &[hkidn()], at(), 4711).unwrap_err();
        assert!(!err.to_string().contains('\u{20ac}'), "{err}");
    }

    #[test]
    fn non_numeric_security_function_does_not_panic() {
        let msg = text(&assemble(&ctx("X", None), &[hkidn()], at(), 4711).unwrap());
        assert!(msg.contains("HNSHK:2:4+PIN:2+X+"), "{msg}");
    }
}
