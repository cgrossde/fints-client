//! The bank as plain data: customer, accounts, TAN methods, offered jobs.
//!
//! Every behaviour a test wants to vary is a field here. The engine reads
//! the model; it never branches on a bank code.

use std::collections::BTreeMap;

use jiff::civil::Date;
use rust_decimal::Decimal;

/// The simulated bank.
#[derive(Debug, Clone)]
pub struct BankModel {
    /// Bankleitzahl, e.g. `12345678`.
    pub blz: String,
    /// BIC of the bank.
    pub bic: String,
    /// Bank name in HIBPA.
    pub name: String,
    /// The one customer who can log in.
    pub customer: Customer,
    /// Accounts in the UPD, in order.
    pub accounts: Vec<Account>,
    /// Two-step TAN methods offered in HITANS and 3920.
    pub tan_methods: Vec<TanMethod>,
    /// HITANS version sent in the BPD: 6 or 7. A decoupled method is only
    /// recognisable from HITANS 7.
    pub hitans_version: u16,
    /// Also send a HITANS 6 with the typed methods next to HITANS 7 (the
    /// BPD of a bank that keeps both versions).
    pub hitans6_too: bool,
    /// Whether the dialog initialisation needs a TAN.
    pub login_sca: LoginSca,
    /// Offered business jobs by segment code (`HKSAL`, `HKKAZ`, `HKWPD`).
    pub jobs: BTreeMap<&'static str, JobSpec>,
    /// BPD version (HIBPA). A client with a lower version gets the BPD.
    pub bpd_version: u32,
    /// UPD version (HIUPA). A client with a lower version gets the UPD.
    /// 0 is dialog-only (Formals): every dialog init gets the UPD, whatever
    /// version the client sends, and the client must not store it.
    pub upd_version: u32,
    /// `false`: a system ID the bank did not issue is answered with 9391.
    pub accept_unknown_system_ids: bool,
    /// `true`: HKKAZ returns only bookings within the requested dates.
    pub filter_by_range: bool,
    /// HISPAS "Nationale Kontoverbindung erlaubt" (`J:J:J` or `J:N:J`);
    /// `None` sends no HISPAS. The client puts the national parts into a
    /// KTI only when this is `Some(true)`.
    pub hispas_national: Option<bool>,
    /// Return code for an order on an account that is not in the UPD or
    /// whose HIUPD does not allow the job. Default `9210`; Consorsbank
    /// answers `9380` (see [`BankModel::not_allowed_text`]).
    pub not_allowed_code: &'static str,
    /// Text of [`BankModel::not_allowed_code`]. Consorsbank: `Keine
    /// Berechtigung für diese Auftragsart mit diesem Konto`.
    pub not_allowed_text: &'static str,
    /// Which the bank checks first on an order: the TAN requirement or the
    /// account's permission for the job. Default [`CheckOrder::TanFirst`].
    pub check_order: CheckOrder,
    /// `true`: an order naming the account by a KTI with only `IBAN:BIC`
    /// (no account number, subaccount and BLZ) is answered `9010
    /// Verarbeitung nicht möglich` (HIRMG 9050), next to `3076` when HKTAN
    /// was sent. Consorsbank, observed live for HKKAZ 7 (trace exchanges 8
    /// and 11); with the full 6-part KTI it answered `0020` (live,
    /// 2026-11-16). Default `false`: `IBAN:BIC` is enough.
    pub kti_requires_national: bool,
    /// HITANS 7 "Maximale Anzahl Statusabfragen" of decoupled methods;
    /// 0 = unlimited (the default).
    pub decoupled_max_polls: u32,
    /// HITANS 7 waits, in seconds, before the first and before each next
    /// status request of decoupled methods. Default 0.
    pub decoupled_wait_secs: u64,
    /// `true`: one host serves several bank codes (comdirect), and an HKIDN
    /// naming a code other than [`BankModel::blz`] gets one HIRMG with
    /// `9050`, `9800`, `9210` and `9340` and the dialog ends. Default
    /// `false`: a foreign code is a [`Violation`](crate::Violation).
    pub foreign_blz_9210: bool,
    /// Payments (HKCCS, HKIPZ, HKIPS), the payee check (HKVPP, HKVPA) and
    /// their parameter segments; `None`: the bank offers none of them. Use
    /// [`BankModel::enable_payments`] to set it consistently.
    pub payments: Option<PaymentsModel>,
    /// HKKAZ answers carry an MT942 part (the second data element of
    /// HIKAZ) with the account's [`Account::pending`] entries; `None` (the
    /// default) sends none, as Consorsbank did in every live answer so far.
    pub mt942: Option<Mt942Model>,
    /// How the bank answers a wrong typed TAN. Default: two wrong TANs
    /// kept, the third ends the order ([`WrongTanModel::default`]).
    pub wrong_tan: WrongTanModel,
}

