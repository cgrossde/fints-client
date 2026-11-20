use super::*;
use crate::types::Iban;

fn salt(n: u8) -> Identities {
    Identities::new(Salt::from_bytes([n; 32]))
}

fn classes(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&b| match b {
            b'0'..=b'9' => b'9',
            b'A'..=b'Z' => b'A',
            b'a'..=b'z' => b'a',
            other => other,
        })
        .collect()
}

#[test]
fn same_salt_same_pseudonym_other_salt_other_pseudonym() {
    let value = b"2026-01-02-10.11.12.123456";
    let (a, b) = (salt(1), salt(1));
    assert_eq!(
        a.bytes(IdClass::BookingRef, value),
        b.bytes(IdClass::BookingRef, value)
    );
    assert_ne!(
        a.bytes(IdClass::BookingRef, value),
        salt(2).bytes(IdClass::BookingRef, value)
    );
    assert_ne!(a.bytes(IdClass::BookingRef, value), value.to_vec());
    // The class is part of the derivation.
    assert_ne!(
        a.bytes(IdClass::BookingRef, value),
        a.bytes(IdClass::EndToEnd, value)
    );
}

#[test]
fn length_and_character_classes_are_kept() {
    let p = salt(7);
    // IDs of the dialog keep their form.
    for value in [&b"E2E-4711/abc.DEF 99"[..], b"M-42", b"user0815", b"X", b""] {
        let out = p.bytes(IdClass::OrderId, value);
        assert_eq!(out.len(), value.len());
        assert_eq!(classes(&out), classes(value));
    }
}

#[test]
fn a_reference_has_one_form_whatever_it_looked_like() {
    let p = salt(7);
    for class in [
        IdClass::BookingRef,
        IdClass::EndToEnd,
        IdClass::MessageId,
        IdClass::PaymentInfoId,
        IdClass::Mandate,
    ] {
        for value in [&b"E2E-4711/abc.DEF 99"[..], b"M-42", b"1234567890123", b"X"] {
            let out = p.bytes(class, value);
            assert_eq!(out.len(), 12, "{class:?}");
            assert!(
                out.iter()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
            );
        }
        assert!(p.bytes(class, b"").is_empty());
        // The constants for "no reference" stay.
        assert_eq!(p.bytes(class, b"NOTPROVIDED"), b"NOTPROVIDED");
        assert_eq!(p.bytes(class, b"NONREF"), b"NONREF");
    }
}

#[test]
fn a_counterparty_iban_and_a_creditor_id_have_one_form() {
    let p = salt(7);
    for iban in [
        &b"DE02120300000000202051"[..],
        b"LU120010001234567891",
        b"NL91ABNA0417164300",
    ] {
        let out = p.bytes(IdClass::OtherIban, iban);
        assert_eq!(out.len(), 22);
        assert_eq!(&out[..2], b"DE");
        assert_eq!(&out[4..12], b"00000000");
        assert!(is_iban(&out));
        assert_eq!(out, p.bytes(IdClass::OtherIban, iban));
    }
    for creditor in [&b"DE98ZZZ09999999999"[..], b"LU96ZZZ0000000000123456789"] {
        let out = p.bytes(IdClass::CreditorId, creditor);
        assert_eq!(out.len(), 18);
        assert_eq!(&out[..2], b"DE");
        assert_eq!(&out[4..7], b"ZZZ");
    }
}

#[test]
fn a_prefix_does_not_give_away_the_whole_value() {
    let p = salt(3);
    let short = p.bytes(IdClass::EndToEnd, b"ORDER-2026-0001");
    let long = p.bytes(IdClass::EndToEnd, b"ORDER-2026-00012");
    assert_ne!(&long[..short.len()], &short[..]);
}

#[test]
fn different_values_get_different_pseudonyms() {
    let p = salt(4);
    let all: std::collections::HashSet<Vec<u8>> = (0..2000)
        .map(|i| p.bytes(IdClass::BookingRef, format!("12{i:08}").as_bytes()))
        .collect();
    assert_eq!(all.len(), 2000);
}

#[test]
fn account_identifiers_get_aliases_numbered_in_order_of_appearance() {
    let p = salt(4);
    let all: Vec<Vec<u8>> = (0..2000)
        .map(|i| p.bytes(IdClass::Account, format!("12{i:08}").as_bytes()))
        .collect();
    let distinct: std::collections::HashSet<&Vec<u8>> = all.iter().collect();
    assert_eq!(distinct.len(), 2000);
    // Consecutive numbers: the n-th value is the first plus n - 1.
    let number = |v: &[u8]| std::str::from_utf8(v).unwrap().parse::<u64>().unwrap();
    let first = number(&all[0]);
    for (n, alias) in all.iter().enumerate().take(5) {
        let expected = (first - 1_000_000_000 + n as u64) % 9_000_000_000 + 1_000_000_000;
        assert_eq!(number(alias), expected);
    }
    // The same value, the same alias, wherever it occurs.
    assert_eq!(p.bytes(IdClass::Account, b"1200000007"), all[7]);
    for class in [IdClass::UserId, IdClass::CreditorId, IdClass::Iban] {
        let value = b"DE98ZZZ09999999999";
        assert_eq!(p.bytes(class, value), p.bytes(class, value));
    }
}

