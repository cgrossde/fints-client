//! HITANS, Zwei-Schritt-TAN-Einreichung, Parameter (TAN parameters in the
//! BPD). PIN/TAN 2020, B.5.1 / B.5.2 (segment versions 6, 7), E.1.1-E.1.5 c)
//! (segment versions 1-5, archive) and D (Data Dictionary, "Parameter
//! Zwei-Schritt-TAN-Einreichung" and "Verfahrensparameter
//! Zwei-Schritt-Verfahren", element versions #1-#7). Segment version n
//! carries the parameter group in element version #n, which carries the
//! method group in element version #n.
//!
//! All TAN methods of the segment sit inside its one parameter group (DE 5),
//! as a repeated nested group of 21 elements each; on the wire they are
//! one colon-separated run: `J:N:0:<21 elements>:<21 elements>:...`.

use crate::codec::{An, Code, DataElementGroup, Id, Jn, Num, Segment};

/// HITANS version 1, bank parameter segment. PIN/TAN 2020, E.1.1 c):
/// the rows of [`HiTans6`] with the parameter group in element version
/// #1.
/// Element version #1 of the parameter group has a
/// fourth fixed element (Sicherheitsprofil Banken-Signatur bei HITAN).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITANS", version = 1)]
pub struct HiTans1 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Zwei-Schritt-TAN-Einreichung.
    pub parameters: TanParameters1,
}

/// Parameter Zwei-Schritt-TAN-Einreichung, element version #1. PIN/TAN
/// 2020, D. Element version #1 has a fourth fixed element before the methods.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanParameters1 {
    /// 1 Einschritt-Verfahren erlaubt.
    pub one_step_allowed: Jn,
    /// 2 Mehr als ein TAN-pflichtiger Auftrag pro Nachricht erlaubt.
    pub multiple_tan_orders_allowed: Jn,
    /// 3 Auftrags-Hashwertverfahren, `code 1`.
    #[de(len = 1)]
    pub order_hash_method: Code,
    /// 4 Sicherheitsprofil Banken-Signatur bei HITAN, `code 1` (element
    /// version #1 only).
    #[de(len = 1)]
    pub bank_signature_profile: Code,
    /// 5 Verfahrensparameter Zwei-Schritt-Verfahren, 1..98, one per method.
    #[de(min_count = 1, max_count = 98)]
    pub methods: Vec<TanMethod1>,
}

/// Verfahrensparameter Zwei-Schritt-Verfahren, element version #1: one
/// TAN method of 11 elements. PIN/TAN 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanMethod1 {
    /// 1 Sicherheitsfunktion, kodiert, `code ..3`, 900 .. 997.
    #[de(max = 3)]
    pub security_function: Code,
    /// 2 TAN-Prozess, `code 1`, 1 or 2.
    #[de(len = 1)]
    pub tan_process: Code,
    /// 3 Technische Identifikation TAN-Verfahren, `id`.
    pub technical_id: Id,
    /// 4 Name des Zwei-Schritt-Verfahrens, `an ..30`.
    #[de(max = 30)]
    pub name: An,
    /// 5 Maximale Länge des TAN-Eingabewertes im Zwei-Schritt-Verfahren, `num ..2`.
    #[de(max = 2)]
    pub max_tan_length: Num,
    /// 6 Erlaubtes Format im Zwei-Schritt-Verfahren, `code 1`.
    #[de(len = 1)]
    pub allowed_format: Code,
    /// 7 Text zur Belegung des Rückgabewertes im Zwei-Schritt-Verfahren, `an ..30`.
    #[de(max = 30)]
    pub return_value_text: An,
    /// 8 Maximale Länge des Rückgabewertes im Zwei-Schritt-Verfahren, `num ..3`, 1..256.
    #[de(max = 3)]
    pub max_return_value_length: Num,
    /// 9 Anzahl unterstützter aktiver TAN-Listen, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_lists: Option<Num>,
    /// 10 Mehrfach-TAN erlaubt, `jn`.
    pub multiple_tan_allowed: Jn,
    /// 11 TAN zeitversetzt / dialogübergreifend erlaubt, `jn`.
    pub tan_delayed_allowed: Jn,
}

/// HITANS version 2, bank parameter segment. PIN/TAN 2020, E.1.2 c):
/// the rows of [`HiTans6`] with the parameter group in element version
/// #2.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITANS", version = 2)]
pub struct HiTans2 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Zwei-Schritt-TAN-Einreichung.
    pub parameters: TanParameters2,
}

/// Parameter Zwei-Schritt-TAN-Einreichung, element version #2. PIN/TAN
/// 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanParameters2 {
    /// 1 Einschritt-Verfahren erlaubt.
    pub one_step_allowed: Jn,
    /// 2 Mehr als ein TAN-pflichtiger Auftrag pro Nachricht erlaubt.
    pub multiple_tan_orders_allowed: Jn,
    /// 3 Auftrags-Hashwertverfahren, `code 1`.
    #[de(len = 1)]
    pub order_hash_method: Code,
    /// 4 Verfahrensparameter Zwei-Schritt-Verfahren, 1..98, one per method.
    #[de(min_count = 1, max_count = 98)]
    pub methods: Vec<TanMethod2>,
}

