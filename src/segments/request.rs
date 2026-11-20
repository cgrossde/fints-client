//! Outgoing segments, built from the codec's declarative definitions.
//!
//! Each function fills one definition from `codec::defs` with the values a
//! read-only PIN/TAN client sends. The protocol encodes the result with the
//! segment number it assigns (`segments::encode`).

use jiff::civil::Date;

use crate::codec::defs::common::{Kik1, Kti1, Ktv3};
use crate::codec::defs::{
    HkCaz1, HkCcs1, HkEnd1, HkIdn2, HkIps1, HkIpz1, HkIpz2, HkKaz5, HkKaz6, HkKaz7, HkSal5, HkSal6,
    HkSal7, HkSal8, HkSyn3, HkTan5, HkTan6, HkTan7, HkVpa1, HkVpp1, HkVvb3, HkWdu5, HkWpd5, HkWpd6,
    SupportedCamtMessages1, SupportedPainMessages1, SupportedPaymentStatusReports1,
};
use crate::codec::{An, Bin, Code, Cur, Dat, Dig, Id, Jn, Num, RawSegment};
use crate::error::{AddressingReason, FinTSError, Result};
use crate::protocol::Account;
use crate::segments::{encode, not_addressable, unbuildable, verbatim};

/// HKTAN versions this client builds (`hktan`).
pub(crate) const HKTAN_VERSIONS: &[u16] = &[5, 6, 7];

/// camt format requested when the bank's HICAZS names none we recognise.
pub(crate) const CAMT052_DEFAULT: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02";

/// Product version sent in HKVVB (Formals C.3.1.3, `an ..5`).
const PRODUCT_VERSION: &str = "1.0";

fn code(value: &str) -> Code {
    Code::new(value).expect("constant code")
}

/// Kreditinstitutskennung `280:<blz>` (Messages B.2).
pub(crate) fn kik(blz: &str) -> Result<Kik1> {
    Ok(Kik1 {
        country: Dig::new("280").expect("constant"),
        bank_code: if blz.is_empty() {
            None
        } else {
            Some(verbatim::<An>(blz, "bank code")?)
        },
    })
}

/// HKIDN v2, identification (Formals C.3.1.2). Kundensystem-Status `1`:
/// a customer system ID is required (PIN/TAN with HKSYN).
pub(crate) fn hkidn(blz: &str, user_id: &str, system_id: &str) -> Result<HkIdn2> {
    Ok(HkIdn2 {
        bank: kik(blz)?,
        customer_id: verbatim::<Id>(user_id, "user ID")?,
        system_id: verbatim::<Id>(system_id, "system ID")?,
        system_status: code("1"),
    })
}

/// HKVVB v3, processing preparation (Formals C.3.1.3). Dialogsprache `1`
/// (German); version 0 asks for the full BPD/UPD.
pub(crate) fn hkvvb(bpd_version: u16, upd_version: u16, product_id: &str) -> Result<HkVvb3> {
    Ok(HkVvb3 {
        bpd_version: Num::new(u64::from(bpd_version)),
        upd_version: Num::new(u64::from(upd_version)),
        language: code("1"),
        product_name: verbatim::<An>(product_id, "product ID")?,
        product_version: An::new(PRODUCT_VERSION).expect("constant"),
    })
}

/// HKSYN v3, synchronisation (Formals C.8.1.2). Modus `0`: assign a new
/// customer system ID.
pub(crate) fn hksyn() -> HkSyn3 {
    HkSyn3 { mode: code("0") }
}

/// HKEND v1, dialog end (Formals C.4.1.2).
pub(crate) fn hkend(dialog_id: &str) -> Result<HkEnd1> {
    Ok(HkEnd1 {
        dialog_id: verbatim::<Id>(dialog_id, "dialog ID")?,
    })
}

