//! FinTS data types and Data Element Group definitions.
//!
//! Provides typed representations of common FinTS structures
//! that appear across multiple segments.

use jiff::civil::Date;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::parser::RawSegment;

// ── Text types ──────────────────────────────────────────────────────────────
//
// Each identifier of the protocol gets its own type, so that a bank code
// cannot end up where a user ID belongs.

/// Declares a text type with `new`, `as_str`, `Display` and `AsRef<str>`.
macro_rules! text_type {
    ($(#[$doc:meta])* $ty:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $ty(String);

        impl $ty {
            /// Wraps the text as given.
            pub fn new(text: impl Into<String>) -> Self {
                Self(text.into())
            }

            /// The wrapped text.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self.0.as_str(), out)
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

text_type!(
    /// Bank code (Bankleitzahl), e.g. "12030000".
    Blz
);
text_type!(
    /// FinTS user ID / login name.
    UserId
);
text_type!(
    /// System ID assigned by the bank via HKSYN.
    SystemId
);
text_type!(
    /// FinTS product registration ID.
    ProductId
);
text_type!(
    /// Dialog ID assigned by the bank in HNHBK.
    DialogId
);
text_type!(
    /// Security function code, e.g. "940" for pushTAN, "999" for PIN-only.
    SecurityFunction
);
text_type!(
    /// Task reference from HITAN, used in HKTAN process 2/S.
    TaskReference
);
text_type!(
    /// Segment type identifier, e.g. "HKSAL", "HKKAZ".
    SegmentType
);
text_type!(
    /// TAN medium name (e.g. device name for pushTAN).
    TanMediumName
);
text_type!(
    /// Touchdown/pagination point returned by the bank (code 3040).
    TouchdownPoint
);
text_type!(
    /// Segment type reference for HKTAN process 4 (e.g. "HKIDN", "HKSAL").
    SegmentRef
);
text_type!(
    /// Currency code (ISO 4217), e.g. "EUR".
    Currency
);

impl Currency {
    /// ISO 4217 minor-unit exponent: 0 for JPY and similar, 3 for BHD, KWD,
    /// TND, JOD, OMR, IQD, LYD, and 2 for every other code, unknown ones
    /// included.
    pub fn exponent(&self) -> u32 {
        match self.0.to_ascii_uppercase().as_str() {
            "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF"
            | "UGX" | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
            "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
            _ => 2,
        }
    }

    /// `amount` in minor units (EUR 12.34 is 1234). `None` when the amount
    /// has more decimals than [`exponent`](Self::exponent) allows (it is
    /// never rounded) or does not fit an `i64`.
    pub fn to_minor(&self, amount: Decimal) -> Option<i64> {
        let amount = amount.normalize();
        let exponent = self.exponent();
        if amount.scale() > exponent {
            return None;
        }
        let minor = amount.checked_mul(Decimal::from(10_i64.pow(exponent)))?;
        minor.to_i64()
    }
}
text_type!(
    /// IBAN (International Bank Account Number).
    Iban
);
text_type!(
    /// BIC (Bank Identifier Code).
    Bic
);
text_type!(
    /// Human-readable bank name.
    BankName
);
text_type!(
    /// FinTS server URL.
    FinTSUrl
);
text_type!(
    /// TAN challenge text to display to the user.
    ChallengeText
);

/// The HHD-UC challenge bytes of an optical TAN method (HITAN
/// "Challenge HHD_UC"), as the bank sent them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HhdUcData(pub Vec<u8>);

/// HHD-UC challenge data sorted into something a user interface can show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TanMedia {
    /// A picture for the TAN app or reader to scan (photoTAN, QR-TAN),
    /// with its media type, typically `image/png`.
    Image { mime_type: String, data: Vec<u8> },
    /// A chipTAN flicker code (HHD 1.4), unchanged; turning it into the
    /// animation is up to the caller.
    Flicker(String),
}

impl HhdUcData {
    /// What the challenge data is.
    ///
    /// A matrix code is a sequence of two length-prefixed fields, each
    /// prefix being two bytes big-endian: first the media type, then the
    /// picture. Data that does not parse that way but is all printable
    /// ASCII is taken as a flicker code. Everything else is `None`.
    pub fn decode(&self) -> Option<TanMedia> {
        let bytes = &self.0[..];
        matrix_code(bytes).or_else(|| flicker_code(bytes))
    }
}

/// Splits the field after a two-byte big-endian length off `input`.
fn length_prefixed(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let (len, rest) = input.split_first_chunk::<2>()?;
    let len = usize::from(u16::from_be_bytes(*len));
    (rest.len() >= len).then(|| rest.split_at(len))
}

fn matrix_code(bytes: &[u8]) -> Option<TanMedia> {
    let (media_type, rest) = length_prefixed(bytes)?;
    let plausible = matches!(media_type.len(), 3..=100)
        && media_type.iter().all(u8::is_ascii_graphic)
        && media_type.contains(&b'/');
    if !plausible {
        return None;
    }
    let (picture, _) = length_prefixed(rest)?;
    Some(TanMedia::Image {
        mime_type: String::from_utf8_lossy(media_type).into_owned(),
        data: picture.to_vec(),
    })
}

fn flicker_code(bytes: &[u8]) -> Option<TanMedia> {
    if bytes.is_empty() || bytes.iter().any(|b| !(b' '..=b'~').contains(b)) {
        return None;
    }
    // Printable ASCII is valid UTF-8.
    let text = std::str::from_utf8(bytes).ok()?;
    Some(TanMedia::Flicker(text.trim().to_owned()))
}

/// A statement in SWIFT MT940/MT942 format, still as bytes (Windows-1252).
#[derive(Debug, Clone, Default)]
pub struct Mt940Data(pub Vec<u8>);

impl Mt940Data {
    /// No data yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes collected so far.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    /// True before the first page arrived (or when all pages were empty).
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    /// Append one more page of a statement fetched with touchdown points.
    /// Pages are concatenated unchanged: a bank may cut the stream anywhere,
    /// even in the middle of a field.
    pub fn extend(&mut self, page: Vec<u8>) {
        self.0.extend_from_slice(&page);
    }
}

/// The user's PIN. `Debug` and `Display` never show it.
#[derive(Clone)]
pub struct Pin(String);

impl Pin {
    pub fn new(pin: impl Into<String>) -> Self {
        Self(pin.into())
    }

    /// The PIN itself, for the signature head only.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

const HIDDEN: &str = "****";

impl fmt::Debug for Pin {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "Pin({HIDDEN})")
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(HIDDEN)
    }
}

// ── Values with a meaning in the protocol ───────────────────────────────────

/// The placeholder Formals uses for an identifier the bank has not given
/// out yet (system ID before synchronisation, dialog ID before the first
/// answer).
const NOT_YET_ASSIGNED: &str = "0";

fn assigned(id: &str) -> bool {
    !matches!(id, "" | NOT_YET_ASSIGNED)
}

impl SystemId {
    /// The value sent before HKSYN gave the customer system an ID.
    pub fn unassigned() -> Self {
        Self::new(NOT_YET_ASSIGNED)
    }

    /// Whether the bank has given out this ID.
    pub fn is_assigned(&self) -> bool {
        assigned(&self.0)
    }
}

impl DialogId {
    /// The value of the first message of a dialog.
    pub fn unassigned() -> Self {
        Self::new(NOT_YET_ASSIGNED)
    }

    /// Whether the bank has opened the dialog under this ID.
    pub fn is_assigned(&self) -> bool {
        assigned(&self.0)
    }
}

impl SecurityFunction {
    /// Security function 999: the one-step procedure, PIN without TAN.
    pub fn pin_only() -> Self {
        Self::new("999")
    }
}

/// "TAN-Prozess" of a two-step method (PIN/TAN document, HITANS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TanProcess {
    /// Process 1: the TAN goes into the same message as the order.
    OneStep,
    /// Process 2 (also 3 and 4 of older versions): order first, TAN in a
    /// later message.
    TwoStep,
}

impl TanProcess {
    /// From the HITANS code: `1` is one message, anything else two.
    pub fn from_str_val(code: &str) -> Self {
        if code == "1" {
            Self::OneStep
        } else {
            Self::TwoStep
        }
    }
}

// ── Reading single elements ─────────────────────────────────────────────────

/// Element `de` of group `deg` as text, `""` when absent.
pub(crate) fn read_str(seg: &RawSegment, deg: usize, de: usize) -> String {
    seg.deg(deg).get_str(de)
}

/// Element `de` of group `deg` as text, `None` when absent or empty.
pub(crate) fn read_opt_str(seg: &RawSegment, deg: usize, de: usize) -> Option<String> {
    Some(read_str(seg, deg, de)).filter(|text| !text.is_empty())
}

// ── What the library hands to callers ───────────────────────────────────────

/// One TAN method the bank offers (an entry of HITANS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TanMethod {
    /// Security function code (e.g. "912" for pushTAN)
    pub security_function: SecurityFunction,
    /// TAN process type.
    pub tan_process: TanProcess,
    /// Human-readable name (e.g. "pushTAN-2.0")
    pub name: String,
    /// Whether TAN medium name must be sent
    pub needs_tan_medium: bool,
    /// Decoupled methods: how many status requests (HKTAN process S) the
    /// bank accepts. HITANS uses `0` for no limit; `-1` here means the bank
    /// did not say.
    pub decoupled_max_polls: i32,
    /// Decoupled methods: seconds before the first status request; `0`
    /// when the bank did not say.
    pub wait_before_first_poll: i32,
    /// Decoupled methods: seconds between status requests; `0` when the
    /// bank did not say.
    pub wait_before_next_poll: i32,
    /// Whether this is a decoupled method
    pub is_decoupled: bool,
    /// The HKTAN version to use: that of the HITANS segment listing the
    /// method.
    pub hktan_version: u16,
    /// The method's DK identifier ("DK-TAN-Verfahren", earlier "ZKA"),
    /// such as `HHD` or `Decoupled`.
    pub zka_name: Option<String>,
    /// HITANS 7 "Automatischer Statusabruf erlaubt". `Some(false)`: the
    /// user confirms in the client before it asks for the status.
    pub decoupled_auto_polling: Option<bool>,
    /// HITANS "Challenge strukturiert": the HITAN challenge text of this
    /// method may carry markup.
    pub challenge_structured: bool,
}

