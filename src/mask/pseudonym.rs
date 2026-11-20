//! Stand-ins for identity values in traces: per-trace aliases for account
//! identifiers, keyed pseudonyms for references and IDs.
//!
//! Both give equal values equal stand-ins, so a trace still shows which
//! values are the same while the values are gone. The user's own account
//! identifiers and the IDs of the dialog keep their length and character
//! classes, because requests and answers must still fit together. What
//! comes from other people does not: a counterparty's IBAN, a creditor ID
//! and every booking and payment reference are written in one fixed form
//! each, because the length and pattern of such a value names its issuer
//! (a payment service's references are recognisable by their shape). They
//! differ in what links them:
//!
//! - **Aliases** for account identifiers: IBAN, account number, user and
//!   customer ID, creditor ID. These have little entropy (a German IBAN is
//!   the bank code, which is in clear, plus ten digits), so no function of
//!   the value may be written: anyone holding the key could try every
//!   value. An alias is a number: the values of a kind are numbered in the
//!   order they first appear in the trace, from a random start drawn per
//!   [`Identities`] (per recorder), and the number is written in the
//!   value's character classes. It has no mathematical link to the value.
//!   The table from value to number lives only in memory and is never
//!   written; two recorders, even with the same [`Salt`], give the same
//!   value unrelated aliases.
//! - **Pseudonyms** (every other class: booking reference, end-to-end ID,
//!   message and payment information ID, mandate reference, instant order
//!   ID, VOP-ID, polling ID, system ID). These are keyed by the [`Salt`],
//!   so traces recorded with the same salt give a value the same
//!   pseudonym.
//!
//! # Aliases
//!
//! Three kinds are numbered apart: accounts (IBAN and account number
//! share one table), users (user ID and customer ID), creditors. The alias
//! of number `n` writes `n` from the right over the value's ASCII letters
//! and digits, a digit in base 10 and a letter in base 26 (`A`/`a` is 0,
//! case kept); every other character stays. Formats:
//!
//! - Account number ([`IdClass::Account`]): leading zeros stay; the rest is
//!   the key of the table, and its first character, if a digit, is not
//!   `0` (the zeros keep their count). So `0001234567` (KTV) and `1234567` (MT940 `:25:`) of the same
//!   account get the same digits after the zeros.
//! - IBAN ([`IdClass::Iban`]): country and bank part of the BBAN stay (the
//!   bank code is in clear in the KTI anyway; [`bank_part_len`]), the
//!   account part gets the account number's alias, and the check digits
//!   are computed again (ISO 13616 mod 97): the alias is a valid IBAN, and
//!   its account part matches the alias of the same account's KTV number.
//!   A value that is not of the IBAN form gets an account number's alias.
//! - IBAN of someone else ([`IdClass::OtherIban`]): always `DE`, check
//!   digits, the bank code `00000000` and ten digits of the alias, whatever
//!   country, bank and length the value had.
//! - Creditor ID ([`IdClass::CreditorId`]): always `DE`, check digits,
//!   `ZZZ` and eleven digits of the alias (mod 97 over the national
//!   identifier and the country, the business code left out).
//!
//! # Pseudonyms
//!
//! For a value `v` of identity class `c` (the tag of [`IdClass`]):
//!
//! 1. `k = HMAC-SHA256(salt, c ‖ 0x00 ‖ v)`;
//! 2. a key stream `HMAC-SHA256(k, 0) ‖ HMAC-SHA256(k, 1) ‖ …` (counter as
//!    four bytes, big-endian);
//! 3. each ASCII digit of `v` becomes the next stream byte below 250,
//!    modulo 10; each upper-case letter the next byte below 234, modulo 26,
//!    as an upper-case letter; lower case likewise. Every other character
//!    stays. A booking or payment reference (booking reference, end-to-end
//!    ID, message and payment information ID, mandate reference) is instead
//!    twelve characters of `0-9A-Z` from the stream, whatever it looked
//!    like; the constants `NOTPROVIDED` and `NONREF` stay.
//!
//! Every character depends on the whole value, so the pseudonym of a
//! prefix says nothing about the pseudonym of the whole value. With the
//! same salt the pseudonyms are the same in every trace; with another salt
//! they are unrelated. Two different values with the same pattern collide
//! with probability one in the number of values of that pattern.
//!
//! - [`IdClass::SystemId`]: `0` (no system ID yet) stays.
//! - Binary data that is not printable ASCII (a VOP-ID or polling ID made
//!   of raw bytes): every byte becomes a stream byte.