/// How the bank answers a wrong typed TAN (HKTAN process 2). PIN/TAN
/// B.5.2 lists 9941, 9210 and 9931 for HKTAN and C.2.1 a failure counter
/// that ends in a lock; RM has 9360 "lock after %1 further failed
/// attempts". Whether a bank keeps the order for another TAN is not in
/// the specification, so both behaviours are modelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrongTanModel {
    /// Wrong TANs answered with `9941` alone: the order (or login) stays
    /// and the same task reference may carry another TAN. The next wrong
    /// TAN ends the order as [`WrongTanModel::end`]. 0: the first wrong
    /// TAN ends it.
    pub kept: u32,
    /// Next to each kept `9941`, `9360` with P1 = the wrong TANs still
    /// possible, the last of which ends the order.
    pub announce_attempts: bool,
    /// What the wrong TAN after the kept ones does.
    pub end: WrongTanEnd,
}

impl Default for WrongTanModel {
    fn default() -> Self {
        Self {
            kept: 2,
            announce_attempts: false,
            end: WrongTanEnd::OrderDeleted,
        }
    }
}

/// How the bank ends an order after too many wrong TANs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrongTanEnd {
    /// `9941` and `9210 Auftrag abgelehnt - Eingereichter Auftrag
    /// gelöscht` on HKTAN: the order is gone (nothing executed), the
    /// dialog stays open. A further HKTAN for it is a violation.
    OrderDeleted,
    /// `9941` on HKTAN and `9800` in HIRMG: the dialog ends.
    DialogEnded,
}

/// How the test bank writes the MT942 part of HIKAZ (HBCI 2.2 IX.2.9).
/// The pending entries are paged like the booked ones: page `n` carries the
/// `n`-th slice of both lists, each slice as its own MT942 statement with
/// its own `:90D:`/`:90C:`, and paging goes on while either list has more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mt942Model {
    /// Erstellungszeitpunkt, the bank's wall clock.
    pub created: jiff::civil::DateTime,
    /// `Some("+0100")`: `:13D:` with that offset; `None`: `:13:` without
    /// one (the HBCI 2.2 form).
    pub offset: Option<&'static str>,
    /// `true`: `:90D:` announces one debit booking more than the
    /// statement lists, so the list is not complete (a broken MT942).
    pub miscount: bool,
    /// `true`: the `:90D:` sum is one cent too high, the count right: the
    /// bank's totals are off, the list is complete.
    pub missum: bool,
}

impl Default for Mt942Model {
    /// `:13D:` at [`TESTBANK_NOW`](crate::TESTBANK_NOW), `+0200` (summer
    /// time on [`TESTBANK_TODAY`](crate::TESTBANK_TODAY)), totals right.
    fn default() -> Self {
        Self {
            created: crate::TESTBANK_NOW,
            offset: Some("+0200"),
            miscount: false,
            missum: false,
        }
    }
}

/// The order in which the bank checks a business order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckOrder {
    /// The TAN requirement first: an order that needs HKTAN and comes
    /// without it gets `9075`, whatever the account allows; the permission
    /// is checked when the order is executed. Consorsbank: HKSAL to the
    /// depot (which allows only HKWPD and HKPRO) without HKTAN got `9050` +
    /// `9075`, resent with HKTAN it got `9380` (trace exchanges 3 and 4).
    #[default]
    TanFirst,
    /// The permission first: an order on an account that does not allow it
    /// gets the not-allowed code, with or without HKTAN, and no TAN is asked
    /// for.
    PermissionFirst,
}