/// The identity of an account of the UPD, stable across logins: its KTV
/// (Kontoverbindung, HIUPD DE 1), or its IBAN when the UPD entry has no
/// account number.
///
/// Normalised: the BLZ is the KTV's own (not the login's), leading zeros
/// of the number are dropped (`0` stays `0`), an empty subaccount is no
/// subaccount, the IBAN is upper case without spaces. Two UPD entries that
/// share IBAN and number but differ in the subaccount (a clearing account
/// and its depot) have different keys.
///
/// The string form ([`Display`](fmt::Display), [`FromStr`](std::str::FromStr),
/// also the serde form) is `<blz>/<number>[/<subaccount>]` or
/// `iban:<IBAN>[/<subaccount>]`; `%` and `/` inside the BLZ, number and
/// IBAN are written `%25` and `%2F`. It round-trips and suits an app's
/// reference to a synced account.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccountKey {
    /// The KTV: bank code, account number, subaccount.
    National {
        blz: Blz,
        number: String,
        subaccount: Option<String>,
    },
    /// A UPD entry without account number (or without bank code): IBAN
    /// and subaccount.
    Iban {
        iban: Iban,
        subaccount: Option<String>,
    },
}

impl AccountKey {
    /// The key of a KTV. `None` without a number or bank code.
    pub fn national(blz: &str, number: &str, subaccount: Option<&str>) -> Option<Self> {
        let (blz, number) = (blz.trim(), number.trim());
        if blz.is_empty() || number.is_empty() {
            return None;
        }
        let stripped = number.trim_start_matches('0');
        Some(Self::National {
            blz: Blz::new(blz),
            number: if stripped.is_empty() { "0" } else { stripped }.to_string(),
            subaccount: normal_subaccount(subaccount),
        })
    }