/// Verfahrensparameter Zwei-Schritt-Verfahren, element version #2: one
/// TAN method of 15 elements. PIN/TAN 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanMethod2 {
    /// 1 Sicherheitsfunktion, kodiert, `code ..3`, 900 .. 997.
    #[de(max = 3)]
    pub security_function: Code,
    /// 2 TAN-Prozess, `code 1`, 1 or 2.
    #[de(len = 1)]
    pub tan_process: Code,
    /// 3 Technische Identifikation TAN-Verfahren, `id`.
    pub technical_id: Id,
    /// 4 Name des Zwei-Schritt-Verfahrens, `an ..30`.
    #[de(max = 30)]
    pub name: An,
    /// 5 Maximale Länge des TAN-Eingabewertes im Zwei-Schritt-Verfahren, `num ..2`.
    #[de(max = 2)]
    pub max_tan_length: Num,
    /// 6 Erlaubtes Format im Zwei-Schritt-Verfahren, `code 1`.
    #[de(len = 1)]
    pub allowed_format: Code,
    /// 7 Text zur Belegung des Rückgabewertes im Zwei-Schritt-Verfahren, `an ..30`.
    #[de(max = 30)]
    pub return_value_text: An,
    /// 8 Maximale Länge des Rückgabewertes im Zwei-Schritt-Verfahren, `num ..3`, 1..256 (element version 2).
    #[de(max = 3)]
    pub max_return_value_length: Num,
    /// 9 Anzahl unterstützter aktiver TAN-Listen, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_lists: Option<Num>,
    /// 10 Mehrfach-TAN erlaubt, `jn`.
    pub multiple_tan_allowed: Jn,
    /// 11 TAN Zeit- und Dialogbezug, `code 1`.
    #[de(len = 1)]
    pub tan_time_dialog_relation: Code,
    /// 12 TAN-Listennummer erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub tan_list_number_required: Code,
    /// 13 Auftragsstorno erlaubt, `jn`.
    pub cancel_allowed: Jn,
    /// 14 Challenge-Klasse erforderlich, `jn`.
    pub challenge_class_required: Jn,
    /// 15 Challenge-Betrag erforderlich, `jn`.
    pub challenge_amount_required: Jn,
}

/// HITANS version 3, bank parameter segment. PIN/TAN 2020, E.1.3 c):
/// the rows of [`HiTans6`] with the parameter group in element version
/// #3.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITANS", version = 3)]
pub struct HiTans3 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Zwei-Schritt-TAN-Einreichung.
    pub parameters: TanParameters3,
}

/// Parameter Zwei-Schritt-TAN-Einreichung, element version #3. PIN/TAN
/// 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanParameters3 {
    /// 1 Einschritt-Verfahren erlaubt.
    pub one_step_allowed: Jn,
    /// 2 Mehr als ein TAN-pflichtiger Auftrag pro Nachricht erlaubt.
    pub multiple_tan_orders_allowed: Jn,
    /// 3 Auftrags-Hashwertverfahren, `code 1`.
    #[de(len = 1)]
    pub order_hash_method: Code,
    /// 4 Verfahrensparameter Zwei-Schritt-Verfahren, 1..98, one per method.
    #[de(min_count = 1, max_count = 98)]
    pub methods: Vec<TanMethod3>,
}

/// Verfahrensparameter Zwei-Schritt-Verfahren, element version #3: one
/// TAN method of 18 elements. PIN/TAN 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanMethod3 {
    /// 1 Sicherheitsfunktion, kodiert, `code ..3`, 900 .. 997.
    #[de(max = 3)]
    pub security_function: Code,
    /// 2 TAN-Prozess, `code 1`, 1 or 2.
    #[de(len = 1)]
    pub tan_process: Code,
    /// 3 Technische Identifikation TAN-Verfahren, `id`.
    pub technical_id: Id,
    /// 4 Name des Zwei-Schritt-Verfahrens, `an ..30`.
    #[de(max = 30)]
    pub name: An,
    /// 5 Maximale Länge des TAN-Eingabewertes im Zwei-Schritt-Verfahren, `num ..2`.
    #[de(max = 2)]
    pub max_tan_length: Num,
    /// 6 Erlaubtes Format im Zwei-Schritt-Verfahren, `code 1`.
    #[de(len = 1)]
    pub allowed_format: Code,
    /// 7 Text zur Belegung des Rückgabewertes im Zwei-Schritt-Verfahren, `an ..30`.
    #[de(max = 30)]
    pub return_value_text: An,
    /// 8 Maximale Länge des Rückgabewertes im Zwei-Schritt-Verfahren, `num ..3`, 1..256 (element version 2).
    #[de(max = 3)]
    pub max_return_value_length: Num,
    /// 9 Anzahl unterstützter aktiver TAN-Listen, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_lists: Option<Num>,
    /// 10 Mehrfach-TAN erlaubt, `jn`.
    pub multiple_tan_allowed: Jn,
    /// 11 TAN Zeit- und Dialogbezug, `code 1`.
    #[de(len = 1)]
    pub tan_time_dialog_relation: Code,
    /// 12 TAN-Listennummer erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub tan_list_number_required: Code,
    /// 13 Auftragsstorno erlaubt, `jn`.
    pub cancel_allowed: Jn,
    /// 14 Challenge-Klasse erforderlich, `jn`.
    pub challenge_class_required: Jn,
    /// 15 Challenge-Betrag erforderlich, `jn`.
    pub challenge_amount_required: Jn,
    /// 16 Initialisierungsmodus, `code`, 00, 01, 02.
    pub initialisation_mode: Code,
    /// 17 Bezeichnung des TAN-Mediums erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub tan_medium_name_required: Code,
    /// 18 Anzahl unterstützter aktiver TAN-Medien, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_media: Option<Num>,
}

/// HITANS version 4, bank parameter segment. PIN/TAN 2020, E.1.4 c):
/// the rows of [`HiTans6`] with the parameter group in element version
/// #4.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITANS", version = 4)]
pub struct HiTans4 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Zwei-Schritt-TAN-Einreichung.
    pub parameters: TanParameters4,
}

/// Parameter Zwei-Schritt-TAN-Einreichung, element version #4. PIN/TAN
/// 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanParameters4 {
    /// 1 Einschritt-Verfahren erlaubt.
    pub one_step_allowed: Jn,
    /// 2 Mehr als ein TAN-pflichtiger Auftrag pro Nachricht erlaubt.
    pub multiple_tan_orders_allowed: Jn,
    /// 3 Auftrags-Hashwertverfahren, `code 1`.
    #[de(len = 1)]
    pub order_hash_method: Code,
    /// 4 Verfahrensparameter Zwei-Schritt-Verfahren, 1..98, one per method.
    #[de(min_count = 1, max_count = 98)]
    pub methods: Vec<TanMethod4>,
}

