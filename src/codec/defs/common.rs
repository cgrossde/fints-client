//! Groups used by several segments ("Mehrfach verwendete Elemente").

use crate::codec::Code;
use crate::codec::{An, Cur, Dat, DataElementGroup, Dig, Id, Jn, Num, Tim, Wrt};

/// Kreditinstitutskennung (`kik`, version 1). Messages 2022, B.2.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Kik1 {
    /// 1 Länderkennzeichen, `ctr` = `dig` length 3 (Formals B.4.2), M.
    #[de(len = 3)]
    pub country: Dig,
    /// 2 Kreditinstitutscode, `an ..30`, C (M where the country has bank
    /// codes, e.g. the BLZ in Germany).
    #[de(max = 30)]
    pub bank_code: Option<An>,
}

/// Kontoverbindung international (`kti`, version 1). Messages 2022, B.3.2.
///
/// Not checked here: "if IBAN is given, BIC must be given too" and "the
/// national form needs account number and bank" (Belegungsrichtlinien).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Kti1 {
    /// 1 IBAN, `an ..34`, O.
    #[de(max = 34)]
    pub iban: Option<An>,
    /// 2 BIC, `an ..11`, O.
    #[de(max = 11)]
    pub bic: Option<An>,
    /// 3 Konto-/Depotnummer, `id`, O.
    pub account_number: Option<Id>,
    /// 4 Unterkontomerkmal, `id`, O.
    pub subaccount: Option<Id>,
    /// 5 Kreditinstitutskennung, `kik`, O.
    pub bank: Option<Kik1>,
}

/// Bezugsnachricht, version 1. Formals F (Data Dictionary, "Bezugsnachricht").
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct ReferenceMessage {
    /// 1 Dialog-ID, `id`, M.
    pub dialog_id: Id,
    /// 2 Nachrichtennummer, `num ..4`, M, > 0.
    #[de(max = 4)]
    pub message_number: Num,
}

/// Kontoverbindung (`ktv`). Messages 2022, B.3.1 (the MVE is printed as
/// version 3; the segment tables cite version 2 for accounts and version 3
/// for depots, and the document gives only this one layout).
///
/// Not checked here: the sub-account feature is mandatory when the bank
/// gave one in the UPD (Belegungsrichtlinie of "Kontoverbindung Auftraggeber").
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Ktv3 {
    /// 1 Konto-/Depotnummer, `id`, M.
    pub account_number: Id,
    /// 2 Unterkontomerkmal, `id`, C: M for an ordering account when the
    /// bank gave one in the UPD, N otherwise.
    pub subaccount: Option<Id>,
    /// 3 Kreditinstitutskennung, `kik`, M.
    pub bank: Kik1,
}

/// Kontoverbindung ZV international (`ktz`). Messages 2022, B.3.3.
///
/// Not checked here: IBAN and BIC are mandatory when `sepa_usable` is `J`
/// and empty otherwise; the sub-account feature is mandatory when the bank
/// gave one in the UPD.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Ktz1 {
    /// 1 Kontoverwendung SEPA, `jn`, M.
    pub sepa_usable: Jn,
    /// 2 IBAN, `an ..34`, C.
    #[de(max = 34)]
    pub iban: Option<An>,
    /// 3 BIC, `an ..11`, C.
    #[de(max = 11)]
    pub bic: Option<An>,
    /// 4 Konto-/Depotnummer, `id`, M.
    pub account_number: Id,
    /// 5 Unterkontomerkmal, `id`, C.
    pub subaccount: Option<Id>,
    /// 6 Kreditinstitutskennung, `kik`, M.
    pub bank: Kik1,
}

/// Zeitstempel (`tsp`). Messages 2022, B.6.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Tsp1 {
    /// 1 Datum, `dat`, M.
    pub date: Dat,
    /// 2 Uhrzeit, `tim`, O.
    pub time: Option<Tim>,
}

/// The amount of a Saldo: `btg` (Messages B.1) read with a missing
/// currency. The specification makes the currency mandatory, banks leave it
/// out and the account currency of the segment stands in for it.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SdoBtg {
    /// 1 Wert, `wrt`, M.
    pub value: Wrt,
    /// 2 Währung, `cur`, M by the specification, read as optional.
    pub currency: Option<Cur>,
}

/// Saldo (`sdo`, version 2). Messages 2022, B.4.
///
/// Date (3) and the currency inside the amount are mandatory in the
/// specification. Both are read as optional: a booked balance without them
/// is still a usable amount, and failing the segment would lose it.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Sdo2 {
    /// 1 Soll-Haben-Kennzeichen, `code 1`, M, `C` (credit) or `D` (debit).
    #[de(len = 1)]
    pub debit_credit: Code,
    /// 2 Betrag, `btg`, M.
    pub amount: SdoBtg,
    /// 3 Datum, `dat`, M by the specification, read as optional.
    pub date: Option<Dat>,
    /// 4 Uhrzeit, `tim`, O.
    pub time: Option<Tim>,
}