    /// The key of an account known by IBAN only. `None` for an empty IBAN.
    pub fn iban(iban: &str, subaccount: Option<&str>) -> Option<Self> {
        let iban: String = iban
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase();
        (!iban.is_empty()).then(|| Self::Iban {
            iban: Iban::new(iban),
            subaccount: normal_subaccount(subaccount),
        })
    }

    /// The subaccount (Unterkontomerkmal), if any.
    pub fn subaccount(&self) -> Option<&str> {
        match self {
            Self::National { subaccount, .. } | Self::Iban { subaccount, .. } => {
                subaccount.as_deref()
            }
        }
    }
}

fn normal_subaccount(subaccount: Option<&str>) -> Option<String> {
    subaccount
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

const KEY_IBAN_PREFIX: &str = "iban:";

fn key_escape(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F")
}

fn key_unescape(s: &str) -> String {
    s.replace("%2F", "/")
        .replace("%2f", "/")
        .replace("%25", "%")
}

impl fmt::Display for AccountKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let subaccount = match self {
            Self::National { blz, number, .. } => {
                write!(f, "{}/{}", key_escape(blz.as_str()), key_escape(number))?;
                self.subaccount()
            }
            Self::Iban { iban, .. } => {
                write!(f, "{KEY_IBAN_PREFIX}{}", key_escape(iban.as_str()))?;
                self.subaccount()
            }
        };
        match subaccount {
            Some(s) => write!(f, "/{s}"),
            None => Ok(()),
        }
    }
}

/// Why a string is not an [`AccountKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountKeyError(String);

impl fmt::Display for AccountKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not an account key: {:?}", self.0)
    }
}

impl std::error::Error for AccountKeyError {}

impl std::str::FromStr for AccountKey {
    type Err = AccountKeyError;