/// The customer's access data.
#[derive(Debug, Clone)]
pub struct Customer {
    /// Benutzerkennung, sent in HKIDN.
    pub user_id: String,
    /// The PIN expected in HNSHA.
    pub pin: String,
    /// Kunden-ID in the UPD.
    pub customer_id: String,
    /// When set, a login with `user_id` gets 3072 with this new user ID.
    /// Both IDs are accepted afterwards.
    pub changed_user_id: Option<String>,
}

/// How the bank answers the HKTAN of the dialog initialisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginSca {
    /// 3076 and a dummy HITAN (no strong authentication needed).
    Exempt,
    /// A TAN challenge with the dialog's TAN method.
    Tan,
}

/// One offered business job.
#[derive(Debug, Clone)]
pub struct JobSpec {
    /// Versions advertised in the parameter segment (HISALS, …). A request
    /// in another version is a violation.
    pub versions: Vec<u16>,
    /// What HIPINS says: does the order need HKTAN?
    pub hipins_needs_tan: bool,
    /// What the bank actually does.
    pub server_tan: TanRule,
    /// Entries per page; more entries are paged with touchdown (3040).
    pub page_size: Option<usize>,
    /// camt descriptors listed in HICAZS, in the bank's order; empty lists
    /// `urn:iso:std:iso:20022:tech:xsd:camt.052.001.02`. Only HKCAZ uses it.
    pub formats: Vec<&'static str>,
}

impl JobSpec {
    /// A job in these versions, no TAN, no paging.
    pub fn new(versions: &[u16]) -> Self {
        Self {
            versions: versions.to_vec(),
            hipins_needs_tan: false,
            server_tan: TanRule::AsHipins,
            page_size: None,
            formats: Vec::new(),
        }
    }
}

/// How the bank treats HKTAN on a business order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TanRule {
    /// HKTAN is required exactly when HIPINS says so; when sent, the bank
    /// grants an exemption (3076).
    AsHipins,
    /// HKTAN is always required (9050 + 9075 without it); with it, the bank
    /// sends a challenge for the dialog's TAN method.
    Required,
    /// As Consorsbank: without HKTAN 9050 + 9075 (no 9800, the dialog
    /// stays); with HKTAN the order is executed and answered with 3076.
    RequiredExempt,
    /// No HKTAN needed; when sent, the bank answers 3076.
    Exempt,
    /// HKTAN is rejected with this code (e.g. `9160`); without HKTAN the
    /// order is executed.
    RejectHktan(&'static str),
    /// A job that pages with touchdown (3040): pages before this one (1 is
    /// the first) are as [`TanRule::Exempt`], this page and every later
    /// one as [`TanRule::Required`]. The page is counted from the
    /// touchdown point, so the job needs [`JobSpec::page_size`].
    OnPage(usize),
}

/// One account in the UPD.
#[derive(Debug, Clone)]
pub struct Account {
    /// IBAN; `None` for a national-only account.
    pub iban: Option<String>,
    /// BIC; `None` uses the bank's.
    pub bic: Option<String>,
    /// Kontonummer.
    pub number: String,
    /// Unterkontomerkmal.
    pub subaccount: Option<String>,
    /// Kontoart (HIUPD 6), e.g. 1 = Kontokorrent, 30 = Depot.
    pub kind: Option<u8>,
    /// Product name.
    pub product: String,
    /// Account holder.
    pub holder: String,
    /// Account currency.
    pub currency: String,
    /// Jobs allowed for this account (HIUPD); others get
    /// [`BankModel::not_allowed_code`].
    pub allowed_jobs: Vec<&'static str>,
    /// Booked balance (closing balance of the bookings).
    pub balance: Decimal,
    /// Date of the booked balance.
    pub balance_date: Date,
    /// Booked entries, oldest first.
    pub bookings: Vec<Booking>,
    /// Not yet booked entries; their sum is the pending balance in HISAL.
    pub pending: Vec<Booking>,
    /// Depot statement (MT535) returned for HKWPD, as is.
    pub mt535: Option<String>,
}

impl Account {
    /// A current account ("Girokonto") of `blz` with a computed German
    /// IBAN, allowed for HKSAL and HKKAZ, without bookings.
    pub fn giro(blz: &str, number: &str, holder: &str) -> Self {
        Self {
            iban: Some(iban_de(blz, number)),
            bic: None,
            number: number.into(),
            subaccount: None,
            kind: Some(1),
            product: "Girokonto".into(),
            holder: holder.into(),
            currency: "EUR".into(),
            allowed_jobs: vec!["HKSAL", "HKKAZ"],
            balance: Decimal::ZERO,
            balance_date: jiff::civil::date(2026, 1, 1),
            bookings: Vec::new(),
            pending: Vec::new(),
            mt535: None,
        }
    }