use std::collections::HashMap;
use std::sync::Mutex;

use hmac_sha256::HMAC;

/// The secret key of the pseudonyms: 32 bytes. Never written into a trace.
/// Account identifiers do not depend on it (they get aliases).
#[derive(Clone, PartialEq, Eq)]
pub struct Salt([u8; 32]);

/// 32 bytes from the operating system's random number generator.
fn random_bytes() -> [u8; 32] {
    use rand::TryRngCore as _;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("the operating system's random number generator works");
    bytes
}

impl Salt {
    /// A new salt from the operating system's random number generator.
    pub fn random() -> Salt {
        Salt(random_bytes())
    }

    /// The salt with these bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Salt {
        Salt(bytes)
    }

    /// The salt as 64 lower-case hex digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Reads 64 hex digits; surrounding white space is ignored.
    pub fn from_hex(text: &str) -> Result<Salt, String> {
        let text = text.trim();
        if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("a salt is 64 hex digits".into());
        }
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16)
                .map_err(|_| "a salt is 64 hex digits".to_string())?;
        }
        Ok(Salt(bytes))
    }
}

impl std::fmt::Debug for Salt {
    /// Never shows the bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Salt(…)")
    }
}

/// What an identity value is. Equal values of different classes get
/// different stand-ins; the classes are chosen so that the same thing
/// gets the same class wherever it occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdClass {
    /// IBAN of one of the user's accounts (KTI, KTZ, HIUPD, a statement's
    /// own account, the debtor of a pain.001). Alias of the account part;
    /// the bank code stays, as it does in the KTI.
    Iban,
    /// IBAN of anyone else (counterparty of a booking, payee). Alias of the
    /// whole BBAN: a counterparty's bank code would say where the user's
    /// other accounts are.
    OtherIban,
    /// National account or depot number (KTV, KTI, KTZ, MT940 `:25:`,
    /// `:86:` `?31`, MT535 `:97A:`, camt `Othr/Id` of an account). Alias.
    Account,
    /// User ID and customer ID (HKIDN, HNSHK, HNVSK, HIUPA, HIUPD). Alias.
    UserId,
    /// Creditor ID: camt and pain `CdtrSchmeId`, MT940 `CRED+`. Alias.
    CreditorId,
    /// Customer system ID (HKIDN, HNSHK, HNVSK, HISYN).
    SystemId,
    /// The bank's booking reference: camt `AcctSvcrRef`, MT940 `:61:`
    /// bank reference.
    BookingRef,
    /// End-to-end ID: pain and camt `EndToEndId`, `OrgnlEndToEndId`,
    /// MT940 `EREF+` and `:61:` customer reference.
    EndToEnd,
    /// pain `MsgId`, `OrgnlMsgId` (and a camt's own `MsgId`).
    MessageId,
    /// pain `PmtInfId`, `OrgnlPmtInfId`.
    PaymentInfoId,
    /// Mandate reference: `MndtId`, MT940 `MREF+`.
    Mandate,
    /// Order ID of an instant payment (HKIPS, HIIPS, HIIPZ).
    OrderId,
    /// VOP-ID (HIVPP, HKVPA).
    VopId,
    /// Polling ID (HIVPP, HKVPP).
    PollingId,
}

/// The alias tables: one per kind of account identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Account = 0,
    User = 1,
    Creditor = 2,
}