    /// Reads the form [`Display`](fmt::Display) writes; the subaccount is
    /// the rest after the second (IBAN form: first) `/`.
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let err = || AccountKeyError(s.to_string());
        let key = if let Some(rest) = s.strip_prefix(KEY_IBAN_PREFIX) {
            let (iban, sub) = match rest.split_once('/') {
                Some((iban, sub)) => (iban, Some(sub)),
                None => (rest, None),
            };
            AccountKey::iban(&key_unescape(iban), sub)
        } else {
            let mut parts = s.splitn(3, '/');
            let blz = parts.next().ok_or_else(err)?;
            let number = parts.next().ok_or_else(err)?;
            AccountKey::national(&key_unescape(blz), &key_unescape(number), parts.next())
        };
        // Only the normal form is accepted, so that one account has one string.
        key.filter(|k| k.to_string() == s).ok_or_else(err)
    }
}

impl Serialize for AccountKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AccountKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Kontoart first, else a depot by the allowed orders; see
/// [`AccountInfo::class`](crate::jobs::AccountInfo::class).
pub(crate) fn classify(
    account_type: Option<u8>,
    allows: impl Fn(&str) -> bool,
) -> Option<(AccountClass, ClassSource)> {
    if let Some(class) = account_type.and_then(AccountClass::from_account_type) {
        return Some((class, ClassSource::Kontoart));
    }
    if allows("HKWPD") && !allows("HKSAL") && !allows("HKKAZ") && !allows("HKCAZ") {
        return Some((AccountClass::Depot, ClassSource::AllowedJobs));
    }
    None
}

/// One HIUPD account as the remainder parser and the trace recorder read
/// it. The jobs use [`crate::protocol::UpdAccount`] and
/// [`crate::jobs::AccountInfo`].
#[derive(Debug, Clone)]
pub(crate) struct SepaAccount {
    pub iban: Iban,
    pub bic: Bic,
    pub account_number: String,
    pub sub_account: String,
    pub blz: Blz,
    pub owner: Option<String>,
    pub product_name: Option<String>,
    pub currency: Option<Currency>,
    /// "Kontoart" from the UPD (HIUPD v6), 1-99. See [`classify`].
    pub account_type: Option<u8>,
    /// "Erlaubte Geschäftsvorfälle" of the UPD: the order codes (HKSAL, HKKAZ,
    /// HKWPD, …) the user may send for this account. Empty when the UPD lists
    /// none. Read by the trace recorder only.
    #[cfg_attr(not(any(test, feature = "trace")), allow(dead_code))]
    pub allowed_jobs: Vec<String>,
}

impl SepaAccount {
    /// See [`classify`].
    #[cfg(any(test, feature = "trace"))]
    pub(crate) fn classify(&self) -> Option<(AccountClass, ClassSource)> {
        classify(self.account_type, |job| {
            self.allowed_jobs.iter().any(|j| j == job)
        })
    }
}

/// Where an [`AccountClass`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassSource {
    /// The Kontoart of the UPD.
    Kontoart,
    /// No Kontoart; deduced from the orders the UPD allows.
    AllowedJobs,
}

/// What kind of account a UPD entry is, from its "Kontoart" (Formals, data
/// dictionary). The spec assigns each tens range one kind; the values
/// inside a range are the bank's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountClass {
    /// Kontoart 1 to 9, current account (Kontokorrent-/Girokonto).
    Checking,
    /// Kontoart 10 to 19, savings account.
    Savings,
    /// Kontoart 20 to 29, time deposit (Festgeld).
    FixedDeposit,
    /// Kontoart 30 to 39, securities account.
    Depot,
    /// Kontoart 40 to 49, loan.
    Loan,
    /// Kontoart 50 to 59, credit card.
    CreditCard,
    /// Kontoart 60 to 69, fund account held at an investment company.
    FundDepot,
    /// Kontoart 70 to 79, building savings contract (Bausparen).
    HomeSavings,
    /// Kontoart 80 to 89, insurance contract.
    Insurance,
    /// Kontoart 90 to 99, anything else.
    Other,
}

impl AccountClass {
    /// Kind by tens range, in order of the ranges 1-9, 10-19, ... 90-99.
    const BY_TENS: [Self; 10] = [
        Self::Checking,
        Self::Savings,
        Self::FixedDeposit,
        Self::Depot,
        Self::Loan,
        Self::CreditCard,
        Self::FundDepot,
        Self::HomeSavings,
        Self::Insurance,
        Self::Other,
    ];

    /// The kind for a Kontoart; `None` outside 1 to 99.
    pub fn from_account_type(account_type: u8) -> Option<Self> {
        if !(1..=99).contains(&account_type) {
            return None;
        }
        Some(Self::BY_TENS[usize::from(account_type / 10)])
    }
}

/// Account balance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBalance {
    pub amount: Decimal,
    /// Date of the booked balance; `None` if the bank left it out.
    pub date: Option<Date>,
    /// The balance's currency; the account currency if the bank left it out.
    pub currency: Currency,
    pub credit_line: Option<Decimal>,
    pub available: Option<Decimal>,
    /// Pending (vorgemerkter) balance amount, if provided by the bank.
    pub pending_amount: Option<Decimal>,
    /// Date of the pending balance, if provided.
    pub pending_date: Option<Date>,
}