/// Verfahrensparameter Zwei-Schritt-Verfahren, element version #4: one
/// TAN method of 22 elements. PIN/TAN 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanMethod4 {
    /// 1 Sicherheitsfunktion, kodiert, `code ..3`, 900 .. 997.
    #[de(max = 3)]
    pub security_function: Code,
    /// 2 TAN-Prozess, `code 1`, 1 or 2.
    #[de(len = 1)]
    pub tan_process: Code,
    /// 3 Technische Identifikation TAN-Verfahren, `id`.
    pub technical_id: Id,
    /// 4 ZKA TAN-Verfahren, `an ..32`, O.
    #[de(max = 32)]
    pub zka_method: Option<An>,
    /// 5 Version ZKA TAN-Verfahren, `an ..10`, O.
    #[de(max = 10)]
    pub zka_version: Option<An>,
    /// 6 Name des Zwei-Schritt-Verfahrens, `an ..30`.
    #[de(max = 30)]
    pub name: An,
    /// 7 Maximale Länge des TAN-Eingabewertes im Zwei-Schritt-Verfahren, `num ..2`.
    #[de(max = 2)]
    pub max_tan_length: Num,
    /// 8 Erlaubtes Format im Zwei-Schritt-Verfahren, `code 1`.
    #[de(len = 1)]
    pub allowed_format: Code,
    /// 9 Text zur Belegung des Rückgabewertes im Zwei-Schritt-Verfahren, `an ..30`.
    #[de(max = 30)]
    pub return_value_text: An,
    /// 10 Maximale Länge des Rückgabewertes im Zwei-Schritt-Verfahren, `num ..3`, 1..256 (element version 2).
    #[de(max = 3)]
    pub max_return_value_length: Num,
    /// 11 Anzahl unterstützter aktiver TAN-Listen, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_lists: Option<Num>,
    /// 12 Mehrfach-TAN erlaubt, `jn`.
    pub multiple_tan_allowed: Jn,
    /// 13 TAN Zeit- und Dialogbezug, `code 1`.
    #[de(len = 1)]
    pub tan_time_dialog_relation: Code,
    /// 14 TAN-Listennummer erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub tan_list_number_required: Code,
    /// 15 Auftragsstorno erlaubt, `jn`.
    pub cancel_allowed: Jn,
    /// 16 SMS-Abbuchungskonto erforderlich, `jn` (element version 1).
    pub sms_charge_account_required: Jn,
    /// 17 Challenge-Klasse erforderlich, `jn`.
    pub challenge_class_required: Jn,
    /// 18 Challenge-Betrag erforderlich, `jn`.
    pub challenge_amount_required: Jn,
    /// 19 Challenge strukturiert, `jn`.
    pub challenge_structured: Jn,
    /// 20 Initialisierungsmodus, `code`, 00, 01, 02.
    pub initialisation_mode: Code,
    /// 21 Bezeichnung des TAN-Mediums erforderlich, `code 1`, 0, 1, 2.
    #[de(len = 1)]
    pub tan_medium_name_required: Code,
    /// 22 Anzahl unterstützter aktiver TAN-Medien, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_media: Option<Num>,
}

/// HITANS version 5, bank parameter segment. PIN/TAN 2020, E.1.5 c):
/// the rows of [`HiTans6`] with the parameter group in element version
/// #5.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITANS", version = 5)]
pub struct HiTans5 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Zwei-Schritt-TAN-Einreichung.
    pub parameters: TanParameters5,
}

/// Parameter Zwei-Schritt-TAN-Einreichung, element version #5. PIN/TAN
/// 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanParameters5 {
    /// 1 Einschritt-Verfahren erlaubt.
    pub one_step_allowed: Jn,
    /// 2 Mehr als ein TAN-pflichtiger Auftrag pro Nachricht erlaubt.
    pub multiple_tan_orders_allowed: Jn,
    /// 3 Auftrags-Hashwertverfahren, `code 1`.
    #[de(len = 1)]
    pub order_hash_method: Code,
    /// 4 Verfahrensparameter Zwei-Schritt-Verfahren, 1..98, one per method.
    #[de(min_count = 1, max_count = 98)]
    pub methods: Vec<TanMethod5>,
}

/// Verfahrensparameter Zwei-Schritt-Verfahren, element version #5: one
/// TAN method of 22 elements. PIN/TAN 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanMethod5 {
    /// 1 Sicherheitsfunktion, kodiert, `code ..3`, 900 .. 997.
    #[de(max = 3)]
    pub security_function: Code,
    /// 2 TAN-Prozess, `code 1`, 1 or 2.
    #[de(len = 1)]
    pub tan_process: Code,
    /// 3 Technische Identifikation TAN-Verfahren, `id`.
    pub technical_id: Id,
    /// 4 ZKA TAN-Verfahren, `an ..32`, O.
    #[de(max = 32)]
    pub zka_method: Option<An>,
    /// 5 Version ZKA TAN-Verfahren, `an ..10`, O.
    #[de(max = 10)]
    pub zka_version: Option<An>,
    /// 6 Name des Zwei-Schritt-Verfahrens, `an ..30`.
    #[de(max = 30)]
    pub name: An,
    /// 7 Maximale Länge des TAN-Eingabewertes im Zwei-Schritt-Verfahren, `num ..2`.
    #[de(max = 2)]
    pub max_tan_length: Num,
    /// 8 Erlaubtes Format im Zwei-Schritt-Verfahren, `code 1`.
    #[de(len = 1)]
    pub allowed_format: Code,
    /// 9 Text zur Belegung des Rückgabewertes im Zwei-Schritt-Verfahren, `an ..30`.
    #[de(max = 30)]
    pub return_value_text: An,
    /// 10 Maximale Länge des Rückgabewertes im Zwei-Schritt-Verfahren, `num ..4`, 1..2048 (element version 3).
    #[de(max = 4)]
    pub max_return_value_length: Num,
    /// 11 Anzahl unterstützter aktiver TAN-Listen, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_lists: Option<Num>,
    /// 12 Mehrfach-TAN erlaubt, `jn`.
    pub multiple_tan_allowed: Jn,
    /// 13 TAN Zeit- und Dialogbezug, `code 1`.
    #[de(len = 1)]
    pub tan_time_dialog_relation: Code,
    /// 14 TAN-Listennummer erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub tan_list_number_required: Code,
    /// 15 Auftragsstorno erlaubt, `jn`.
    pub cancel_allowed: Jn,
    /// 16 SMS-Abbuchungskonto erforderlich, `code 1`, 0, 1, 2 (element version 2).
    #[de(len = 1)]
    pub sms_charge_account_required: Code,
    /// 17 Auftraggeberkonto erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub principal_account_required: Code,
    /// 18 Challenge-Klasse erforderlich, `jn`.
    pub challenge_class_required: Jn,
    /// 19 Challenge strukturiert, `jn`.
    pub challenge_structured: Jn,
    /// 20 Initialisierungsmodus, `code`, 00, 01, 02.
    pub initialisation_mode: Code,
    /// 21 Bezeichnung des TAN-Mediums erforderlich, `code 1`, 0, 1, 2.
    #[de(len = 1)]
    pub tan_medium_name_required: Code,
    /// 22 Anzahl unterstützter aktiver TAN-Medien, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_media: Option<Num>,
}