impl IdClass {
    fn tag(self) -> &'static str {
        match self {
            IdClass::Iban => "iban",
            IdClass::OtherIban => "other-iban",
            IdClass::Account => "account",
            IdClass::UserId => "user-id",
            IdClass::CreditorId => "creditor-id",
            IdClass::SystemId => "system-id",
            IdClass::BookingRef => "booking-ref",
            IdClass::EndToEnd => "end-to-end-id",
            IdClass::MessageId => "message-id",
            IdClass::PaymentInfoId => "payment-info-id",
            IdClass::Mandate => "mandate-ref",
            IdClass::OrderId => "order-id",
            IdClass::VopId => "vop-id",
            IdClass::PollingId => "polling-id",
        }
    }

    /// The alias table of an account identifier, which gets a per-trace
    /// alias and never a pseudonym; `None` for a class that gets a
    /// pseudonym.
    fn kind(self) -> Option<Kind> {
        match self {
            IdClass::Iban | IdClass::OtherIban | IdClass::Account => Some(Kind::Account),
            IdClass::UserId => Some(Kind::User),
            IdClass::CreditorId => Some(Kind::Creditor),
            _ => None,
        }
    }
}

/// The value-to-number tables of the aliases. Held in memory only.
#[derive(Default)]
struct Table {
    numbers: HashMap<(Kind, Vec<u8>), u64>,
    count: [u64; 3],
}

/// Makes the stand-ins of one trace: aliases for account identifiers,
/// pseudonyms keyed by one [`Salt`] for the rest.
pub(crate) struct Identities {
    salt: Salt,
    /// How many days every date of the trace is moved (`mask::dates`).
    days: i32,
    /// The random start of each kind's numbering.
    start: [u64; 3],
    table: Mutex<Table>,
}

impl std::fmt::Debug for Identities {
    /// Never shows the salt or the alias table.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Identities(…)")
    }
}

/// The key stream of one value.
struct Stream {
    key: [u8; 32],
    counter: u32,
    block: [u8; 32],
    at: usize,
}

impl Stream {
    fn new(key: [u8; 32]) -> Stream {
        let mut s = Stream {
            key,
            counter: 0,
            block: [0; 32],
            at: 32,
        };
        s.refill();
        s
    }

    fn refill(&mut self) {
        self.block = HMAC::mac(self.counter.to_be_bytes(), self.key);
        self.counter = self.counter.wrapping_add(1);
        self.at = 0;
    }

    fn byte(&mut self) -> u8 {
        if self.at == 32 {
            self.refill();
        }
        let b = self.block[self.at];
        self.at += 1;
        b
    }

    /// A uniform number below `n` (rejection sampling).
    fn below(&mut self, n: u8) -> u8 {
        let limit = 256 - (256 % u16::from(n));
        loop {
            let b = self.byte();
            if u16::from(b) < limit {
                return b % n;
            }
        }
    }
}

impl Identities {
    /// Pseudonyms keyed by `salt`; aliases numbered from a fresh random
    /// start.
    pub(crate) fn new(salt: Salt) -> Identities {
        let r = random_bytes();
        let start = std::array::from_fn(|k| {
            u64::from_be_bytes(r[8 * k..8 * k + 8].try_into().expect("8 bytes"))
        });
        Identities {
            salt,
            days: 0,
            start,
            table: Mutex::default(),
        }
    }

    /// These identities with every date moved by `days`.
    pub(crate) fn with_days(mut self, days: i32) -> Identities {
        self.days = days;
        self
    }

    /// How many days a date of the trace is moved.
    pub(crate) fn days(&self) -> i32 {
        self.days
    }

    /// A number of days to move the dates of one trace by: 3 to 30 days
    /// back, drawn from the operating system's random number generator.
    pub(crate) fn random_days() -> i32 {
        -(3 + i32::from(random_bytes()[0] % 28))
    }

    fn stream(&self, class: IdClass, value: &[u8]) -> Stream {
        let mut mac = HMAC::new(self.salt.0);
        mac.update(class.tag());
        mac.update([0u8]);
        mac.update(value);
        Stream::new(mac.finalize())
    }

    /// ASCII letters and digits of `value` mapped with `stream`, every
    /// other byte kept.
    fn map(stream: &mut Stream, value: &[u8]) -> Vec<u8> {
        value
            .iter()
            .map(|&c| match c {
                b'0'..=b'9' => b'0' + stream.below(10),
                b'A'..=b'Z' => b'A' + stream.below(26),
                b'a'..=b'z' => b'a' + stream.below(26),
                other => other,
            })
            .collect()
    }