    /// A depot account allowed for HKWPD that returns `mt535`.
    pub fn depot(blz: &str, number: &str, holder: &str, mt535: &str) -> Self {
        Self {
            kind: Some(30),
            product: "Depot".into(),
            allowed_jobs: vec!["HKWPD"],
            mt535: Some(mt535.into()),
            ..Self::giro(blz, number, holder)
        }
    }

    /// Adds bookings and sets the balance to `opening` plus their sum, dated
    /// on the last booking.
    pub fn with_bookings(mut self, opening: Decimal, bookings: Vec<Booking>) -> Self {
        self.balance = opening + bookings.iter().map(|b| b.amount).sum::<Decimal>();
        if let Some(last) = bookings.last() {
            self.balance_date = last.booking_date;
        }
        self.bookings = bookings;
        self
    }

    /// The opening balance before the first booking.
    pub fn opening_balance(&self) -> Decimal {
        self.balance - self.bookings.iter().map(|b| b.amount).sum::<Decimal>()
    }
}

/// One entry of an account statement.
#[derive(Debug, Clone)]
pub struct Booking {
    /// Buchungsdatum.
    pub booking_date: Date,
    /// Valutadatum.
    pub value_date: Date,
    /// Signed amount: negative is a debit.
    pub amount: Decimal,
    /// Buchungstext (`:86:` ?00).
    pub posting_text: String,
    /// Verwendungszweck (`:86:` ?20…?29).
    pub purpose: String,
    /// Name of the other party (`:86:` ?32/?33).
    pub counterparty: Option<String>,
    /// The bank's own booking reference: camt `Ntry/AcctSvcrRef`. MT940
    /// leaves it out (`:61:` stays `NONREF`).
    pub reference: Option<String>,
}

impl Booking {
    /// A booking on `date` (booking and value date) with a decimal amount
    /// such as `"-12.34"`.
    pub fn new(date: Date, amount: &str, purpose: &str) -> Self {
        let amount: Decimal = amount.parse().expect("decimal amount");
        Self {
            booking_date: date,
            value_date: date,
            posting_text: if amount.is_sign_negative() {
                "UEBERWEISUNG".into()
            } else {
                "GUTSCHRIFT".into()
            },
            amount,
            purpose: purpose.into(),
            counterparty: None,
            reference: None,
        }
    }

    /// Sets the posting text.
    pub fn posting_text(mut self, text: &str) -> Self {
        self.posting_text = text.into();
        self
    }

    /// Sets the other party's name.
    pub fn counterparty(mut self, name: &str) -> Self {
        self.counterparty = Some(name.into());
        self
    }

    /// Sets the bank's booking reference (camt `AcctSvcrRef`).
    pub fn reference(mut self, reference: &str) -> Self {
        self.reference = Some(reference.into());
        self
    }
}

/// A two-step TAN method.
#[derive(Debug, Clone)]
pub struct TanMethod {
    /// Sicherheitsfunktion, e.g. 921.
    pub function: u16,
    /// Name shown to the user.
    pub name: String,
    /// How the TAN is given.
    pub kind: TanKind,
    /// HITANS "Challenge strukturiert": the challenge text carries markup.
    pub challenge_structured: bool,
}

/// How a TAN challenge is answered.
#[derive(Debug, Clone)]
pub enum TanKind {
    /// Approved in an app; the client polls (HKTAN process S, or 2 before
    /// HKTAN 7). The poll number `approve_after_polls` gets the approval.
    Decoupled { approve_after_polls: u32 },
    /// The user types `tan`; the bank shows `challenge`.
    Typed { tan: String, challenge: String },
}

impl TanMethod {
    /// A decoupled method approved on poll number `polls`.
    pub fn decoupled(function: u16, name: &str, polls: u32) -> Self {
        Self {
            function,
            name: name.into(),
            kind: TanKind::Decoupled {
                approve_after_polls: polls,
            },
            challenge_structured: false,
        }
    }