/// A time the bank stamped on its data: the bank's wall clock, and its UTC
/// offset when the bank sent one. FinTS carries no time zones (Formals
/// `tim` is the sender's local time); SWIFT `:13D:` adds a fixed offset,
/// the older `:13:` (HBCI 2.2 IX.2.9) has none. Not serde (jiff's
/// `Offset` is not); like [`jobs::Statement`](crate::jobs::Statement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankTime {
    /// Date and time as the bank wrote them.
    pub datetime: jiff::civil::DateTime,
    /// The offset from UTC the bank gave; `None` when it gave none.
    pub offset: Option<jiff::tz::Offset>,
}

impl BankTime {
    /// The instant, when the bank gave an offset. Without one the wall
    /// clock is the bank's (German banks: Europe/Berlin), which the library
    /// does not guess.
    pub fn timestamp(&self) -> Option<jiff::Timestamp> {
        self.offset.and_then(|o| o.to_timestamp(self.datetime).ok())
    }
}

/// Whether a transaction is booked or pending (vorgemerkt).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStatus {
    Booked,
    Pending,
}

// ── Securities / Depot types ────────────────────────────────────────────────

text_type!(
    /// ISIN (International Securities Identification Number), e.g. "DE0005140008".
    Isin
);
text_type!(
    /// WKN (Wertpapierkennnummer), e.g. "514000".
    Wkn
);

/// A single securities position in a depot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityHolding {
    /// ISIN of the security.
    pub isin: Option<Isin>,
    /// WKN (German securities identification number).
    pub wkn: Option<Wkn>,
    /// Human-readable name of the security.
    pub name: String,
    /// Number of shares/units held (can be fractional for funds).
    pub quantity: Decimal,
    /// Current market price per unit.
    pub price: Option<Decimal>,
    /// Currency of the price.
    pub price_currency: Option<Currency>,
    /// Date of the price quote.
    pub price_date: Option<Date>,
    /// Total market value (quantity * price).
    pub market_value: Option<Decimal>,
    /// Currency of the market value.
    pub market_value_currency: Option<Currency>,
    /// Original purchase value (Einstandswert), if available.
    pub acquisition_value: Option<Decimal>,
    /// Purchase (acquisition) date, from the German structured MT535
    /// `:70E::HOLD` narrative.
    pub acquisition_date: Option<Date>,
    /// Purchase price per unit (percent of face amount for bonds).
    pub acquisition_price: Option<Decimal>,
    /// Currency of the purchase price; `None` for percent prices.
    pub acquisition_price_currency: Option<Currency>,
    /// Profit/loss amount, if available.
    pub profit_loss: Option<Decimal>,
    /// Exchange/market where the price was quoted.
    pub exchange: Option<String>,
    /// Depot number this holding belongs to.
    pub depot_id: Option<String>,
    /// Date the statement holding this position was made for (MT535
    /// `:98A::STAT` or `:98C::STAT` in the general block), if the bank sent one.
    pub statement_date: Option<Date>,
    /// Raw parsed data for debugging/extension.
    pub raw: serde_json::Value,
}

/// Whether securities came into or left the depot (MT536 `:22H::REDE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DepotTransactionDirection {
    /// `RECE`: received (purchase, inbound transfer).
    Receive,
    /// `DELI`: delivered (sale, outbound transfer).
    Deliver,
    /// Not given or not recognised.
    Unknown,
}

/// One depot transaction (securities movement) from an MT536 statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepotTransaction {
    pub isin: Option<Isin>,
    pub wkn: Option<Wkn>,
    /// Description of the security.
    pub name: String,
    /// Posted quantity: pieces, or face amount for bonds (see `raw["quantity_type"]`).
    pub quantity: Decimal,
    /// Posted amount, signed as the bank sends it (`N` = negative).
    pub amount: Option<Decimal>,
    pub currency: Option<Currency>,
    pub direction: DepotTransactionDirection,
    /// Transaction type indicator (`:22F::TRAN`), e.g. `SETT`, `CORP`.
    pub transaction_type: Option<String>,
    /// Effective settlement date (`ESET`).
    pub effective_date: Option<Date>,
    /// Trade date (`TRAD`).
    pub trade_date: Option<Date>,
    /// Settlement date (`SETT`).
    pub settlement_date: Option<Date>,
    /// Deal price per unit (percent of face amount when no currency).
    pub price: Option<Decimal>,
    pub price_currency: Option<Currency>,
    /// Reversal (Storno) of an earlier transaction.
    pub is_reversal: bool,
    /// Free text from `:70E:`, lines joined with `\n`.
    pub details: Option<String>,
    /// Depot number from the statement header.
    pub depot_id: Option<String>,
    /// Raw parsed data for debugging/extension.
    pub raw: serde_json::Value,
}