    /// The stand-in of `value` (bytes of text or binary data). The same
    /// length and character classes, except for the classes with one fixed
    /// form (a counterparty's IBAN, a creditor ID, a reference).
    pub(crate) fn bytes(&self, class: IdClass, value: &[u8]) -> Vec<u8> {
        if value.is_empty() {
            return Vec::new();
        }
        match class {
            IdClass::Iban if is_iban(value) => self.iban_alias(value),
            IdClass::OtherIban if is_iban(value) => {
                let mut out = b"DE0000000000".to_vec();
                out.extend(self.alias(Kind::Account, &value[4..], b"0000000000", false));
                set_check_digits(&mut out, 4);
                out
            }
            IdClass::Iban | IdClass::OtherIban | IdClass::Account => self.account_alias(value),
            IdClass::UserId => self.alias(Kind::User, value, value, false),
            IdClass::CreditorId if is_creditor_id(value) => {
                let mut out = b"DE00ZZZ".to_vec();
                out.extend(self.alias(Kind::Creditor, &value[7..], b"00000000000", false));
                set_check_digits(&mut out, 7);
                out
            }
            IdClass::BookingRef
            | IdClass::EndToEnd
            | IdClass::MessageId
            | IdClass::PaymentInfoId
            | IdClass::Mandate => {
                if matches!(value, b"NOTPROVIDED" | b"NONREF") {
                    return value.to_vec();
                }
                let mut stream = self.stream(class, value);
                (0..REFERENCE_LEN)
                    .map(|_| match stream.below(36) {
                        n @ 0..=9 => b'0' + n,
                        n => b'A' + n - 10,
                    })
                    .collect()
            }
            IdClass::CreditorId => self.alias(Kind::Creditor, value, value, false),
            IdClass::SystemId if value == b"0" => value.to_vec(),
            IdClass::VopId | IdClass::PollingId
                if !value.iter().all(|b| (0x20..0x7f).contains(b)) =>
            {
                let mut stream = self.stream(class, value);
                value.iter().map(|_| stream.byte()).collect()
            }
            _ => {
                let mut stream = self.stream(class, value);
                Self::map(&mut stream, value)
            }
        }
    }

    /// The stand-in of a text value: ASCII letters and digits are
    /// replaced, every other character stays.
    pub(crate) fn text(&self, class: IdClass, value: &str) -> String {
        let reference = matches!(
            class,
            IdClass::BookingRef
                | IdClass::EndToEnd
                | IdClass::MessageId
                | IdClass::PaymentInfoId
                | IdClass::Mandate
        );
        if value.is_ascii() || reference {
            return self
                .bytes(class, value.as_bytes())
                .into_iter()
                .map(char::from)
                .collect();
        }
        // Replace the ASCII characters only; the derivation and the alias
        // table see the whole value.
        if let Some(kind) = class.kind() {
            // A non-ASCII character is neither letter nor digit to `fill`.
            let pattern: Vec<u8> = value
                .chars()
                .map(|c| if c.is_ascii() { c as u8 } else { 0x80 })
                .collect();
            let out = self.alias(kind, value.as_bytes(), &pattern, false);
            return value
                .chars()
                .zip(out)
                .map(|(c, b)| if c.is_ascii() { char::from(b) } else { c })
                .collect();
        }
        let mut stream = self.stream(class, value.as_bytes());
        value
            .chars()
            .map(|c| {
                if c.is_ascii() {
                    char::from(Self::map(&mut stream, &[c as u8])[0])
                } else {
                    c
                }
            })
            .collect()
    }

    /// The number of `key` in the table of `kind` (the next one if `key`
    /// is new), plus the kind's random start.
    fn number(&self, kind: Kind, key: &[u8]) -> u64 {
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        let Table { numbers, count } = &mut *table;
        let n = *numbers.entry((kind, key.to_vec())).or_insert_with(|| {
            count[kind as usize] += 1;
            count[kind as usize]
        });
        self.start[kind as usize].wrapping_add(n)
    }

    /// The alias of `key` in the table of `kind`, written over `pattern`.
    fn alias(&self, kind: Kind, key: &[u8], pattern: &[u8], nonzero_first: bool) -> Vec<u8> {
        fill(pattern, self.number(kind, key), nonzero_first)
    }