/// HITANS version 6, bank parameter segment.
///
/// ```text
/// Nr. Name                                    Typ  Format Länge Status Anzahl
///  1  Segmentkopf                             DEG               M     1
///  2  Maximale Anzahl Aufträge                DE   num   ..3    M     1
///  3  Anzahl Signaturen mindestens            DE   num    1     M     1   0, 1, 2, 3
///  4  Sicherheitsklasse                       DE   code   1     M     1   0, 1, 2, 3, 4
///  5  Parameter Zwei-Schritt-TAN-Einreichung  DEG               M     1   (version 6)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITANS", version = 6)]
pub struct HiTans6 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Zwei-Schritt-TAN-Einreichung.
    pub parameters: TanParameters6,
}

/// Parameter Zwei-Schritt-TAN-Einreichung, element version #6.
///
/// ```text
/// Nr. Name                                         Typ  Format Länge Status Anzahl
///  1  Einschritt-Verfahren erlaubt                 DE   jn     #     M     1
///  2  Mehr als ein TAN-pflichtiger Auftrag pro
///     Nachricht erlaubt                            DE   jn     #     M     1
///  3  Auftrags-Hashwertverfahren                   DE   code   1     M     1
///  4  Verfahrensparameter Zwei-Schritt-Verfahren   DEG               M     1..98  (version 6)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanParameters6 {
    /// 1 Einschritt-Verfahren erlaubt.
    pub one_step_allowed: Jn,
    /// 2 Mehr als ein TAN-pflichtiger Auftrag pro Nachricht erlaubt.
    pub multiple_tan_orders_allowed: Jn,
    /// 3 Auftrags-Hashwertverfahren.
    #[de(len = 1)]
    pub order_hash_method: Code,
    /// 4 Verfahrensparameter Zwei-Schritt-Verfahren, one per TAN method.
    #[de(min_count = 1, max_count = 98)]
    pub methods: Vec<TanMethod6>,
}

/// Verfahrensparameter Zwei-Schritt-Verfahren, element version #6: one TAN
/// method.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanMethod6 {
    /// 1 Sicherheitsfunktion, kodiert, `code ..3`, 900 .. 997.
    #[de(max = 3)]
    pub security_function: Code,
    /// 2 TAN-Prozess, `code 1`, 1 or 2.
    #[de(len = 1)]
    pub tan_process: Code,
    /// 3 Technische Identifikation TAN-Verfahren, `id`.
    pub technical_id: Id,
    /// 4 ZKA TAN-Verfahren, `an ..32`, O.
    #[de(max = 32)]
    pub zka_method: Option<An>,
    /// 5 Version ZKA TAN-Verfahren, `an ..10`, O.
    #[de(max = 10)]
    pub zka_version: Option<An>,
    /// 6 Name des Zwei-Schritt-Verfahrens, `an ..30`.
    #[de(max = 30)]
    pub name: An,
    /// 7 Maximale Länge des TAN-Eingabewertes im Zwei-Schritt-Verfahren, `num ..2`.
    #[de(max = 2)]
    pub max_tan_length: Num,
    /// 8 Erlaubtes Format im Zwei-Schritt-Verfahren, `code 1`.
    #[de(len = 1)]
    pub allowed_format: Code,
    /// 9 Text zur Belegung des Rückgabewertes im Zwei-Schritt-Verfahren, `an ..30`.
    #[de(max = 30)]
    pub return_value_text: An,
    /// 10 Maximale Länge des Rückgabewertes im Zwei-Schritt-Verfahren, `num ..4`, 1..2048.
    #[de(max = 4)]
    pub max_return_value_length: Num,
    /// 11 Mehrfach-TAN erlaubt, `jn`.
    pub multiple_tan_allowed: Jn,
    /// 12 TAN Zeit- und Dialogbezug, `code 1`.
    #[de(len = 1)]
    pub tan_time_dialog_relation: Code,
    /// 13 Auftragsstorno erlaubt, `jn`.
    pub cancel_allowed: Jn,
    /// 14 SMS-Abbuchungskonto erforderlich, `code 1`, 0, 1, 2.
    #[de(len = 1)]
    pub sms_charge_account_required: Code,
    /// 15 Auftraggeberkonto erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub principal_account_required: Code,
    /// 16 Challenge-Klasse erforderlich, `jn`.
    pub challenge_class_required: Jn,
    /// 17 Challenge strukturiert, `jn`.
    pub challenge_structured: Jn,
    /// 18 Initialisierungsmodus, `code`, 00, 01, 02.
    pub initialisation_mode: Code,
    /// 19 Bezeichnung des TAN-Mediums erforderlich, `code 1`, 0, 1, 2.
    #[de(len = 1)]
    pub tan_medium_name_required: Code,
    /// 20 Antwort HHD_UC erforderlich, `jn`.
    pub hhd_uc_response_required: Jn,
    /// 21 Anzahl unterstützter aktiver TAN-Medien, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_media: Option<Num>,
}