/// A parsed transaction.
///
/// A camt entry (`Ntry`) with several `TxDtls` (a batch booking) maps to
/// one `Transaction`, built from the first `TxDtls`: the counterparty and
/// purpose fields describe only that part of the batch. The amount is the
/// entry's, and `booking_id` is the entry's own reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    /// Booking date (Buchungstag): the MT940 entry date, or the value date
    /// when the bank leaves the entry date out. A pending CAMT entry with
    /// neither date takes the day the bank made the report (`CreDtTm`).
    pub date: Date,
    /// Value date (Valuta, Wertstellung).
    pub valuta_date: Option<Date>,
    /// Signed from the account holder's view: negative = debit.
    pub amount: Decimal,
    pub currency: Currency,
    pub applicant_name: Option<String>,
    pub applicant_iban: Option<Iban>,
    pub applicant_bic: Option<Bic>,
    /// Remittance text: the SEPA `SVWZ+` value of MT940 `:86:`, or CAMT
    /// `Ustrd` (`None` when the entry has none; its `AddtlNtryInf` is the
    /// `posting_text`). In MT940 without SEPA keywords, the whole `:86:` text. The keywords themselves and the
    /// other SEPA fields have their own fields; MT940 keeps the joined
    /// `:86:` text in `raw["tag86"]`. Characters are as the bank sent them.
    pub purpose: Option<String>,
    /// Booking text (Buchungstext), e.g. "SEPA-BASISLASTSCHRIFT".
    pub posting_text: Option<String>,
    /// Customer reference (Kundenreferenz) from MT940 `:61:`; `None` for
    /// "NONREF". CAMT has no such field: always `None` there (the end-to-end
    /// ID is `end_to_end_id`).
    pub customer_reference: Option<String>,
    /// Bank reference from `:61:` (after `//`), or CAMT `AcctSvcrRef`
    /// (transaction level before entry level), if the bank sends one. Not
    /// known to be unique: for de-duplication use `booking_id`.
    pub bank_reference: Option<String>,
    /// The bank's own ID of this booking, set only where the bank's profile
    /// ([`BookingIdSource`](crate::banks::BookingIdSource)) names a reference
    /// the bank keeps unique and stable: no two transactions of a statement
    /// share it, and the bank does not reuse it for another booking. Safe
    /// as a key for de-duplication across fetches.
    ///
    /// `None` for banks without such a reference, for every MT940 booking,
    /// for pending bookings (whether a booking keeps its reference from
    /// pending to booked is unverified at any bank), for an entry without
    /// the reference, and for a reference that occurs more than once in
    /// one statement.
    pub booking_id: Option<String>,
    /// The DK business transaction code (Geschäftsvorfallcode, three
    /// digits) at the start of `:86:`; `166` is a SEPA credit transfer
    /// received.
    pub gvc: Option<String>,
    /// The bank's Primanota number, subfield `?10` of `:86:`.
    pub prima_nota: Option<String>,
    /// ISO 20022 purpose code (e.g. `SALA`, `IDCP`): `PURP+` or `Purp/Cd`.
    pub purpose_code: Option<String>,
    /// SEPA direct debit mandate reference (`MREF+`, `MndtId`). `None` for
    /// a placeholder such as `NONREF`.
    pub mandate_id: Option<String>,
    /// SEPA end-to-end reference (`EREF+`, `EndToEndId`). `None` for a
    /// placeholder such as `NOTPROVIDED`.
    pub end_to_end_id: Option<String>,
    /// SEPA creditor ID of a direct debit (`CRED+` of MT940 `:86:`, CAMT
    /// `Cdtr/Id/PrvtId/Othr/Id`). `None` for a placeholder.
    pub creditor_id: Option<String>,
    /// SEPA identifier of the debtor (`DEBT+` of MT940 `:86:`, CAMT
    /// `Dbtr/Id`): the originator ID of a transfer or the debtor's own ID.
    /// `None` for a placeholder.
    pub debtor_id: Option<String>,
    /// The ultimate debtor (Abweichender Zahlungspflichtiger) when it
    /// differs from the direct one: CAMT `UltmtDbtr/Nm`; MT940 `ABWA+` of a
    /// transfer, `ABWE+` of a direct debit. For an R-transaction (return,
    /// reversal) the original payment's roles.
    pub ultimate_debtor: Option<String>,
    /// The ultimate creditor (Abweichender Zahlungsempfänger): CAMT
    /// `UltmtCdtr/Nm`; MT940 `ABWE+` of a transfer, `ABWA+` of a direct
    /// debit.
    pub ultimate_creditor: Option<String>,
    /// Whether the booking is a reversal (Storno): CAMT `RvslInd` true,
    /// MT940 `:61:` mark `RC` or `RD`.
    pub reversal: bool,
    /// Original amount before conversion or before a return, with its
    /// currency (always positive): CAMT `AmtDtls/InstdAmt/Amt`; MT940 `OAMT+`,
    /// which carries no currency, so the statement currency is used.
    pub original_amount: Option<(Decimal, Currency)>,
    /// Fees or compensation charged on the booking, positive: CAMT `Chrgs`
    /// (`TtlChrgsAndTaxAmt`, else the first `Rcrd/Amt`); MT940 `COAM+` in the
    /// statement currency.
    pub fees: Option<(Decimal, Currency)>,
    /// Return or rejection reason code (e.g. `AC04`, `MD06`): CAMT
    /// `RtrInf/Rsn/Cd`; MT940 the code after the `REJECT`/`RETURN` marker in
    /// `SVWZ+`.
    pub return_reason: Option<String>,
    /// Direct debit sequence type (`FRST`, `RCUR`, `OOFF`, `FNAL`): CAMT
    /// `PmtTpInf/SeqTp`. MT940 has no such field.
    pub sequence_type: Option<String>,
    pub raw: serde_json::Value,
    /// Whether this transaction is booked or pending.
    pub status: TransactionStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_to_minor() {
        let d = |s: &str| s.parse::<Decimal>().unwrap();
        let eur = Currency::new("EUR");
        assert_eq!(eur.to_minor(d("12.34")), Some(1234));
        assert_eq!(eur.to_minor(d("12.3400")), Some(1234));
        assert_eq!(eur.to_minor(d("12.345")), None);
        assert_eq!(eur.to_minor(d("-0.5")), Some(-50));
        assert_eq!(Currency::new("JPY").to_minor(d("100")), Some(100));
        assert_eq!(Currency::new("JPY").to_minor(d("1.5")), None);
        assert_eq!(Currency::new("KWD").to_minor(d("1.234")), Some(1234));
        assert_eq!(Currency::new("XYZ").to_minor(d("1.23")), Some(123));
        assert_eq!(eur.to_minor(Decimal::MAX), None);
        assert_eq!(eur.to_minor(d("92233720368547758.07")), Some(i64::MAX));
        assert_eq!(eur.to_minor(d("92233720368547758.08")), None);
    }

    /// Two-byte big-endian length, then the bytes.
    fn field(bytes: &[u8]) -> Vec<u8> {
        let mut out = u16::try_from(bytes.len()).unwrap().to_be_bytes().to_vec();
        out.extend_from_slice(bytes);
        out
    }

    #[test]
    fn matrix_code_gives_the_picture() {
        let picture = b"\x89PNG-synthetic";
        let challenge = [field(b"image/png"), field(picture)].concat();
        let expected = TanMedia::Image {
            mime_type: "image/png".to_owned(),
            data: picture.to_vec(),
        };
        assert_eq!(HhdUcData(challenge).decode(), Some(expected));
    }

    #[test]
    fn printable_challenge_is_a_flicker_code() {
        let code = "0248A0120452019980812345678";
        let decoded = HhdUcData(format!(" {code} ").into_bytes()).decode();
        assert_eq!(decoded, Some(TanMedia::Flicker(code.to_owned())));
    }

    #[test]
    fn short_or_unprintable_challenge_is_none() {
        let mut cut_off = [field(b"image/png"), field(&[7; 40])].concat();
        cut_off.truncate(cut_off.len() - 1);
        for challenge in [cut_off, Vec::new(), vec![0x00, 0x01, 0xff]] {
            assert_eq!(HhdUcData(challenge).decode(), None);
        }
    }

    #[test]
    fn kontoart_ranges() {
        let class = AccountClass::from_account_type;
        assert_eq!(class(1), Some(AccountClass::Checking));
        assert_eq!(class(9), Some(AccountClass::Checking));
        assert_eq!(class(10), Some(AccountClass::Savings));
        assert_eq!(class(35), Some(AccountClass::Depot));
        assert_eq!(class(99), Some(AccountClass::Other));
        assert_eq!(class(0), None);
        assert_eq!(class(100), None);
    }

    fn class(kontoart: Option<u8>, jobs: &[&str]) -> Option<(AccountClass, ClassSource)> {
        classify(kontoart, |job| jobs.contains(&job))
    }

    #[test]
    fn kontoart_wins_over_allowed_jobs() {
        assert_eq!(
            class(Some(1), &["HKWPD"]),
            Some((AccountClass::Checking, ClassSource::Kontoart))
        );
        assert_eq!(
            class(Some(30), &["HKSAL"]),
            Some((AccountClass::Depot, ClassSource::Kontoart))
        );
    }

    #[test]
    fn no_kontoart_hkwpd_only_is_depot() {
        assert_eq!(
            class(None, &["HKWPD", "HKPRO"]),
            Some((AccountClass::Depot, ClassSource::AllowedJobs))
        );
    }

    #[test]
    fn no_kontoart_hkwpd_with_account_jobs_is_not_depot() {
        for other in ["HKSAL", "HKKAZ", "HKCAZ"] {
            assert_eq!(class(None, &["HKWPD", other]), None, "{other}");
        }
    }

    #[test]
    fn nothing_known_is_unclassified() {
        assert_eq!(class(None, &[]), None);
        assert_eq!(class(None, &["HKSAL"]), None);
        // Out of range counts as no Kontoart.
        assert_eq!(class(Some(0), &[]), None);
        assert_eq!(
            class(Some(0), &["HKWPD"]),
            Some((AccountClass::Depot, ClassSource::AllowedJobs))
        );
    }

    #[test]
    fn account_key_is_normalised() {
        let k = AccountKey::national(" 12345678", "0000000301", Some("")).unwrap();
        assert_eq!(
            k,
            AccountKey::National {
                blz: Blz::new("12345678"),
                number: "301".into(),
                subaccount: None,
            }
        );
        assert_eq!(k, AccountKey::national("12345678", "301", None).unwrap());
        assert_eq!(
            AccountKey::national("12345678", "000", Some(" 01 ")).unwrap(),
            AccountKey::National {
                blz: Blz::new("12345678"),
                number: "0".into(),
                subaccount: Some("01".into()),
            }
        );
        assert_eq!(AccountKey::national("", "301", None), None);
        assert_eq!(AccountKey::national("12345678", " ", None), None);
        assert_eq!(
            AccountKey::iban("de00 1234 5678 0000 0003 01", Some("")).unwrap(),
            AccountKey::Iban {
                iban: Iban::new("DE00123456780000000301"),
                subaccount: None,
            }
        );
        assert_eq!(AccountKey::iban("", None), None);
        // Same IBAN and number, another subaccount: another account.
        assert_ne!(
            k,
            AccountKey::national("12345678", "301", Some("01")).unwrap()
        );
    }

    #[test]
    fn account_key_string_round_trips() {
        let keys = [
            (
                AccountKey::national("12345678", "0301", None),
                "12345678/301",
            ),
            (
                AccountKey::national("12345678", "301", Some("01")),
                "12345678/301/01",
            ),
            (
                AccountKey::national("12345678", "30/1", Some("A/B")),
                "12345678/30%2F1/A/B",
            ),
            (
                AccountKey::iban("DE00123456780000000301", None),
                "iban:DE00123456780000000301",
            ),
            (
                AccountKey::iban("DE00123456780000000301", Some("TOKEN/1")),
                "iban:DE00123456780000000301/TOKEN/1",
            ),
        ];
        for (key, text) in keys {
            let key = key.unwrap();
            assert_eq!(key.to_string(), text);
            assert_eq!(text.parse::<AccountKey>().unwrap(), key, "{text}");
            let json = serde_json::to_string(&key).unwrap();
            assert_eq!(json, format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<AccountKey>(&json).unwrap(), key);
        }
        for bad in [
            "",
            "12345678",
            "12345678/",
            "/301",
            "12345678/0301",
            "12345678/301/",
            "iban:",
            "iban:de00",
        ] {
            assert!(bad.parse::<AccountKey>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn account_classes() {
        assert_eq!(
            AccountClass::from_account_type(1),
            Some(AccountClass::Checking)
        );
        assert_eq!(
            AccountClass::from_account_type(10),
            Some(AccountClass::Savings)
        );
        assert_eq!(
            AccountClass::from_account_type(39),
            Some(AccountClass::Depot)
        );
        assert_eq!(
            AccountClass::from_account_type(55),
            Some(AccountClass::CreditCard)
        );
        assert_eq!(AccountClass::from_account_type(0), None);
        assert_eq!(
            AccountClass::from_account_type(9),
            Some(AccountClass::Checking)
        );
        assert_eq!(
            AccountClass::from_account_type(20),
            Some(AccountClass::FixedDeposit)
        );
        assert_eq!(
            AccountClass::from_account_type(49),
            Some(AccountClass::Loan)
        );
        assert_eq!(
            AccountClass::from_account_type(60),
            Some(AccountClass::FundDepot)
        );
        assert_eq!(
            AccountClass::from_account_type(79),
            Some(AccountClass::HomeSavings)
        );
        assert_eq!(
            AccountClass::from_account_type(89),
            Some(AccountClass::Insurance)
        );
        assert_eq!(
            AccountClass::from_account_type(99),
            Some(AccountClass::Other)
        );
        assert_eq!(AccountClass::from_account_type(100), None);
    }
}