    /// A method where the user types `tan`.
    pub fn typed(function: u16, name: &str, tan: &str) -> Self {
        Self {
            function,
            name: name.into(),
            kind: TanKind::Typed {
                tan: tan.into(),
                challenge: "Bitte geben Sie die TAN ein".into(),
            },
            challenge_structured: false,
        }
    }

    /// Announce "Challenge strukturiert" in HITANS and send the challenge
    /// `text` (which may hold markup).
    pub fn with_structured_challenge(mut self, text: &str) -> Self {
        self.challenge_structured = true;
        if let TanKind::Typed { challenge, .. } = &mut self.kind {
            *challenge = text.into();
        }
        self
    }

    /// Whether the method is decoupled.
    pub fn is_decoupled(&self) -> bool {
        matches!(self.kind, TanKind::Decoupled { .. })
    }
}

impl BankModel {
    /// The job spec for `code`; panics if the bank does not offer it.
    pub fn job_mut(&mut self, code: &str) -> &mut JobSpec {
        self.jobs
            .iter_mut()
            .find(|(c, _)| **c == code)
            .map(|(_, j)| j)
            .unwrap_or_else(|| panic!("the test bank does not offer {code}"))
    }

    /// The first account; panics if there is none.
    pub fn account_mut(&mut self) -> &mut Account {
        self.accounts.first_mut().expect("the bank has an account")
    }

    /// The BIC of an account, falling back to the bank's.
    pub fn bic_of<'a>(&'a self, account: &'a Account) -> &'a str {
        account.bic.as_deref().unwrap_or(&self.bic)
    }
}

impl BankModel {
    /// Offers payments: sets [`BankModel::payments`], lists HKCCS and HKIPZ
    /// in HIPINS as TAN-required (when the model has no entry for them), and
    /// adds HKCCS, HKIPZ (and HKIPS when the model has it) to the allowed
    /// jobs of every account with an IBAN. A profile that must list other
    /// HIPINS entries (HKVPP, HKVPA, HKIPS) adds them to [`BankModel::jobs`]
    /// itself.
    pub fn enable_payments(&mut self, payments: PaymentsModel) {
        for code in ["HKCCS", "HKIPZ"] {
            self.jobs.entry(code).or_insert_with(|| JobSpec {
                hipins_needs_tan: true,
                ..JobSpec::new(&[])
            });
        }
        let mut orders = vec!["HKCCS", "HKIPZ"];
        if payments.hkips.is_some() {
            orders.push("HKIPS");
        }
        for account in self.accounts.iter_mut().filter(|a| a.iban.is_some()) {
            for order in &orders {
                if !account.allowed_jobs.contains(order) {
                    account.allowed_jobs.push(order);
                }
            }
        }
        self.payments = Some(payments);
    }
}

/// What the bank offers for payments. The BPD segments HICCSS, HIIPZS,
/// HIIPSS and HISPAS (formats of HKCCS) are written from it; the engine
/// answers HKCCS, HKIPZ, HKIPS from it.
#[derive(Debug, Clone)]
pub struct PaymentsModel {
    /// HKCCS (HICCSS 1) offered.
    pub hkccs: bool,
    /// pain.001 descriptors for HKCCS, as listed in HISPAS, in order.
    pub hkccs_formats: Vec<&'static str>,
    /// HKIPZ versions offered (1 and/or 2); empty: no HKIPZ.
    pub hkipz_versions: Vec<u16>,
    /// pain.001 descriptors for HKIPZ (HIIPZS), in order.
    pub hkipz_formats: Vec<&'static str>,
    /// HIIPZS "erlaubte Purpose Codes", as the bank lists them.
    pub hkipz_purpose_codes: Option<&'static str>,
    /// HIIPZS 2 "Umwandlung nach SEPA-Überweisung zulässig".
    pub hkipz_conversion_allowed: bool,
    /// HKIPS (HIIPSS 1); `None`: not offered.
    pub hkips: Option<InstantStatusModel>,
    /// An instant payment above this amount is not accepted.
    pub instant_limit: Option<Decimal>,
    /// How the bank rejects an amount above [`PaymentsModel::instant_limit`]
    /// (unless HKIPZ 2 allows the conversion and asks for it: then 3270).
    pub instant_limit_answer: LimitAnswer,
    /// HKIPZ is answered `3045` (status query by HKIPS) when this is above
    /// 0; the first `instant_status_after - 1` HKIPS are answered `3045` as
    /// well and the next one gets HIIPS with the final status. 0: the order
    /// is final at once.
    pub instant_status_after: u32,
    /// The payee registry: IBAN to registered name.
    pub payees: Vec<Payee>,
    /// IBANs the check does not apply to (RVNA), with the reason text.
    pub not_applicable: Vec<(String, String)>,
    /// The payee check (HIVPPS, HIVPAS); `None`: not offered.
    pub vop: Option<VopModel>,
    /// The TAN codes (`0030`, `3955`) of an order's challenge come on the
    /// order's segment (HKCCS) instead of HKTAN; the HITAN stays with HKTAN
    /// (Consorsbank, live 2026-11-18).
    pub tan_codes_on_order: bool,
}