/// What an HKTAN asks for (PIN/TAN B.4, B.5).
#[derive(Debug, Clone, Copy)]
pub(crate) enum TanRequest<'a> {
    /// Process 4: start strong authentication for the order `segment`.
    Start { segment: &'a str },
    /// Process 2: the TAN for `task_reference` follows in HNSHA.
    Submit { task_reference: &'a str },
    /// Ask whether a decoupled confirmation for `task_reference` arrived.
    DecoupledStatus { task_reference: &'a str },
}

/// HKTAN v5, v6 or v7 (PIN/TAN E.1.5, B.5.1, B.5.2).
///
/// Process `S` (decoupled status) exists only in version 7 (B.5.2). In
/// versions 5 and 6 a decoupled confirmation is polled with process 2 and
/// the order reference, without a TAN (PIN/TAN B.5.2; common client
/// behaviour, `S` only from version 7; not tried at a bank, docs/banks/unverified.md).
/// HKTAN 5 keeps the segment code with process 4, a deliberate deviation
/// from PIN/TAN E.1.5 a.
pub(crate) fn hktan(
    version: u16,
    request: TanRequest<'_>,
    tan_medium: Option<&str>,
    number: u16,
) -> Result<RawSegment> {
    let (process, segment_code, order_reference, further_tan_follows) = match request {
        TanRequest::Start { segment } => ("4", Some(segment), None, None),
        TanRequest::Submit { task_reference } => ("2", None, Some(task_reference), Some(false)),
        TanRequest::DecoupledStatus { task_reference } => {
            let process = if version <= 6 { "2" } else { "S" };
            (process, None, Some(task_reference), Some(false))
        }
    };
    let text = |v: Option<&str>, what| {
        v.filter(|s| !s.is_empty())
            .map(|s| verbatim::<An>(s, what))
            .transpose()
    };
    let segment_code = text(segment_code, "TAN segment reference")?;
    let order_reference = text(order_reference, "TAN order reference")?;
    let tan_medium_name = text(tan_medium, "TAN medium name")?;
    let further_tan_follows = further_tan_follows.map(Jn);
    if version == 5 {
        return encode(
            &HkTan5 {
                process: code(process),
                segment_code,
                account: None,
                order_hash: None,
                order_reference,
                tan_list_number: None,
                further_tan_follows,
                cancel_order: None,
                sms_charge_account: None,
                challenge_class: None,
                challenge_class_parameters: None,
                tan_medium_name,
            },
            number,
        );
    }
    macro_rules! hktan_value {
        ($ty:ident) => {
            $ty {
                process: code(process),
                segment_code,
                account: None,
                order_hash: None,
                order_reference,
                further_tan_follows,
                cancel_order: None,
                sms_charge_account: None,
                challenge_class: None,
                challenge_class_parameters: None,
                tan_medium_name,
                hhd_uc_response: None,
            }
        };
    }
    match version {
        6 => encode(&hktan_value!(HkTan6), number),
        7 => encode(&hktan_value!(HkTan7), number),
        _ => Err(unbuildable("HKTAN", version)),
    }
}

// ── Account identification ─────────────────────────────────────────────
//
// Segment versions address accounts in one of two ways (Messages B.3):
//   ktv (national, Ktv3): number:subaccount:280:blz
//   kti (international, Kti1): iban:bic[:number:subaccount:280:blz]
// KTI is sent as `IBAN:BIC`, plus number, subaccount and 280:BLZ when the
// bank's HISPAS allows national account details and the account has them
// (some banks answer 9010 to a bare `IBAN:BIC`). National formats need the
// account number and BLZ, which `Account` derives from German IBANs or
// takes explicitly.

/// National account (Kontoverbindung, ktv): `number:subaccount:280:blz`.
fn ktv(account: &Account, segment: &str, version: u16) -> Result<Ktv3> {
    match (account.account_number(), account.blz()) {
        (Some(number), Some(blz)) => Ok(Ktv3 {
            account_number: verbatim::<Id>(number, "account number")?,
            subaccount: account
                .subaccount()
                .filter(|s| !s.is_empty())
                .map(|s| verbatim::<Id>(s, "subaccount"))
                .transpose()?,
            bank: kik(blz)?,
        }),
        _ => Err(not_addressable(
            segment,
            version,
            AddressingReason::NoNationalDetails,
        )),
    }
}

/// International account (Kontoverbindung international, kti): `IBAN:BIC`.
///
/// An account without IBAN (a credit card) is sent in the national-only
/// form `::number:subaccount:280:blz` (Messages B.3.2), when the bank's
/// HISPAS allows national account details and the account has number and
/// bank code. Otherwise it is not addressable.
fn kti(account: &Account, segment: &str, version: u16) -> Result<Kti1> {
    let national = match (
        account.kti_national(),
        account.account_number(),
        account.blz(),
    ) {
        (true, Some(_), Some(_)) => Some(ktv(account, segment, version)?),
        _ => None,
    };
    if account.iban().is_empty() {
        let Some(national) = national else {
            return Err(not_addressable(
                segment,
                version,
                AddressingReason::NoIbanBic,
            ));
        };
        return Ok(Kti1 {
            iban: None,
            bic: None,
            account_number: Some(national.account_number),
            subaccount: national.subaccount,
            bank: Some(national.bank),
        });
    }
    if account.bic().is_empty() {
        return Err(not_addressable(
            segment,
            version,
            AddressingReason::NoIbanBic,
        ));
    }
    Ok(Kti1 {
        iban: Some(verbatim::<An>(account.iban(), "IBAN")?),
        bic: Some(verbatim::<An>(account.bic(), "BIC")?),
        account_number: national.as_ref().map(|k| k.account_number.clone()),
        subaccount: national.as_ref().and_then(|k| k.subaccount.clone()),
        bank: national.map(|k| k.bank),
    })
}

fn date(d: Date) -> Result<Dat> {
    Dat::new(d).map_err(|e| FinTSError::Serialize(format!("date {d}: {e}")))
}

fn touchdown_point(touchdown: Option<&str>) -> Result<Option<An>> {
    touchdown
        .filter(|t| !t.is_empty())
        .map(|t| verbatim::<An>(t, "touchdown point"))
        .transpose()
}

/// HKSAL, balance (Messages C.2.1.2): `account+N+max entries+touchdown`.
/// v5/v6 address the account nationally, v7 and v8 by IBAN.
pub(crate) fn hksal(
    number: u16,
    version: u16,
    account: &Account,
    touchdown: Option<&str>,
) -> Result<RawSegment> {
    let all_accounts = Jn(false);
    let touchdown = touchdown_point(touchdown);
    match version {
        5 => encode(
            &HkSal5 {
                account: ktv(account, "HKSAL", version)?,
                all_accounts,
                max_entries: None,
                touchdown: touchdown?,
            },
            number,
        ),
        6 => encode(
            &HkSal6 {
                account: ktv(account, "HKSAL", version)?,
                all_accounts,
                max_entries: None,
                touchdown: touchdown?,
            },
            number,
        ),
        7 => encode(
            &HkSal7 {
                account: kti(account, "HKSAL", version)?,
                all_accounts,
                max_entries: None,
                touchdown: touchdown?,
            },
            number,
        ),
        8 => encode(
            &HkSal8 {
                account: kti(account, "HKSAL", version)?,
                all_accounts,
                max_entries: None,
                touchdown: touchdown?,
            },
            number,
        ),
        _ => Err(unbuildable("HKSAL", version)),
    }
}

/// HKKAZ, transactions as MT940 (Messages C.2.1.1.1):
/// `account+N+from+to+max entries+touchdown`. v5/v6 national, v7 by IBAN.
pub(crate) fn hkkaz(
    number: u16,
    version: u16,
    account: &Account,
    start_date: Date,
    end_date: Date,
    touchdown: Option<&str>,
) -> Result<RawSegment> {
    let (from_date, to_date) = (Some(date(start_date)?), Some(date(end_date)?));
    let all_accounts = Jn(false);
    let touchdown = touchdown_point(touchdown)?;
    macro_rules! hkkaz {
        ($ty:ident, $account:expr) => {
            encode(
                &$ty {
                    account: $account,
                    all_accounts,
                    from_date,
                    to_date,
                    max_entries: None,
                    touchdown,
                },
                number,
            )
        };
    }
    match version {
        5 => hkkaz!(HkKaz5, ktv(account, "HKKAZ", version)?),
        6 => hkkaz!(HkKaz6, ktv(account, "HKKAZ", version)?),
        7 => hkkaz!(HkKaz7, kti(account, "HKKAZ", version)?),
        _ => Err(unbuildable("HKKAZ", version)),
    }
}

/// HKCAZ v1, transactions as camt.052 (Messages C.2.3.1.1.1):
/// `account+camt format+N+from+to+max entries+touchdown`. `camt_format` is
/// the camt.052 schema URN the bank lists in HICAZS.
pub(crate) fn hkcaz(
    number: u16,
    version: u16,
    account: &Account,
    camt_format: &str,
    start_date: Date,
    end_date: Date,
    touchdown: Option<&str>,
) -> Result<RawSegment> {
    if version != 1 {
        return Err(unbuildable("HKCAZ", version));
    }
    encode(
        &HkCaz1 {
            account: kti(account, "HKCAZ", version)?,
            camt_messages: SupportedCamtMessages1 {
                descriptors: vec![verbatim::<An>(camt_format, "camt format")?],
            },
            all_accounts: Jn(false),
            from_date: Some(date(start_date)?),
            to_date: Some(date(end_date)?),
            max_entries: None,
            touchdown: touchdown_point(touchdown)?,
        },
        number,
    )
}

/// HKWPD, holdings as MT535 (Messages C.4.3.1):
/// `depot+currency+price quality+max entries+touchdown`. The depot is
/// addressed nationally in both versions.
pub(crate) fn hkwpd(
    number: u16,
    version: u16,
    account: &Account,
    currency: Option<&str>,
    touchdown: Option<&str>,
) -> Result<RawSegment> {
    let currency = currency
        .filter(|c| !c.is_empty())
        .map(|c| verbatim::<Cur>(c, "currency"))
        .transpose()?;
    let touchdown = touchdown_point(touchdown)?;
    match version {
        5 => encode(
            &HkWpd5 {
                depot: ktv(account, "HKWPD", version)?,
                currency,
                quality: None,
                max_entries: None,
                touchdown,
            },
            number,
        ),
        6 => encode(
            &HkWpd6 {
                depot: ktv(account, "HKWPD", version)?,
                currency,
                quality: None,
                max_entries: None,
                touchdown,
            },
            number,
        ),
        _ => Err(unbuildable("HKWPD", version)),
    }
}

/// HKWDU v5, depot transactions as MT536 (Messages C.4.3.2):
/// `depot+N+security+from+to+max entries+touchdown`, all securities.
pub(crate) fn hkwdu(
    number: u16,
    version: u16,
    account: &Account,
    start_date: Option<Date>,
    end_date: Option<Date>,
    touchdown: Option<&str>,
) -> Result<RawSegment> {
    if version != 5 {
        return Err(unbuildable("HKWDU", version));
    }
    encode(
        &HkWdu5 {
            depot: ktv(account, "HKWDU", version)?,
            all_depots: Jn(false),
            security_reference: None,
            start_date: start_date.map(date).transpose()?,
            end_date: end_date.map(date).transpose()?,
            max_entries: None,
            touchdown: touchdown_point(touchdown)?,
        },
        number,
    )
}

// ── Payments (payments design section 6) ───────────────────────────────

/// HKCCS v1: SEPA credit transfer (Messages C.10.2.1). `descriptor` is the
/// pain.001 descriptor as the bank announced it, `pain` the message bytes.
pub(crate) fn hkccs(
    number: u16,
    version: u16,
    account: &Account,
    descriptor: &str,
    pain: &[u8],
) -> Result<RawSegment> {
    match version {
        1 => encode(
            &HkCcs1 {
                account: kti(account, "HKCCS", version)?,
                descriptor: verbatim::<An>(descriptor, "SEPA descriptor")?,
                message: Bin(pain.to_vec()),
            },
            number,
        ),
        _ => Err(unbuildable("HKCCS", version)),
    }
}

/// HKIPZ v1 or v2: SEPA instant credit transfer (Messages C.10.2.9.1).
/// `convert` is v2's "Umwandlung nach SEPA-Überweisung zulässig", sent only
/// when `Some` (the BPD allows it); v1 has no such element.
pub(crate) fn hkipz(
    number: u16,
    version: u16,
    account: &Account,
    descriptor: &str,
    pain: &[u8],
    convert: Option<bool>,
) -> Result<RawSegment> {
    let account_kti = kti(account, "HKIPZ", version)?;
    let descriptor = verbatim::<An>(descriptor, "SEPA descriptor")?;
    let message = Bin(pain.to_vec());
    match version {
        1 => encode(
            &HkIpz1 {
                account: account_kti,
                descriptor,
                message,
            },
            number,
        ),
        2 => encode(
            &HkIpz2 {
                account: account_kti,
                descriptor,
                message,
                convert_to_sepa_allowed: convert.map(Jn),
            },
            number,
        ),
        _ => Err(unbuildable("HKIPZ", version)),
    }
}

/// HKIPS v1: status of an instant payment (Messages C.10.2.9.2).
/// `descriptors` are the pain messages the client understands in the answer.
pub(crate) fn hkips(
    number: u16,
    version: u16,
    account: &Account,
    descriptors: &[String],
    order_id: &str,
) -> Result<RawSegment> {
    match version {
        1 => encode(
            &HkIps1 {
                account: kti(account, "HKIPS", version)?,
                formats: SupportedPainMessages1 {
                    descriptors: descriptors
                        .iter()
                        .map(|d| verbatim::<An>(d, "SEPA descriptor"))
                        .collect::<Result<_>>()?,
                },
                order_id: verbatim::<An>(order_id, "order ID")?,
            },
            number,
        ),
        _ => Err(unbuildable("HKIPS", version)),
    }
}

/// HKVPP v1: payee check (VoP C.10.7.1.1). Message 1 carries only the
/// pain.002 descriptors; a poll also the polling ID and touchdown point of
/// the last answer. "Maximale Anzahl Einträge" stays empty.
pub(crate) fn hkvpp(
    number: u16,
    version: u16,
    descriptors: &[String],
    polling_id: Option<&[u8]>,
    touchdown: Option<&str>,
) -> Result<RawSegment> {
    match version {
        1 => encode(
            &HkVpp1 {
                reports: SupportedPaymentStatusReports1 {
                    descriptors: descriptors
                        .iter()
                        .map(|d| verbatim::<An>(d, "pain.002 descriptor"))
                        .collect::<Result<_>>()?,
                },
                polling_id: polling_id.map(|p| Bin(p.to_vec())),
                max_entries: None,
                touchdown: touchdown
                    .map(|t| verbatim::<An>(t, "touchdown point"))
                    .transpose()?,
            },
            number,
        ),
        _ => Err(unbuildable("HKVPP", version)),
    }
}

/// HKVPA v1: release after the payee check (VoP C.10.7.1.2).
pub(crate) fn hkvpa(number: u16, version: u16, vop_id: &[u8]) -> Result<RawSegment> {
    match version {
        1 => encode(
            &HkVpa1 {
                vop_id: Bin(vop_id.to_vec()),
            },
            number,
        ),
        _ => Err(unbuildable("HKVPA", version)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::to_wire;
    use crate::types::Blz;

    fn wire_of<T: crate::codec::Segment>(v: &T, number: u16) -> String {
        String::from_utf8(to_wire(v, number, None).unwrap()).unwrap()
    }

    fn wire(seg: RawSegment) -> String {
        String::from_utf8(crate::codec::wire::serialize_segment(&seg).unwrap()).unwrap()
    }

    #[test]
    fn hkidn_wire_format() {
        assert_eq!(
            wire_of(&hkidn("12030000", "testuser", "0").unwrap(), 3),
            "HKIDN:3:2+280:12030000+testuser+0+1'"
        );
    }

    #[test]
    fn hkvvb_wire_format() {
        assert_eq!(
            wire_of(&hkvvb(0, 0, "Testprodukt").unwrap(), 4),
            "HKVVB:4:3+0+0+1+Testprodukt+1.0'"
        );
        assert_eq!(
            wire_of(&hkvvb(78, 3, "P").unwrap(), 4),
            "HKVVB:4:3+78+3+1+P+1.0'"
        );
    }

    #[test]
    fn hksyn_wire_format() {
        assert_eq!(wire_of(&hksyn(), 5), "HKSYN:5:3+0'");
    }

    #[test]
    fn hkend_wire_format() {
        assert_eq!(
            wire_of(&hkend("DLG1234567").unwrap(), 5),
            "HKEND:5:1+DLG1234567'"
        );
    }

    // ── Accounts (synthetic) ───────────────────────────────────────────

    /// A German giro account; its IBAN holds bank code 12345678 and
    /// account number 4711.
    const GIRO_IBAN: &str = "DE00123456780000004711";
    const GIRO_BIC: &str = "TESTDEFFXXX";
    /// The KTV derived from that IBAN.
    const GIRO_KTV: &str = "4711::280:12345678";
    /// A non-German account: no KTV can be derived from its IBAN.
    const FOREIGN_IBAN: &str = "NL00TEST0123456789";
    const FOREIGN_BIC: &str = "TESTNL2AXXX";

    fn giro() -> Account {
        Account::new(GIRO_IBAN, GIRO_BIC).unwrap()
    }

    fn foreign() -> Account {
        Account::new(FOREIGN_IBAN, FOREIGN_BIC).unwrap()
    }

    /// A depot the UPD lists by number only.
    fn depot() -> Account {
        Account::national("900100", None, &Blz::new("12345678"), GIRO_BIC).unwrap()
    }

    fn day(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    /// One account element on its own, as a one-group segment body.
    fn element_wire<E: crate::codec::Element>(element: &E) -> String {
        let mut bytes = Vec::new();
        element
            .encode(&mut bytes, &crate::codec::Rules::default())
            .unwrap();
        wire(RawSegment {
            degs: vec![crate::codec::DEG(bytes)],
        })
    }

    fn is_not_addressable(result: Result<RawSegment>, expected: AddressingReason) -> bool {
        matches!(
            result,
            Err(FinTSError::AccountNotAddressable { reason, .. }) if reason == expected
        )
    }

    #[test]
    fn kti_is_iban_and_bic_unless_national_parts_are_allowed() {
        let kti_of = |account: &Account| element_wire(&kti(account, "HKSAL", 7).unwrap());
        assert_eq!(kti_of(&giro()), format!("{GIRO_IBAN}:{GIRO_BIC}'"));
        assert_eq!(
            kti_of(&giro().with_kti_national(true)),
            format!("{GIRO_IBAN}:{GIRO_BIC}:{GIRO_KTV}'")
        );
        // Nothing national to add for a foreign account.
        assert_eq!(
            kti_of(&foreign().with_kti_national(true)),
            format!("{FOREIGN_IBAN}:{FOREIGN_BIC}'")
        );
    }

    #[test]
    fn ktv_needs_national_details() {
        assert_eq!(
            element_wire(&ktv(&giro(), "HKSAL", 5).unwrap()),
            format!("{GIRO_KTV}'")
        );
        let refused = ktv(&foreign(), "HKKAZ", 6).unwrap_err();
        assert!(
            matches!(
                &refused,
                FinTSError::AccountNotAddressable {
                    code,
                    version: 6,
                    reason: AddressingReason::NoNationalDetails,
                } if code.as_str() == "HKKAZ"
            ),
            "{refused}"
        );
    }

    /// A depot known by number only is addressed by KTV; an order whose
    /// version takes a KTI refuses it.
    #[test]
    fn national_only_account() {
        assert_eq!(
            wire(hkwpd(7, 6, &depot(), None, None).unwrap()),
            "HKWPD:7:6+900100::280:12345678'"
        );
        let (from, to) = (day(2026, 2, 1), day(2026, 2, 28));
        assert!(hksal(7, 7, &depot(), None).is_err());
        assert!(hkcaz(7, 1, &depot(), CAMT052_DEFAULT, from, to, None).is_err());
    }

    /// An account without IBAN whose bank allows national details (HISPAS)
    /// is sent as `::number:subaccount:280:bank code` (Messages B.3.2).
    #[test]
    fn kti_without_iban_carries_the_national_parts() {
        let card =
            Account::national("5100000000000009", Some("01"), &Blz::new("12345678"), "").unwrap();
        let national = "::5100000000000009:01:280:12345678";
        let allowed = card.clone().with_kti_national(true);
        assert_eq!(
            wire(hksal(4, 7, &allowed, None).unwrap()),
            format!("HKSAL:4:7+{national}+N'")
        );
        let camt = hkcaz(
            4,
            1,
            &allowed,
            CAMT052_DEFAULT,
            day(2026, 2, 1),
            day(2026, 2, 28),
            None,
        );
        assert!(wire(camt.unwrap()).starts_with(&format!("HKCAZ:4:1+{national}+urn")));
        assert!(is_not_addressable(
            hksal(4, 7, &card, None),
            AddressingReason::NoIbanBic
        ));
    }

    // ── HKSAL (Saldo5–8) ───────────────────────────────────────────────

    /// Versions 5 and 6 address the account by KTV, 7 and 8 by KTI; DE 3
    /// asks for one account (`N`), DE 5 is the touchdown point.
    #[test]
    fn hksal_versions() {
        let kti = format!("{GIRO_IBAN}:{GIRO_BIC}");
        for (version, account) in [(5, GIRO_KTV), (6, GIRO_KTV), (7, &kti), (8, &kti)] {
            assert_eq!(
                wire(hksal(9, version, &giro(), None).unwrap()),
                format!("HKSAL:9:{version}+{account}+N'")
            );
        }
        assert_eq!(
            wire(hksal(9, 8, &giro(), Some("AB:12")).unwrap()),
            format!("HKSAL:9:8+{kti}+N++AB?:12'")
        );
        for unknown in [4, 9] {
            assert!(hksal(9, unknown, &giro(), None).is_err(), "{unknown}");
        }
    }

    // ── HKKAZ (KUmsZeit5–7) ────────────────────────────────────────────

    #[test]
    fn hkkaz_versions() {
        let (from, to) = (day(2025, 12, 1), day(2026, 1, 31));
        let kti = format!("{GIRO_IBAN}:{GIRO_BIC}");
        for (version, account) in [(5, GIRO_KTV), (6, GIRO_KTV), (7, &kti)] {
            assert_eq!(
                wire(hkkaz(9, version, &giro(), from, to, None).unwrap()),
                format!("HKKAZ:9:{version}+{account}+N+20251201+20260131'")
            );
        }
        // DE 6 (maximum entries) stays empty before the touchdown point.
        assert_eq!(
            wire(hkkaz(9, 7, &giro(), from, to, Some("P2")).unwrap()),
            format!("HKKAZ:9:7+{kti}+N+20251201+20260131++P2'")
        );
        assert!(hkkaz(9, 8, &giro(), from, to, None).is_err());
    }

    // ── HKCAZ (KUmsZeitCamt1) ──────────────────────────────────────────

    #[test]
    fn hkcaz_version_1() {
        let (from, to) = (day(2026, 2, 1), day(2026, 2, 28));
        let request = |touchdown| hkcaz(9, 1, &giro(), CAMT052_DEFAULT, from, to, touchdown);
        assert_eq!(
            wire(request(None).unwrap()),
            format!(
                "HKCAZ:9:1+{GIRO_IBAN}:{GIRO_BIC}+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+N+20260201+20260228'"
            )
        );
        assert!(wire(request(Some("P2")).unwrap()).ends_with("+N+20260201+20260228++P2'"));
        assert!(hkcaz(9, 2, &giro(), CAMT052_DEFAULT, from, to, None).is_err());
    }

    // ── HKWDU (Depotumsätze 5) ─────────────────────────────────────────

    #[test]
    fn hkwdu_version_5() {
        let (from, to) = (Some(day(2026, 1, 1)), Some(day(2026, 1, 31)));
        assert_eq!(
            wire(hkwdu(9, 5, &depot(), from, to, None).unwrap()),
            "HKWDU:9:5+900100::280:12345678+N++20260101+20260131'"
        );
        assert_eq!(
            wire(hkwdu(9, 5, &depot(), None, None, None).unwrap()),
            "HKWDU:9:5+900100::280:12345678+N'"
        );
        // Currency, both dates and the maximum stay empty before it.
        assert_eq!(
            wire(hkwdu(9, 5, &depot(), None, None, Some("P2")).unwrap()),
            "HKWDU:9:5+900100::280:12345678+N+++++P2'"
        );
        assert!(hkwdu(9, 6, &depot(), None, None, None).is_err());
    }

    // ── HKWPD (WPDepotList5/6) ─────────────────────────────────────────

    #[test]
    fn hkwpd_versions() {
        for version in [5, 6] {
            assert_eq!(
                wire(hkwpd(9, version, &giro(), None, None).unwrap()),
                format!("HKWPD:9:{version}+{GIRO_KTV}'")
            );
        }
        assert_eq!(
            wire(hkwpd(9, 6, &depot(), Some("USD"), None).unwrap()),
            "HKWPD:9:6+900100::280:12345678+USD'"
        );
        // Currency, price quality and maximum stay empty before it.
        assert_eq!(
            wire(hkwpd(9, 6, &depot(), None, Some("P2")).unwrap()),
            "HKWPD:9:6+900100::280:12345678++++P2'"
        );
        assert!(hkwpd(9, 7, &depot(), None, None).is_err());
    }

    fn tan(version: u16, request: TanRequest<'_>, medium: Option<&str>, number: u16) -> String {
        let raw = hktan(version, request, medium, number).unwrap();
        String::from_utf8(crate::codec::wire::serialize_segment(&raw).unwrap()).unwrap()
    }

    // ── HKTAN (TAN2Step6/7) ────────────────────────────────────────────

    #[test]
    fn hktan_v7_process4_wire_format() {
        assert_eq!(
            tan(7, TanRequest::Start { segment: "HKIDN" }, None, 4),
            "HKTAN:4:7+4+HKIDN'"
        );
    }

    #[test]
    fn hktan_v7_process4_with_medium() {
        assert_eq!(
            tan(7, TanRequest::Start { segment: "HKKAZ" }, Some("iPhone"), 4),
            "HKTAN:4:7+4+HKKAZ+++++++++iPhone'"
        );
    }

    /// Deliberate (not tried at a bank, docs/banks/unverified.md): HKTAN version 6 knows the
    /// processes 1-4 (PIN/TAN B.5.1); process `S` (decoupled status) is
    /// version 7 only (B.5.2). A decoupled confirmation with HKTAN 6 is
    /// polled with process 2 and the order reference. Not
    /// `HKTAN:3:6+S++++REF-1+N'`.
    #[test]
    fn hktan_v6_decoupled_poll_uses_process_2_wire_format() {
        assert_eq!(
            tan(
                6,
                TanRequest::DecoupledStatus {
                    task_reference: "REF-1"
                },
                None,
                3
            ),
            "HKTAN:3:6+2++++REF-1+N'"
        );
    }

    /// Deliberate (not tried at a bank, docs/banks/unverified.md): same rule as for version 6.
    /// PIN/TAN B.5.2: process `S` exists only from HKTAN 7 (common client
    /// behaviour agrees), so HKTAN 5 polls a decoupled
    /// confirmation with process 2 and the order reference. Not
    /// `HKTAN:3:5+S++++REF-1++N'`. HKTAN 5 keeps the segment
    /// code with process 4 (deliberate; deviates from
    /// PIN/TAN E.1.5 a).
    #[test]
    fn hktan_v5_decoupled_poll_uses_process_2_wire_format() {
        assert_eq!(
            tan(
                5,
                TanRequest::DecoupledStatus {
                    task_reference: "REF-1"
                },
                None,
                3
            ),
            "HKTAN:3:5+2++++REF-1++N'"
        );
    }

    #[test]
    fn hktan_v7_decoupled_poll_uses_process_s() {
        assert_eq!(
            tan(
                7,
                TanRequest::DecoupledStatus {
                    task_reference: "TASK-42"
                },
                None,
                3
            ),
            "HKTAN:3:7+S++++TASK-42+N'"
        );
    }

    #[test]
    fn hktan_v7_process2_wire_format() {
        assert_eq!(
            tan(
                7,
                TanRequest::Submit {
                    task_reference: "REF-1"
                },
                None,
                3
            ),
            "HKTAN:3:7+2++++REF-1+N'"
        );
    }

    /// HKTAN 6, process 2 (PIN/TAN B.4.2, B.5.1): the order reference of
    /// the challenge in DE 5, "Weitere TAN folgt" N; the TAN itself goes
    /// into HNSHA. The same segment confirms the login and, since the jobs
    /// step core, any order.
    #[test]
    fn hktan_v6_process2_wire_format() {
        assert_eq!(
            tan(
                6,
                TanRequest::Submit {
                    task_reference: "TASK0001"
                },
                None,
                3
            ),
            "HKTAN:3:6+2++++TASK0001+N'"
        );
    }

    #[test]
    fn hktan_rejects_v4() {
        assert!(hktan(4, TanRequest::Start { segment: "HKIDN" }, None, 3).is_err());
    }

    #[test]
    fn caller_values_are_not_trimmed() {
        assert_eq!(
            wire_of(&hkidn("12030000", " user ", "0").unwrap(), 3),
            "HKIDN:3:2+280:12030000+ user +0+1'"
        );
    }
}