/// HITANS version 7, bank parameter segment (adds decoupled methods).
/// PIN/TAN 2020, B.5.2 c) and D (element version #7).
///
/// ```text
/// Nr. Name                                    Typ  Format Länge Status Anzahl
///  1  Segmentkopf                             DEG               M     1
///  2  Maximale Anzahl Aufträge                DE   num   ..3    M     1
///  3  Anzahl Signaturen mindestens            DE   num    1     M     1   0, 1, 2, 3
///  4  Sicherheitsklasse                       DE   code   1     M     1   0, 1, 2, 3, 4
///  5  Parameter Zwei-Schritt-TAN-Einreichung  DEG               M     1   (version 7)
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITANS", version = 7)]
pub struct HiTans7 {
    /// 2 Maximale Anzahl Aufträge.
    #[de(max = 3)]
    pub max_orders: Num,
    /// 3 Anzahl Signaturen mindestens.
    #[de(len = 1)]
    pub min_signatures: Num,
    /// 4 Sicherheitsklasse.
    #[de(len = 1)]
    pub security_class: Code,
    /// 5 Parameter Zwei-Schritt-TAN-Einreichung.
    pub parameters: TanParameters7,
}

/// Parameter Zwei-Schritt-TAN-Einreichung, element version #7. PIN/TAN 2020,
/// D. Same four rows as version #6, with the method group in version 7.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanParameters7 {
    /// 1 Einschritt-Verfahren erlaubt.
    pub one_step_allowed: Jn,
    /// 2 Mehr als ein TAN-pflichtiger Auftrag pro Nachricht erlaubt.
    pub multiple_tan_orders_allowed: Jn,
    /// 3 Auftrags-Hashwertverfahren, `code 1`.
    #[de(len = 1)]
    pub order_hash_method: Code,
    /// 4 Verfahrensparameter Zwei-Schritt-Verfahren, 1..98, one per method.
    #[de(min_count = 1, max_count = 98)]
    pub methods: Vec<TanMethod7>,
}