impl Default for PaymentsModel {
    /// HKCCS and HKIPZ 1 with pain.001.001.09, no HKIPS, no instant limit,
    /// no payees, no payee check.
    fn default() -> Self {
        const PAIN001_09: &str = "urn:iso:std:iso:20022:tech:xsd:pain.001.001.09";
        Self {
            hkccs: true,
            hkccs_formats: vec![PAIN001_09],
            hkipz_versions: vec![1],
            hkipz_formats: vec![PAIN001_09],
            hkipz_purpose_codes: None,
            hkipz_conversion_allowed: false,
            hkips: None,
            instant_limit: None,
            instant_limit_answer: LimitAnswer::Code9255,
            instant_status_after: 0,
            payees: Vec::new(),
            not_applicable: Vec::new(),
            vop: None,
            tan_codes_on_order: false,
        }
    }
}

/// HIIPSS parameters.
#[derive(Debug, Clone)]
pub struct InstantStatusModel {
    /// "Mindestwartezeit" in seconds (`num ..2`).
    pub min_wait: u8,
    /// pain.001 descriptors listed (may be empty).
    pub formats: Vec<&'static str>,
}

/// How an amount above the instant limit is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LimitAnswer {
    /// `9255 Echtzeitlimit überschritten` with a link (Rückmeldecodes).
    #[default]
    Code9255,
    /// `9210 Betrag zu groß für Instant Payment Zahlung`.
    Code9210,
}

/// One entry of the payee registry (synthetic).
#[derive(Debug, Clone)]
pub struct Payee {
    /// IBAN of the account.
    pub iban: String,
    /// The registered account holder.
    pub name: String,
}

impl Payee {
    /// An entry.
    pub fn new(iban: &str, name: &str) -> Self {
        Self {
            iban: iban.into(),
            name: name.into(),
        }
    }
}