    /// An account number: leading zeros stay, the rest is the key.
    fn account_alias(&self, value: &[u8]) -> Vec<u8> {
        let zeros = value.iter().take_while(|&&b| b == b'0').count();
        let rest = &value[zeros..];
        let mut out = value[..zeros].to_vec();
        if !rest.is_empty() {
            out.extend(self.alias(Kind::Account, rest, rest, true));
        }
        out
    }

    /// An IBAN: country and bank part stay, the account part gets the
    /// account number's alias, the check digits are computed again.
    fn iban_alias(&self, iban: &[u8]) -> Vec<u8> {
        let bank = (4 + bank_part_len(&iban[..2])).min(iban.len());
        let mut out = iban[..bank].to_vec();
        out.extend(self.account_alias(&iban[bank..]));
        set_check_digits(&mut out, 4);
        out
    }
}

/// Length of the stand-in of a booking or payment reference. It fits the
/// shortest field a reference is sent in (MT940 `:61:`, 16 characters).
const REFERENCE_LEN: usize = 12;

/// `number` written from the right over the ASCII letters and digits of
/// `pattern` (digits base 10, letters base 26 from `A`/`a`, case kept);
/// every other byte stays. With `nonzero_first` the first letter or digit,
/// if a digit, is `1` to `9`. What does not fit is dropped.
fn fill(pattern: &[u8], number: u64, nonzero_first: bool) -> Vec<u8> {
    let first = pattern.iter().position(u8::is_ascii_alphanumeric);
    let mut rest = number;
    let mut out = pattern.to_vec();
    for (i, c) in out.iter_mut().enumerate().rev() {
        let (base, radix) = match *c {
            b'0'..=b'9' if nonzero_first && Some(i) == first => (b'1', 9),
            b'0'..=b'9' => (b'0', 10),
            b'A'..=b'Z' => (b'A', 26),
            b'a'..=b'z' => (b'a', 26),
            _ => continue,
        };
        *c = base + (rest % radix) as u8;
        rest /= radix;
    }
    out
}

/// The length of the bank part of the BBAN (bank code, and branch code
/// where an account number is per branch), by country (ISO 13616
/// registry). Other countries: 0, the whole BBAN gets the alias.
fn bank_part_len(country: &[u8]) -> usize {
    match country {
        b"DE" | b"ES" | b"PL" => 8,
        b"AT" | b"CH" | b"LI" => 5,
        b"NL" => 4,
        b"BE" | b"LU" => 3,
        b"FR" | b"MC" | b"GB" => 10,
        b"IT" | b"SM" => 11,
        _ => 0,
    }
}

/// Two upper-case letters, two digits, 11 to 30 upper-case letters and
/// digits (ISO 13616: 15 to 34 characters).
pub(crate) fn is_iban(v: &[u8]) -> bool {
    (15..=34).contains(&v.len()) && is_country_check_alnum(v)
}

/// A SEPA creditor ID: two upper-case letters, two digits, a business
/// code of three letters or digits, a national identifier of letters and
/// digits; 8 to 35 characters.
fn is_creditor_id(v: &[u8]) -> bool {
    (8..=35).contains(&v.len()) && is_country_check_alnum(v)
}

fn is_country_check_alnum(v: &[u8]) -> bool {
    v[..2].iter().all(u8::is_ascii_uppercase)
        && v[2..4].iter().all(u8::is_ascii_digit)
        && v[4..]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Sets the check digits (positions 2 and 3) of an IBAN or creditor ID:
/// 98 minus the remainder modulo 97 of the characters from `body` on,
/// then the country and `00`, letters read as 10 to 35 (ISO 13616; the
/// creditor ID leaves its business code, positions 4 to 6, out).
fn set_check_digits(id: &mut [u8], body: usize) {
    let mut rem: u32 = 0;
    for &c in id[body..].iter().chain(&id[..2]).chain(b"00") {
        let v = if c.is_ascii_digit() {
            u32::from(c - b'0')
        } else {
            u32::from(c - b'A') + 10
        };
        rem = if v >= 10 {
            (rem * 100 + v) % 97
        } else {
            (rem * 10 + v) % 97
        };
    }
    let check = 98 - rem;
    id[2] = b'0' + (check / 10) as u8;
    id[3] = b'0' + (check % 10) as u8;
}

#[cfg(test)]
mod tests;