/// Verfahrensparameter Zwei-Schritt-Verfahren, element version #7: one TAN
/// or decoupled method. PIN/TAN 2020, D. Elements 1 to 21 are those of
/// version #6 (rows 4 and 5 are named "DK TAN-Verfahren"), rows 7 and 8
/// become conditional and rows 22 to 26 are new.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct TanMethod7 {
    /// 1 Sicherheitsfunktion, kodiert, `code ..3`, 900 .. 997.
    #[de(max = 3)]
    pub security_function: Code,
    /// 2 TAN-Prozess, `code 1`, 1 or 2.
    #[de(len = 1)]
    pub tan_process: Code,
    /// 3 Technische Identifikation TAN-Verfahren, `id`.
    pub technical_id: Id,
    /// 4 DK TAN-Verfahren, `an ..32`, O (also "Decoupled", "DecoupledPush").
    #[de(max = 32)]
    pub dk_method: Option<An>,
    /// 5 Version DK TAN-Verfahren, `an ..10`, O.
    #[de(max = 10)]
    pub dk_version: Option<An>,
    /// 6 Name des Zwei-Schritt-Verfahrens, `an ..30`.
    #[de(max = 30)]
    pub name: An,
    /// 7 Maximale Länge des TAN-Eingabewertes im Zwei-Schritt-Verfahren,
    /// `num ..2`, C: N for "Decoupled" and "Decoupled push", M otherwise.
    #[de(max = 2)]
    pub max_tan_length: Option<Num>,
    /// 8 Erlaubtes Format im Zwei-Schritt-Verfahren, `code 1`, C: N for
    /// "Decoupled" and "Decoupled push", M otherwise.
    #[de(len = 1)]
    pub allowed_format: Option<Code>,
    /// 9 Text zur Belegung des Rückgabewertes im Zwei-Schritt-Verfahren, `an ..30`.
    #[de(max = 30)]
    pub return_value_text: An,
    /// 10 Maximale Länge des Rückgabewertes im Zwei-Schritt-Verfahren, `num ..4`, 1..2048.
    #[de(max = 4)]
    pub max_return_value_length: Num,
    /// 11 Mehrfach-TAN erlaubt, `jn`.
    pub multiple_tan_allowed: Jn,
    /// 12 TAN Zeit- und Dialogbezug, `code 1`.
    #[de(len = 1)]
    pub tan_time_dialog_relation: Code,
    /// 13 Auftragsstorno erlaubt, `jn`.
    pub cancel_allowed: Jn,
    /// 14 SMS-Abbuchungskonto erforderlich, `code 1`, 0, 1, 2.
    #[de(len = 1)]
    pub sms_charge_account_required: Code,
    /// 15 Auftraggeberkonto erforderlich, `code 1`, 0, 2.
    #[de(len = 1)]
    pub principal_account_required: Code,
    /// 16 Challenge-Klasse erforderlich, `jn`.
    pub challenge_class_required: Jn,
    /// 17 Challenge strukturiert, `jn`.
    pub challenge_structured: Jn,
    /// 18 Initialisierungsmodus, `code`, 00, 01, 02.
    pub initialisation_mode: Code,
    /// 19 Bezeichnung des TAN-Mediums erforderlich, `code 1`, 0, 1, 2.
    #[de(len = 1)]
    pub tan_medium_name_required: Code,
    /// 20 Antwort HHD_UC erforderlich, `jn`.
    pub hhd_uc_response_required: Jn,
    /// 21 Anzahl unterstützter aktiver TAN-Medien, `num 1`, O.
    #[de(len = 1)]
    pub active_tan_media: Option<Num>,
    /// 22 Maximale Anzahl Statusabfragen Decoupled, `num ..3`, C: M for
    /// "Decoupled", N otherwise.
    #[de(max = 3)]
    pub max_status_polls: Option<Num>,
    /// 23 Wartezeit vor erster Statusabfrage, `num ..3`, C: M for
    /// "Decoupled", N otherwise.
    #[de(max = 3)]
    pub wait_before_first_poll: Option<Num>,
    /// 24 Wartezeit vor nächster Statusabfrage, `num ..3`, C: M for
    /// "Decoupled", N otherwise.
    #[de(max = 3)]
    pub wait_before_next_poll: Option<Num>,
    /// 25 Manuelle Bestätigung möglich, `jn`, C: O for "Decoupled", N otherwise.
    pub manual_confirmation_possible: Option<Jn>,
    /// 26 Automatisierte Statusabfragen erlaubt, `jn`, C: O for "Decoupled",
    /// N otherwise.
    pub automated_polling_allowed: Option<Jn>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HITANS:3:6:4+1+1+1+J:N:0:921:2:TESTHHD:HHD:1.4:Testverfahren Alpha:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0'",
    "HITANS:3:7:4+1+1+1+N:N:0:931:2:TESTHHD:HHD:1.4:Kartenleser Test:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0:::::\
     :932:2:TESTAPP:Decoupled::App-Freigabe Test:::Freigabe:2048:N:2:N:0:0:N:N:00:2:N:1:60:4:2:J:J'",
    "HITANS:3:1:4+1+1+1+J:N:0:0:910:2:TESTLST:Liste Test:6:1:TAN:6:1:N:N'",
    "HITANS:3:2:4+1+1+1+J:N:0:911:2:TESTLST:Liste Test:6:1:TAN:6::N:1:0:N:N:N'",
    "HITANS:3:3:4+1+1+1+J:N:0:912:2:TESTSMS:SMS Test:6:1:TAN:6::N:1:0:N:N:N:00:2:1'",
    "HITANS:3:4:4+1+1+1+J:N:0:913:2:TESTHHD:HHD:1.3.2:Kartenleser Test:6:1:TAN:6::N:1:0:N:N:N:N:N:00:0:0'",
    "HITANS:3:5:4+1+1+1+J:N:0:942:2:MTAN2:mobileTAN::mobile TAN:6:1:SMS:2048::J:1:0:N:1:0:N:N:00:2:1:\
     944:2:SECODER_UC:HHD:1.4:chipTAN USB:6:1:TAN:6::N:1:0:N:0:2:N:J:00:0'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{CodecError, ErrorKind, from_wire, to_wire};
    use proptest::prelude::*;

    /// Synthetic HITANS v6: three TAN methods in ONE parameter group
    /// (21 elements per method). The same invented fixture as the
    /// hand-written parser's `regression_hitans_all_methods_in_one_group`.
    const SYNTHETIC_HITANS_V6: &str = "HITANS:41:6:4+1+1+1+J:N:0:\
        921:2:TESTHHD:HHD:1.4:Testverfahren Alpha:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0:\
        922:2:TESTSMS:mobileTAN::Testverfahren Beta:6:1:TAN:6:N:1:N:0:0:N:N:00:2:N:1:\
        923:2:TESTOPT:HHDOPT1:1.4:Testverfahren Gamma:6:1:TAN:6:N:1:N:0:0:N:N:00:1:N:1'";

    fn parse(s: &str) -> Result<HiTans6, CodecError> {
        from_wire::<HiTans6>(s.as_bytes()).map(|(_, v)| v)
    }

    #[test]
    fn all_methods_in_one_group_round_trip() {
        let (header, v) = from_wire::<HiTans6>(SYNTHETIC_HITANS_V6.as_bytes()).unwrap();
        assert_eq!((header.number, header.reference), (41, Some(4)));
        assert_eq!(v.parameters.methods.len(), 3);
        let back = to_wire(&v, header.number, header.reference).unwrap();
        assert_eq!(String::from_utf8(back).unwrap(), SYNTHETIC_HITANS_V6);

        let beta = &v.parameters.methods[1];
        assert_eq!(beta.security_function.as_str(), "922");
        assert_eq!(beta.zka_method.as_ref().map(An::as_str), Some("mobileTAN"));
        assert_eq!(beta.zka_version, None);
        assert_eq!(beta.name.as_str(), "Testverfahren Beta");
        assert_eq!(beta.tan_medium_name_required.as_str(), "2");
        assert_eq!(beta.active_tan_media, Some(Num::new(1)));
        assert!(v.parameters.one_step_allowed.value());
    }

    #[test]
    fn last_method_may_be_cut_after_its_last_filled_element() {
        // The optional element 21 of the last method is left out (H.1.5).
        let cut = SYNTHETIC_HITANS_V6.replace(":00:1:N:1'", ":00:1:N'");
        let v = parse(&cut).unwrap();
        assert_eq!(v.parameters.methods[2].active_tan_media, None);
        assert_eq!(
            String::from_utf8(to_wire(&v, 41, Some(4)).unwrap()).unwrap(),
            cut
        );
    }

    #[test]
    fn method_errors_name_the_method() {
        // Method 2 (index 1) with a 31-byte name.
        let long = SYNTHETIC_HITANS_V6.replace("Testverfahren Beta", &"x".repeat(31));
        let e = parse(&long).unwrap_err();
        assert_eq!(
            e.kind,
            ErrorKind::TooLong {
                max: 30,
                actual: 31
            }
        );
        assert_eq!(e.field, "parameters.methods[1].name");
        // DE 5, element 3 + 21 + 6.
        assert_eq!((e.de, e.element), (Some(5), Some(30)));
        assert_eq!(
            e.segment.map(|s| (s.code, s.number)),
            Some(("HITANS".into(), 41))
        );

        let e = parse("HITANS:41:6:4+1+1+1+J:N:0'").unwrap_err();
        assert_eq!(e.kind, ErrorKind::TooFewRepetitions { min: 1, actual: 0 });

        // A truncated method: the mandatory elements after the name are missing.
        let e = parse("HITANS:41:6:4+1+1+1+J:N:0:921:2:X::1.4:Name'").unwrap_err();
        assert_eq!(e.kind, ErrorKind::Missing);
        assert_eq!(e.field, "parameters.methods[0].max_tan_length");
    }

    #[test]
    fn at_most_98_methods() {
        let mut v = parse(SYNTHETIC_HITANS_V6).unwrap();
        let m = v.parameters.methods[0].clone();
        v.parameters.methods = vec![m.clone(); 98];
        let bytes = to_wire(&v, 41, Some(4)).unwrap();
        assert_eq!(from_wire::<HiTans6>(&bytes).unwrap().1, v);
        v.parameters.methods.push(m);
        assert_eq!(
            to_wire(&v, 41, Some(4)).unwrap_err().kind,
            ErrorKind::TooManyRepetitions {
                max: 98,
                actual: 99
            }
        );
        // 99 methods on the wire: the 99th is data after the 98 the
        // definition allows.
        let one = SYNTHETIC_HITANS_V6
            .split_once("J:N:0:")
            .unwrap()
            .1
            .split(":922:")
            .next()
            .unwrap();
        let wire = format!("HITANS:41:6:4+1+1+1+J:N:0:{}'", vec![one; 99].join(":"));
        let e = parse(&wire).unwrap_err();
        assert_eq!(e.kind, ErrorKind::UnexpectedData);
        assert_eq!((e.de, e.element), (Some(5), Some(3 + 98 * 21 + 1)));
    }

    fn arb_an(max: usize) -> impl Strategy<Value = An> {
        proptest::string::string_regex(&format!("[A-Za-z0-9 +:?@'.-]{{1,{max}}}"))
            .unwrap()
            .prop_filter_map("non-empty after trim", |s| {
                An::new(&s).ok().filter(|a| !a.as_str().is_empty())
            })
    }

    fn code(s: &str) -> Code {
        Code::new(s).unwrap()
    }

    prop_compose! {
        fn arb_method()(
            sf in 900u16..998,
            process in prop_oneof![Just("1"), Just("2")],
            technical_id in "[A-Z0-9]{1,30}",
            zka_method in proptest::option::of(arb_an(32)),
            zka_version in proptest::option::of(arb_an(10)),
            name in arb_an(30),
            max_tan_length in 0u64..100,
            text in arb_an(30),
            max_return in 1u64..2049,
            flags in proptest::collection::vec(any::<bool>(), 6),
            medium in prop_oneof![Just("0"), Just("1"), Just("2")],
            active in proptest::option::of(0u64..10),
        ) -> TanMethod6 {
            TanMethod6 {
                security_function: code(&sf.to_string()),
                tan_process: code(process),
                technical_id: Id::new(technical_id).unwrap(),
                zka_method,
                zka_version,
                name,
                max_tan_length: Num::new(max_tan_length),
                allowed_format: code("1"),
                return_value_text: text,
                max_return_value_length: Num::new(max_return),
                multiple_tan_allowed: Jn(flags[0]),
                tan_time_dialog_relation: code("1"),
                cancel_allowed: Jn(flags[1]),
                sms_charge_account_required: code("0"),
                principal_account_required: code("2"),
                challenge_class_required: Jn(flags[2]),
                challenge_structured: Jn(flags[3]),
                initialisation_mode: code("00"),
                tan_medium_name_required: code(medium),
                hhd_uc_response_required: Jn(flags[4]),
                active_tan_media: active.map(Num::new),
            }
        }
    }

    proptest! {
        #[test]
        fn arbitrary_parameter_segments_round_trip(
            methods in proptest::collection::vec(arb_method(), 1..6),
            one_step in any::<bool>(),
        ) {
            let v = HiTans6 {
                max_orders: Num::new(1),
                min_signatures: Num::new(1),
                security_class: code("1"),
                parameters: TanParameters6 {
                    one_step_allowed: Jn(one_step),
                    multiple_tan_orders_allowed: Jn::NO,
                    order_hash_method: code("0"),
                    methods,
                },
            };
            let bytes = to_wire(&v, 4, Some(3)).unwrap();
            let (_, back) = from_wire::<HiTans6>(&bytes).unwrap();
            prop_assert_eq!(&back, &v);
            prop_assert_eq!(to_wire(&back, 4, Some(3)).unwrap(), bytes);
        }

        /// Random byte changes to a valid segment never panic the parser.
        #[test]
        fn mutated_segments_never_panic(
            edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)
        ) {
            let mut bytes = SYNTHETIC_HITANS_V6.as_bytes().to_vec();
            for (at, byte) in edits {
                let i = at.index(bytes.len());
                bytes[i] = byte;
            }
            let _ = from_wire::<HiTans6>(&bytes);
            if let Ok(segs) = crate::codec::wire::tokenize(&bytes) {
                for s in segs {
                    let _ = crate::codec::decode_segment::<HiTans6>(&s);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests7 {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    /// Synthetic HITANS v7: a classic and a decoupled method, in one group
    /// of 26 elements per method.
    const V7: &str = super::SAMPLES[1];

    #[test]
    fn wire_round_trips() {
        let v: HiTans7 = round_trip(V7);
        let methods = &v.parameters.methods;
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].max_tan_length, Some(Num::new(6)));
        assert_eq!(methods[0].max_status_polls, None);
        let app = &methods[1];
        assert_eq!(app.dk_method.as_ref().map(An::as_str), Some("Decoupled"));
        // Elements 7 and 8 are not sent for a decoupled method.
        assert_eq!(
            (app.max_tan_length, app.allowed_format.as_ref()),
            (None, None)
        );
        assert_eq!(app.max_status_polls, Some(Num::new(60)));
        assert_eq!(app.wait_before_first_poll, Some(Num::new(4)));
        assert_eq!(app.wait_before_next_poll, Some(Num::new(2)));
        assert_eq!(app.manual_confirmation_possible, Some(Jn::YES));
        assert_eq!(app.automated_polling_allowed, Some(Jn::YES));
    }

    /// The hand-written parser's v7 fixture reads, method by method, with
    /// the same security function, name and polling parameters as that
    /// parser's version 7 layout (kept for HITANS above 7). The fixture
    /// carries empty trailing positions, which the codec writes back cut.
    #[test]
    fn agrees_with_the_hand_written_parser() {
        let fixture = "HITANS:42:7:4+1+1+1+N:N:0:\
            931:2:TESTHHD:HHD:1.4:Kartenleser Test:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0::::::\
            932:2:TESTAPP:Decoupled::App-Freigabe Test:::Freigabe:2048:N:2:N:0:0:N:N:00:2:N:1:60:4:2:J:J:\
            933:2:TESTSMS:mobileTAN::SMS Test:6:1:TAN:6:N:1:N:0:0:N:N:00:2:N:1:::::'";
        let raw = crate::codec::wire::tokenize(fixture.as_bytes()).unwrap();
        let old = crate::segments::response::parse_hitans(&raw[0]);
        let (_, new) = from_wire::<HiTans7>(fixture.as_bytes()).unwrap();
        assert_eq!(old.len(), new.parameters.methods.len());
        for (o, n) in old.iter().zip(&new.parameters.methods) {
            assert_eq!(o.security_function.as_str(), n.security_function.as_str());
            assert_eq!(o.name, n.name.as_str());
        }
        assert_eq!(
            new.parameters.methods[1].max_status_polls,
            Some(Num::new(60))
        );
        assert_eq!(old[1].decoupled_max_polls, 60);
    }

    #[test]
    fn rules_are_enforced() {
        // 99 methods are too many; a method needs its name.
        let e = from_wire::<HiTans7>(b"HITANS:3:7:4+1+1+1+J:N:0'").unwrap_err();
        assert_eq!(e.kind, ErrorKind::TooFewRepetitions { min: 1, actual: 0 });
        let bad = V7.replace(":Decoupled:", ":DecoupledDecoupledDecoupledDecoupled:");
        let e = from_wire::<HiTans7>(bad.as_bytes()).unwrap_err();
        assert_eq!(e.field, "parameters.methods[1].dk_method");
    }
}

#[cfg(test)]
mod tests_older {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    #[test]
    fn wire_round_trips() {
        let v: HiTans1 = round_trip(SAMPLES[2]);
        assert_eq!(v.parameters.bank_signature_profile.as_str(), "0");
        let m = &v.parameters.methods[0];
        assert_eq!(
            (m.security_function.as_str(), m.name.as_str()),
            ("910", "Liste Test")
        );
        assert_eq!(m.active_tan_lists, Some(Num::new(1)));
        assert_eq!(m.tan_delayed_allowed, Jn::NO);

        let v: HiTans2 = round_trip(SAMPLES[3]);
        let m = &v.parameters.methods[0];
        assert_eq!(m.active_tan_lists, None);
        assert_eq!(m.tan_list_number_required.as_str(), "0");

        let v: HiTans3 = round_trip(SAMPLES[4]);
        let m = &v.parameters.methods[0];
        assert_eq!(m.tan_medium_name_required.as_str(), "2");
        assert_eq!(m.active_tan_media, Some(Num::new(1)));

        let v: HiTans4 = round_trip(SAMPLES[5]);
        let m = &v.parameters.methods[0];
        assert_eq!(m.zka_method.as_ref().map(An::as_str), Some("HHD"));
        assert_eq!(m.zka_version.as_ref().map(An::as_str), Some("1.3.2"));
        assert_eq!(m.sms_charge_account_required, Jn::NO);
        assert_eq!(m.active_tan_media, Some(Num::new(0)));

        let v: HiTans5 = round_trip(SAMPLES[6]);
        let methods = &v.parameters.methods;
        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].max_return_value_length, Num::new(2048));
        assert_eq!(methods[0].sms_charge_account_required.as_str(), "1");
        assert_eq!(methods[0].principal_account_required.as_str(), "0");
        assert_eq!(methods[1].principal_account_required.as_str(), "2");
        // The last method is cut after its last filled element (H.1.5).
        assert_eq!(methods[1].active_tan_media, None);
    }

    /// The hand-written parser's HITANS 5 fixture (`hitans_v5_layout`):
    /// security function 942, name "mobile TAN", 22 elements per method.
    #[test]
    fn reads_the_hand_written_parsers_fixture() {
        let v: HiTans5 = round_trip(
            "HITANS:53:5:4+1+1+1+N:N:0:942:2:MTAN2:mobileTAN::mobile TAN:6:1:SMS:3:1:J:1:0:N:0:2:N:J:00:1:1'",
        );
        let m = &v.parameters.methods[0];
        assert_eq!(m.security_function.as_str(), "942");
        assert_eq!(m.name.as_str(), "mobile TAN");
        assert_eq!(m.tan_medium_name_required.as_str(), "1");
    }

    #[test]
    fn rules_are_enforced() {
        // Version 1 needs the fourth fixed element before the methods: the
        // version 2 layout read as version 1 fails.
        let e = from_wire::<HiTans1>(SAMPLES[3].replace(":2:4+", ":1:4+").as_bytes()).unwrap_err();
        assert_ne!(e.kind, ErrorKind::UnknownSegment);
        let e = from_wire::<HiTans5>(b"HITANS:3:5:4+1+1+1+J:N:0'").unwrap_err();
        assert_eq!(e.kind, ErrorKind::TooFewRepetitions { min: 1, actual: 0 });
        // Element version 3 of "Maximale Länge des Rückgabewertes" is `num ..4`.
        let e = from_wire::<HiTans4>(SAMPLES[5].replace(":TAN:6::", ":TAN:2048::").as_bytes())
            .unwrap_err();
        assert_eq!(e.field, "parameters.methods[0].max_return_value_length");
    }
}