/// The payee check (Verification of Payee) of a bank.
#[derive(Debug, Clone)]
pub struct VopModel {
    /// HIVPPS "Maximale Anzahl CdtTrfTxInf".
    pub max_transactions: u64,
    /// HIVPPS "Aufklärungstext strukturiert".
    pub structured_text: bool,
    /// HIVPPS "Art der Lieferung".
    pub delivery: Delivery,
    /// pain.002 descriptors in HIVPPS, in order (joined with `;`).
    pub psr_formats: Vec<&'static str>,
    /// Order codes the check is required for (HIVPPS).
    pub required_orders: Vec<&'static str>,
    /// How the result is delivered in HIVPP.
    pub result_as: ResultAs,
    /// The first this many answers to HKVPP (message 1, then each HKVPP
    /// alone) are pending (3093, polling ID); the next one is final.
    pub pending_polls: u32,
    /// HIVPP "Wartezeit vor nächster Abfrage" in seconds (`num 1`, 0-9).
    pub wait_secs: u8,
    /// What the bank does after a clean match.
    pub hkvpa_on_match: OnMatch,
    /// HIVPP explanation text sent with a result that is not a match.
    pub explanation: Option<String>,
    /// Whether the pain.002 lists a matching transaction in `TxInfAndSts`.
    /// Anlage 3 2.2.6.6 requires only the non-RCVC ones; `false` leaves a
    /// match to `NbOfTxsPerSts`.
    pub list_matches: bool,
    /// Whether a pending answer carries a polling ID. VoP C.10.7.1.1 makes
    /// it optional; without one the client polls with the touchdown only.
    pub polling_id: bool,
    /// The final HIVPP carries a VOP-ID but no result (no pain.002, no
    /// DEG) and 3095 instead of 0025: "fachlich kein VOP-Prüfergebnis"
    /// (VoP C.10.7.1.1). The flow is that of a match.
    pub empty_result: bool,
    /// A result that is not a match comes with a TAN challenge instead of
    /// 3945 (against VoP E.8.1 point 5): the client must still send the
    /// execution order after the user confirms.
    pub challenge_on_deviation: bool,
    /// A match comes with 3090 (check the result) instead of 0025.
    pub deviation_on_match: bool,
    /// The pain.002 gives the status only as `PmtInfSts`: no `GrpSts`, no
    /// counts, no `TxInfAndSts`.
    pub block_status_only: bool,
    /// An extra HIRMG code on the final HKVPP answer next to the derived
    /// 3060 (comdirect, live 2026-11-18: `3905`).
    pub message_notice: Option<(&'static str, &'static str)>,
    /// The code the bank puts on HKVPA when it takes the execution order
    /// (DKB `0010`, comdirect `0020`, live 2026-11-18); `None`: HKVPA gets
    /// no code of its own.
    pub vpa_answer: Option<(&'static str, &'static str)>,
    /// The code the bank puts on the order itself (HKCCS) when it executes
    /// it right after HKVPA (comdirect `0010`, live 2026-11-18); `None`:
    /// the generic `0020`.
    pub order_answer_after_vpa: Option<(&'static str, &'static str)>,
}

impl Default for VopModel {
    /// Complete delivery, unstructured text, pain.002.001.10 as URN, the
    /// check required for HKCCS and HKIPZ, result as pain.002, no polling,
    /// HKVPA required with the TAN.
    fn default() -> Self {
        Self {
            max_transactions: 1000,
            structured_text: false,
            delivery: Delivery::Complete,
            psr_formats: vec!["urn:iso:std:iso:20022:tech:xsd:pain.002.001.10"],
            required_orders: vec!["HKCCS", "HKIPZ"],
            result_as: ResultAs::Pain002,
            pending_polls: 0,
            wait_secs: 0,
            hkvpa_on_match: OnMatch::Require,
            explanation: None,
            list_matches: true,
            polling_id: true,
            empty_result: false,
            challenge_on_deviation: false,
            deviation_on_match: false,
            block_status_only: false,
            message_notice: None,
            vpa_answer: None,
            order_answer_after_vpa: None,
        }
    }
}

/// HIVPPS "Art der Lieferung".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// `S`: results come in parts (pain.002 per poll, merged by the client).
    Stepwise,
    /// `V`: only the last answer holds the result.
    Complete,
}

/// How HIVPP carries the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultAs {
    /// A pain.002 payment status report (VoP Anlage 3, 2.2.6).
    Pain002,
    /// The single-transaction DEG of HIVPP.
    SingleDeg,
}

/// What the bank does after a payee check without deviation (VoP E.8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnMatch {
    /// HITAN with the challenge, `0025` and `3091`: no HKVPA follows.
    Skip3091,
    /// HITAN with the challenge and `0025`: the TAN comes with an HKVPA.
    Require,
    /// `0025` and `3945` on HKTAN, no challenge: the order is sent again
    /// with HKVPA and a new HKTAN.
    Always3945,
}

/// A payment the bank executed (after the TAN, or on an SCA waiver).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    /// The order segment: `HKCCS` or `HKIPZ`.
    pub order: &'static str,
    /// Segment version of the order.
    pub version: u16,
    /// IBAN of the debited account.
    pub debtor_iban: String,
    /// pain.001 `MsgId`.
    pub msg_id: String,
    /// pain.001 `EndToEndId`.
    pub end_to_end_id: String,
    /// Payee name as sent.
    pub payee_name: String,
    /// Payee IBAN as sent.
    pub payee_iban: String,
    /// Amount in EUR.
    pub amount: Decimal,
    /// Executed as an instant payment.
    pub instant: bool,
    /// HKIPZ converted to a standard transfer (3270).
    pub converted: bool,
    /// "Auftragsidentifikation" the bank gave an HKIPZ.
    pub order_id: Option<String>,
    /// The VOP-ID the order was released with, if the check ran.
    pub vop_id: Option<Vec<u8>>,
    /// HKIPS status queries answered so far.
    pub status_polls: u32,
}