#[test]
fn aliases_do_not_depend_on_the_salt() {
    // Two recorders with the same salt: the same references, other
    // aliases for the same account identifiers.
    let (a, b) = (salt(1), salt(1));
    for (class, value) in [
        (IdClass::Iban, "DE89370400440532013000"),
        (IdClass::Account, "0532013000"),
        (IdClass::UserId, "testuser1"),
        (IdClass::CreditorId, "DE98ZZZ09999999999"),
    ] {
        assert_ne!(a.text(class, value), b.text(class, value), "{value}");
    }
    assert_eq!(
        a.text(IdClass::EndToEnd, "E2E-1"),
        b.text(IdClass::EndToEnd, "E2E-1")
    );
}

#[test]
fn an_iban_alias_validates_and_keeps_country_and_bank_part() {
    let p = salt(5);
    for (iban, bank) in [
        ("DE89370400440532013000", 12),
        ("DE02120300000000202051", 12),
        ("AT611904300234573201", 9),
        ("GB29NWBK60161331926819", 14),
        ("NO9386011117947", 4),
    ] {
        let out = p.text(IdClass::Iban, iban);
        assert_eq!(out.len(), iban.len());
        assert_eq!(&out[..2], &iban[..2]);
        assert_eq!(&out[4..bank], &iban[4..bank], "{out} from {iban}");
        assert_ne!(out, iban);
        assert!(Iban::parse(&out).is_ok(), "{out} from {iban}");
        assert_eq!(
            classes(&out.as_bytes()[4..]),
            classes(&iban.as_bytes()[4..])
        );
    }
}

#[test]
fn iban_and_account_number_of_one_account_get_matching_aliases() {
    let p = salt(6);
    let iban = p.text(IdClass::Iban, "DE89370400440532013000");
    let ktv = p.text(IdClass::Account, "0532013000");
    let short = p.text(IdClass::Account, "532013000");
    assert_eq!(&iban[12..], ktv);
    assert_eq!(&ktv[1..], short);
    // Leading zeros stay and keep their count.
    let padded = p.text(IdClass::Account, "0001234567");
    let plain = p.text(IdClass::Account, "1234567");
    assert!(padded.starts_with("000"));
    assert_ne!(&padded[3..4], "0");
    assert_eq!(&padded[3..], plain);
    assert_eq!(p.text(IdClass::Account, "0000"), "0000");
}

#[test]
fn a_creditor_id_alias_keeps_its_format_and_check_digits() {
    let p = salt(11);
    let out = p.text(IdClass::CreditorId, "DE98ZZZ09999999999");
    assert_eq!(out.len(), 18);
    assert_eq!(&out[..2], "DE");
    assert_eq!(&out[4..7], "ZZZ");
    assert_ne!(&out[7..], "09999999999");
    // ISO 7064 mod 97-10 over national ID, country and check digits.
    let check: String = format!("{}{}", &out[7..], &out[..4])
        .chars()
        .map(|c| c.to_digit(36).unwrap().to_string())
        .collect();
    let rem = check
        .bytes()
        .fold(0u32, |r, d| (r * 10 + u32::from(d - b'0')) % 97);
    assert_eq!(rem, 1, "{out}");
}

#[test]
fn a_user_id_alias_keeps_length_and_classes() {
    let p = salt(12);
    for value in ["testuser1", "Max.Muster-42", "1234567890"] {
        let out = p.text(IdClass::UserId, value);
        assert_eq!(classes(out.as_bytes()), classes(value.as_bytes()));
        assert_ne!(out, value);
    }
}

#[test]
fn system_id_zero_stays() {
    let p = salt(8);
    assert_eq!(p.text(IdClass::SystemId, "0"), "0");
    assert_ne!(p.text(IdClass::SystemId, "TESTSYS0001"), "TESTSYS0001");
}

#[test]
fn raw_binary_ids_are_mapped_byte_by_byte() {
    let p = salt(9);
    let raw = [0u8, 1, 2, 0xff, b'-', b'A'];
    let out = p.bytes(IdClass::VopId, &raw);
    assert_eq!(out.len(), raw.len());
    assert_ne!(out, raw);
    assert_eq!(out, p.bytes(IdClass::VopId, &raw));
}

#[test]
fn non_ascii_characters_stay() {
    let p = salt(10);
    let out = p.text(IdClass::OrderId, "MÜ-123");
    assert_eq!(out.chars().count(), 6);
    assert_eq!(out.chars().nth(1), Some('Ü'));
    assert_eq!(out.chars().nth(2), Some('-'));
    assert!(out.chars().next().unwrap().is_ascii_uppercase());
    // A reference with one is written in the fixed form like any other.
    let out = p.text(IdClass::Mandate, "MÜ-123");
    assert!(out.len() == 12 && out.is_ascii(), "{out}");
}

#[test]
fn salt_hex_round_trips_and_debug_hides_it() {
    let s = Salt::random();
    assert_eq!(Salt::from_hex(&format!("{}\n", s.to_hex())).unwrap(), s);
    assert!(Salt::from_hex("abc").is_err());
    assert_eq!(format!("{s:?}"), "Salt(…)");
    assert_ne!(Salt::random(), s);
}