/// The date `y-m-d`; panics on an invalid date.
pub fn date(y: i16, m: i8, d: i8) -> Date {
    Date::new(y, m, d).expect("valid date")
}

/// A German IBAN for a bank code and account number, with the check digits
/// computed (ISO 13616, mod 97).
pub fn iban_de(blz: &str, number: &str) -> String {
    let bban = format!("{blz:0>8}{number:0>10}");
    // Country "DE" = 13 14, check digits placeholder 00.
    let digits = format!("{bban}131400");
    let rem = digits
        .bytes()
        .fold(0u32, |acc, d| (acc * 10 + u32::from(d - b'0')) % 97);
    format!("DE{:02}{bban}", 98 - rem)
}

impl Default for BankModel {
    /// One Girokonto with two bookings, a typed TAN method (921) in
    /// HITANS 6, login exempt from SCA, HKSAL and HKKAZ 5–7, HKWPD 5–6.
    fn default() -> Self {
        let blz = "12345678";
        let account = Account::giro(blz, "0001234567", "Testkundin Eins").with_bookings(
            "500.00".parse().expect("decimal"),
            vec![
                Booking::new(date(2026, 3, 2), "-45.67", "SVWZ+Testmiete Maerz")
                    .posting_text("DAUERAUFTRAG")
                    .counterparty("Test Vermietung"),
                Booking::new(date(2026, 3, 5), "1200.00", "SVWZ+Testgehalt Maerz")
                    .posting_text("LOHN GEHALT")
                    .counterparty("Test Arbeitgeber"),
            ],
        );
        let mut jobs = BTreeMap::new();
        jobs.insert("HKSAL", JobSpec::new(&[5, 6, 7]));
        jobs.insert("HKKAZ", JobSpec::new(&[5, 6, 7]));
        jobs.insert("HKWPD", JobSpec::new(&[5, 6]));
        Self {
            blz: blz.into(),
            bic: "TESTDEFFXXX".into(),
            name: "Testbank".into(),
            customer: Customer {
                user_id: "testuser1".into(),
                pin: "test-pin-1".into(),
                customer_id: "testuser1".into(),
                changed_user_id: None,
            },
            accounts: vec![account],
            tan_methods: vec![TanMethod::typed(921, "Test-TAN", "123456")],
            hitans_version: 6,
            hitans6_too: false,
            login_sca: LoginSca::Exempt,
            jobs,
            bpd_version: 12,
            upd_version: 4,
            accept_unknown_system_ids: false,
            filter_by_range: false,
            hispas_national: None,
            not_allowed_code: "9210",
            not_allowed_text: "Konto ungültig oder nicht für diesen Auftrag freigegeben",
            check_order: CheckOrder::TanFirst,
            kti_requires_national: false,
            decoupled_max_polls: 0,
            decoupled_wait_secs: 0,
            foreign_blz_9210: false,
            payments: None,
            mt942: None,
            wrong_tan: WrongTanModel::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iban_check_digits() {
        // ISO 13616: move the first four characters to the end, letters to
        // numbers (A = 10), and the remainder mod 97 is 1.
        fn mod97(iban: &str) -> u32 {
            let rearranged = format!("{}{}", &iban[4..], &iban[..4]);
            rearranged.chars().fold(0, |acc, c| {
                let v = c.to_digit(36).expect("alphanumeric");
                if v >= 10 {
                    (acc * 100 + v) % 97
                } else {
                    (acc * 10 + v) % 97
                }
            })
        }
        for (blz, number) in [("12345678", "0001234567"), ("87654321", "0000000042")] {
            let iban = iban_de(blz, number);
            assert_eq!(iban.len(), 22);
            assert_eq!(&iban[4..12], blz);
            assert_eq!(&iban[12..], number);
            assert_eq!(mod97(&iban), 1, "{iban}");
        }
    }

    #[test]
    fn default_balance_is_opening_plus_bookings() {
        let m = BankModel::default();
        assert_eq!(m.accounts[0].balance.to_string(), "1654.33");
        assert_eq!(m.accounts[0].opening_balance().to_string(), "500.00");
    }
}
