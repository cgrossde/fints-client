//! The dialog state machine: initialisation (with synchronisation and
//! system IDs), TAN steps, business orders with touchdown, dialog end.
//!
//! Dialogs live in the bank, not in a connection, so a paused dialog can be
//! resumed by a new client against the same [`BankHandle`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use fints_client::codec::defs::common::{Kik1, Kti1, Ktv3, Sdo2, SdoBtg};
use fints_client::codec::defs::{
    AllowedOrder2, BalanceParameters1, BookedCamt1, CamtTurnoverParameters1, DepotListParameters2,
    HiBpa3, HiCaz1, HiCazs1, HiKaz5, HiKaz6, HiKaz7, HiKazs5, HiKazs6, HiKazs7, HiPins1, HiSal5,
    HiSal6, HiSal7, HiSal8, HiSals5, HiSals6, HiSals7, HiSals8, HiSyn4, HiTan5, HiTan6, HiTan7,
    HiTans6, HiTans7, HiUpa4, HiUpd6, HiWpd5, HiWpd6, HiWpds5, HiWpds6, HkCaz1, HkEnd1, HkIdn2,
    HkKaz5, HkKaz6, HkKaz7, HkSal5, HkSal6, HkSal7, HkSal8, HkSyn3, HkVvb3, HkWpd5, HkWpd6,
    PinTanOrder1, PinTanParameters1, SupportedHbciVersions2, SupportedLanguages2, TanMethod6,
    TanMethod7, TanParameters6, TanParameters7, TurnoverParameters2,
};
use fints_client::codec::{
    self, An, Bin, Code, Cur, Dat, Dig, Id, Jn, Num, ParsedSegment, RawSegment, Segment,
    SegmentHeader, TypedSegment, Wrt,
};
use rust_decimal::Decimal;

use crate::fault::Fault;
use crate::model::Payment;
use crate::model::{
    Account, BankModel, Booking, CheckOrder, JobSpec, LoginSca, TanKind, TanMethod, TanRule,
    WrongTanEnd,
};
use crate::request::{self, Request, Violation};
use crate::response::{self, Answer, Envelope, Feedback, Reply};
use crate::statement;

mod payments;

type Hook = Box<dyn FnMut(&Request, &RawSegment) -> Option<Answer> + Send>;
type ClearTamper = Box<dyn FnMut(&Request, Vec<Vec<u8>>) -> Vec<Vec<u8>> + Send>;
type RawTamper = Box<dyn FnMut(&Request, Vec<u8>) -> Vec<u8> + Send>;
type FaultWhen = Box<dyn FnMut(&Request) -> bool + Send>;

/// A test bank before it starts: the model plus hooks.
pub struct TestBank {
    state: State,
}

/// A running test bank, shared by every connection.
#[derive(Clone)]
pub struct BankHandle(Arc<Mutex<State>>);

struct State {
    model: BankModel,
    dialogs: HashMap<String, DialogState>,
    issued_system_ids: Vec<String>,
    next_dialog: u32,
    next_system_id: u32,
    next_task: u32,
    next_vop: u32,
    payments: Vec<Payment>,
    exchanges: usize,
    requests: Vec<Request>,
    violations: Vec<Violation>,
    hooks: Vec<(&'static str, Hook)>,
    tamper_clear: Option<ClearTamper>,
    tamper_raw: Option<RawTamper>,
    /// Armed one-shot faults, and the one the last exchange fired.
    faults: Vec<(FaultWhen, Fault)>,
    fired: Option<Fault>,
}

#[derive(Debug, Clone)]
struct DialogState {
    user_id: String,
    system_id: String,
    last_message: u32,
    authenticated: bool,
    ended: bool,
    security_function: String,
    pending: Option<Pending>,
    /// Touchdown points issued, as `code/account/point`.
    touchdowns: HashSet<String>,
    /// The payment order being checked or executed in this dialog.
    payment: Option<payments::PaymentCase>,
}

#[derive(Debug, Clone)]
struct Pending {
    task: String,
    kind: TanKind,
    polls: u32,
    /// The order waiting for the TAN; `None` for the dialog initialisation.
    order: Option<(String, RawSegment)>,
    /// The VOP-ID an HKVPA must carry with the first TAN message.
    vpa_expected: Option<Vec<u8>>,
    /// Wrong typed TANs for this task so far.
    wrong_tans: u32,
}

/// HKTAN as the bank needs it, whatever its version.
#[derive(Debug, Clone)]
struct Hktan {
    number: u16,
    version: u16,
    process: String,
    segment_code: Option<String>,
    order_reference: Option<String>,
}

impl TestBank {
    /// A bank with this model.
    pub fn new(model: BankModel) -> Self {
        Self {
            state: State {
                model,
                dialogs: HashMap::new(),
                issued_system_ids: Vec::new(),
                next_dialog: 0,
                next_system_id: 0,
                next_task: 0,
                next_vop: 0,
                payments: Vec::new(),
                exchanges: 0,
                requests: Vec::new(),
                violations: Vec::new(),
                hooks: Vec::new(),
                tamper_clear: None,
                tamper_raw: None,
                faults: Vec::new(),
                fired: None,
            },
        }
    }

    /// Answers business segments with this code through `hook` first; a
    /// `None` falls back to the bank's own answer. The hook's answer is
    /// used as is (no TAN rule applied).
    pub fn on(
        mut self,
        code: &'static str,
        hook: impl FnMut(&Request, &RawSegment) -> Option<Answer> + Send + 'static,
    ) -> Self {
        self.state.hooks.push((code, Box::new(hook)));
        self
    }

    /// Rewrites the inner segments of every answer (each with its `'`)
    /// before the envelope is built. The envelope's numbers are computed
    /// from what the function returns.
    pub fn tamper_clear(
        mut self,
        f: impl FnMut(&Request, Vec<Vec<u8>>) -> Vec<Vec<u8>> + Send + 'static,
    ) -> Self {
        self.state.tamper_clear = Some(Box::new(f));
        self
    }

    /// Rewrites every complete answer (decoded, before base64).
    pub fn tamper_raw(
        mut self,
        f: impl FnMut(&Request, Vec<u8>) -> Vec<u8> + Send + 'static,
    ) -> Self {
        self.state.tamper_raw = Some(Box::new(f));
        self
    }

    /// Delivers `fault` instead of the answer to the first request for
    /// which `when` is true (once; the bank has processed the request).
    /// Faults only act over HTTP ([`TestBank::serve`]).
    pub fn fault(
        mut self,
        when: impl FnMut(&Request) -> bool + Send + 'static,
        fault: Fault,
    ) -> Self {
        self.state.faults.push((Box::new(when), fault));
        self
    }

    /// Answers the `nth` (1-based) business segment with this code with
    /// 9800 (dialog aborted), e.g. the second HKKAZ page.
    pub fn abort_dialog_on(self, code: &'static str, nth: usize) -> Self {
        let mut seen = 0;
        self.on(code, move |_, _| {
            seen += 1;
            (seen == nth).then(|| Answer {
                message: vec![Feedback::new("9800", "Dialog abgebrochen")],
                ..Answer::default()
            })
        })
    }

    /// Starts the bank without a transport.
    pub fn handle(self) -> BankHandle {
        BankHandle(Arc::new(Mutex::new(self.state)))
    }
}

impl BankHandle {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One exchange: a decoded customer message in, a decoded bank message
    /// out. A violation is recorded and returned.
    pub fn exchange(&self, message: &[u8]) -> Result<Vec<u8>, Violation> {
        self.exchange_with(message, None)
    }

    pub(crate) fn exchange_with(
        &self,
        message: &[u8],
        content_type: Option<String>,
    ) -> Result<Vec<u8>, Violation> {
        self.lock().exchange(message, content_type)
    }

    /// Arms a one-shot fault on a running bank, like [`TestBank::fault`].
    pub fn inject(&self, when: impl FnMut(&Request) -> bool + Send + 'static, fault: Fault) {
        self.lock().faults.push((Box::new(when), fault));
    }

    /// The fault fired by the last exchange, if any (taken).
    pub(crate) fn take_fault(&self) -> Option<Fault> {
        self.lock().fired.take()
    }

    /// Every readable request so far.
    pub fn requests(&self) -> Vec<Request> {
        self.lock().requests.clone()
    }

    /// Every violation so far.
    pub fn violations(&self) -> Vec<Violation> {
        self.lock().violations.clone()
    }

    /// Panics if the client broke a message rule.
    pub fn assert_clean(&self) {
        let v = self.violations();
        assert!(
            v.is_empty(),
            "the client broke message rules:\n{}",
            v.iter().map(|v| format!("  {v}\n")).collect::<String>()
        );
    }

    /// Changes the model of the running bank.
    pub fn modify(&self, f: impl FnOnce(&mut BankModel)) {
        f(&mut self.lock().model);
    }

    /// A copy of the current model.
    pub fn model(&self) -> BankModel {
        self.lock().model.clone()
    }

    /// The payments the bank executed, in order.
    pub fn payments(&self) -> Vec<Payment> {
        self.lock().payments.clone()
    }

    /// The user approves every pending decoupled payment in the app, after
    /// the client stopped asking (a cancel, a poll limit, a crash): the
    /// bank executes each, whether or not its dialog is still open. A
    /// payment that waits for an HKVPA with the TAN is not released by the
    /// app alone and stays pending. Returns how many it executed.
    pub fn approve_in_app(&self) -> usize {
        let mut state = self.lock();
        let State {
            model,
            dialogs,
            payments,
            ..
        } = &mut *state;
        let mut executed = 0;
        for dialog in dialogs.values_mut() {
            let releasable = dialog.pending.as_ref().is_some_and(|p| {
                matches!(p.kind, TanKind::Decoupled { .. })
                    && p.vpa_expected.is_none()
                    && p.order
                        .as_ref()
                        .is_some_and(|(code, _)| payments::is_payment(code))
            });
            if !releasable {
                continue;
            }
            let pending = dialog.pending.take().expect("pending payment");
            let (code, raw) = pending.order.expect("payment order");
            let vop_id = dialog.payment.as_ref().and_then(|c| c.vop_id.clone());
            let before = payments.len();
            payments::execute_payment(model, payments, &code, &raw, vop_id)
                .expect("the approved order is readable");
            executed += payments.len() - before;
        }
        executed
    }

    /// The system IDs the bank issued (HISYN), in order.
    pub fn issued_system_ids(&self) -> Vec<String> {
        self.lock().issued_system_ids.clone()
    }
}

fn an(s: &str) -> An {
    An::new(s).unwrap_or_else(|e| panic!("text {s:?}: {e}"))
}

fn code(s: &str) -> Code {
    Code::new(s).unwrap_or_else(|e| panic!("code {s:?}: {e}"))
}

fn id(s: &str) -> Id {
    Id::new(s).unwrap_or_else(|e| panic!("id {s:?}: {e}"))
}

fn num(n: impl Into<u64>) -> Num {
    Num::new(n.into())
}

fn kik(blz: &str) -> Kik1 {
    Kik1 {
        country: Dig::new("280").expect("country code"),
        bank_code: Some(an(blz)),
    }
}

fn sdo(value: Decimal, date: jiff::civil::Date, currency: &str) -> Sdo2 {
    Sdo2 {
        debit_credit: code(if value.is_sign_negative() && !value.is_zero() {
            "D"
        } else {
            "C"
        }),
        amount: SdoBtg {
            value: Wrt::new(value.abs()).expect("amount"),
            currency: Some(Cur::new(currency).expect("currency")),
        },
        date: Some(Dat::new(date).expect("date")),
        time: None,
    }
}

const TEXT_OK: &str = "Auftrag ausgeführt";
const TEXT_SCA_REQUIRED: &str = "Starke Kundenauthentifizierung notwendig";

impl State {
    fn exchange(
        &mut self,
        message: &[u8],
        content_type: Option<String>,
    ) -> Result<Vec<u8>, Violation> {
        let index = self.exchanges;
        self.exchanges += 1;
        let violate = |state: &mut State, message: String| {
            let v = Violation {
                exchange: index,
                message,
            };
            state.violations.push(v.clone());
            v
        };
        let req = match request::parse(index, message, content_type) {
            Ok(r) => r,
            Err(m) => return Err(violate(self, m)),
        };
        self.requests.push(req.clone());
        self.fired = None;
        match self.process(&req) {
            Ok(reply) => {
                let hit = self.faults.iter_mut().position(|(w, _)| w(&req));
                self.fired = hit.map(|i| self.faults.remove(i).1);
                Ok(reply)
            }
            Err(m) => Err(violate(self, m)),
        }
    }

    fn process(&mut self, req: &Request) -> Result<Vec<u8>, String> {
        let model = self.model.clone();
        let (dialog_id, mut dialog, mut answers) = if req.dialog_id == "0" {
            if req.message_number != 1 {
                return Err(format!(
                    "a new dialog starts with message 1, got {}",
                    req.message_number
                ));
            }
            self.next_dialog += 1;
            let dialog_id = format!("TESTDLG{:04}", self.next_dialog);
            let (dialog, answers) = self.init(&model, req)?;
            (dialog_id, dialog, answers)
        } else {
            let mut dialog = self
                .dialogs
                .get(&req.dialog_id)
                .cloned()
                .ok_or_else(|| format!("unknown dialog {}", req.dialog_id))?;
            if dialog.ended {
                return Err(format!("dialog {} has ended", req.dialog_id));
            }
            if req.message_number != dialog.last_message + 1 {
                return Err(format!(
                    "dialog {}: message {} after message {}",
                    req.dialog_id, req.message_number, dialog.last_message
                ));
            }
            dialog.last_message = req.message_number;
            let answers = self.in_dialog(&model, req, &req.dialog_id, &mut dialog)?;
            (req.dialog_id.clone(), dialog, answers)
        };
        answers.sort_by_key(|(reference, _)| *reference);

        let mut inner = response::inner_segments(&answers);
        if let Some(f) = self.tamper_clear.as_mut() {
            inner = f(req, inner);
        }
        dialog.last_message = req.message_number;
        let mut message = response::envelope(
            &Envelope {
                dialog_id: &dialog_id,
                message_number: req.message_number,
                blz: &model.blz,
                user_id: &dialog.user_id,
                system_id: &dialog.system_id,
                security_function: req.security_function.as_deref().unwrap_or("999"),
            },
            &inner,
        );
        if let Some(f) = self.tamper_raw.as_mut() {
            message = f(req, message);
        }
        self.dialogs.insert(dialog_id, dialog);
        Ok(message)
    }

    /// Dialog initialisation: HKIDN, HKVVB, optional HKSYN and HKTAN.
    fn init(
        &mut self,
        m: &BankModel,
        req: &Request,
    ) -> Result<(DialogState, Vec<(u16, Answer)>), String> {
        for c in req.codes() {
            if !matches!(c, "HNSHK" | "HKIDN" | "HKVVB" | "HKSYN" | "HKTAN" | "HNSHA") {
                return Err(format!("{c} in a dialog initialisation"));
            }
        }
        let idn_no = req.find("HKIDN").map(|(n, _)| n).ok_or("HKIDN missing")?;
        let idn: HkIdn2 = req.get().ok_or("HKIDN unreadable")?;
        let (vvb_no, _) = req
            .find("HKVVB")
            .ok_or("HKVVB missing in the initialisation")?;
        let vvb: HkVvb3 = req.get().ok_or("HKVVB unreadable")?;
        let sf = req.security_function.clone().unwrap_or_default();
        let user = idn.customer_id.as_str().to_string();
        let mut dialog = DialogState {
            user_id: user.clone(),
            system_id: idn.system_id.as_str().to_string(),
            last_message: 1,
            authenticated: false,
            ended: false,
            security_function: sf.clone(),
            pending: None,
            touchdowns: HashSet::new(),
            payment: None,
        };

        let bank_code = idn.bank.bank_code.as_ref().map(An::as_str);
        if m.foreign_blz_9210 && idn.bank.country.as_str() == "280" && bank_code != Some(&m.blz) {
            // One host, several bank codes: the user belongs to another one.
            dialog.ended = true;
            let message = [
                ("9050", "Die Nachricht enthält Fehler"),
                ("9800", "Dialog abgebrochen"),
                ("9210", "Bankleitzahl passt nicht zum Benutzer"),
                ("9340", "Ungültige Signatur"),
            ];
            return Ok((
                dialog,
                vec![(
                    idn_no,
                    Answer {
                        message: message.map(|(c, t)| Feedback::new(c, t)).to_vec(),
                        ..Answer::default()
                    },
                )],
            ));
        }
        if idn.bank.country.as_str() != "280" || bank_code != Some(m.blz.as_str()) {
            return Err(format!(
                "HKIDN addresses bank {}:{}, this is {}",
                idn.bank.country,
                bank_code.unwrap_or(""),
                m.blz
            ));
        }
        let known_user = user == m.customer.user_id
            || m.customer.changed_user_id.as_deref() == Some(user.as_str());
        if !known_user || req.pin.as_deref() != Some(m.customer.pin.as_str()) {
            dialog.ended = true;
            return Ok((
                dialog,
                vec![(
                    idn_no,
                    Answer::error("9942", "PIN falsch oder Benutzerkennung unbekannt"),
                )],
            ));
        }
        let system_id = idn.system_id.as_str();
        if system_id != "0"
            && !self.issued_system_ids.iter().any(|s| s == system_id)
            && !m.accept_unknown_system_ids
        {
            dialog.ended = true;
            return Ok((
                dialog,
                vec![(idn_no, Answer::error("9391", "Kundensystem-ID ungültig"))],
            ));
        }

        let mut answers = Vec::new();
        if let Some(new_id) = &m.customer.changed_user_id
            && &user != new_id
        {
            answers.push((
                idn_no,
                Answer::default().with(
                    Feedback::new("3072", "Zugangsdaten geändert").with_params(&[new_id, new_id]),
                ),
            ));
        }

        let mut vvb_answer = Answer::ok(Vec::new());
        let functions: Vec<String> = m
            .tan_methods
            .iter()
            .map(|t| t.function.to_string())
            .collect();
        if !functions.is_empty() {
            let params: Vec<&str> = functions.iter().map(String::as_str).collect();
            vvb_answer = vvb_answer.with(
                Feedback::new(
                    "3920",
                    "Zugelassene Zwei-Schritt-Verfahren für den Benutzer",
                )
                .with_params(&params),
            );
        }
        if vvb.bpd_version.value() < u64::from(m.bpd_version) {
            vvb_answer.data.extend(bpd(m));
        }
        // Version 0 is dialog-only: sent every time, never stored (Formals).
        if m.upd_version == 0 || vvb.upd_version.value() < u64::from(m.upd_version) {
            vvb_answer.data.extend(upd(m, &user));
        }
        answers.push((vvb_no, vvb_answer));

        let syncing = if let Some((syn_no, _)) = req.find("HKSYN") {
            let syn: HkSyn3 = req.get().ok_or("HKSYN unreadable")?;
            if syn.mode.as_str() != "0" {
                return Err(format!("HKSYN mode {} is not supported (only 0)", syn.mode));
            }
            self.next_system_id += 1;
            let new_id = format!("TESTSYS{:04}", self.next_system_id);
            self.issued_system_ids.push(new_id.clone());
            answers.push((
                syn_no,
                Answer::ok(vec![
                    HiSyn4 {
                        system_id: Some(id(&new_id)),
                        message_number: None,
                        signature_id_key: None,
                        signature_id_ds: None,
                    }
                    .into(),
                ]),
            ));
            true
        } else {
            false
        };

        match hktan(req) {
            Some(t) => {
                if sf == "999" {
                    return Err("HKTAN with security function 999".into());
                }
                if t.process != "4" || t.segment_code.as_deref() != Some("HKIDN") {
                    return Err(format!(
                        "HKTAN in the initialisation must be process 4 for HKIDN, got {} for {:?}",
                        t.process, t.segment_code
                    ));
                }
                let method = method_for(m, &sf)?;
                match m.login_sca {
                    LoginSca::Exempt => {
                        answers.push((t.number, exemption(t.version)));
                        dialog.authenticated = true;
                    }
                    LoginSca::Tan => {
                        let (answer, pending) = self.challenge(method, t.version, None);
                        dialog.pending = Some(pending);
                        answers.push((t.number, answer));
                    }
                }
            }
            None if syncing || sf == "999" => {}
            None => match m.login_sca {
                LoginSca::Exempt => dialog.authenticated = true,
                LoginSca::Tan => {
                    answers.push((idn_no, Answer::error("9075", TEXT_SCA_REQUIRED)));
                    dialog.ended = true;
                }
            },
        }
        Ok((dialog, answers))
    }

    /// A message in a running dialog.
    fn in_dialog(
        &mut self,
        m: &BankModel,
        req: &Request,
        dialog_id: &str,
        dialog: &mut DialogState,
    ) -> Result<Vec<(u16, Answer)>, String> {
        let codes = req.codes();
        if let Some((no, _)) = req.find("HKEND") {
            let end: HkEnd1 = req.get().ok_or("HKEND unreadable")?;
            if end.dialog_id.as_str() != dialog_id {
                return Err(format!(
                    "HKEND names dialog {}, sent in {dialog_id}",
                    end.dialog_id
                ));
            }
            if codes.len() != 3 {
                return Err(format!("HKEND must be alone, got {codes:?}"));
            }
            dialog.ended = true;
            return Ok(vec![(
                no,
                Answer {
                    message: vec![Feedback::new("0100", "Dialog beendet")],
                    ..Answer::code("0020", TEXT_OK)
                },
            )]);
        }

        let tan = hktan(req);
        if ["HKVPP", "HKVPA", "HKCCS", "HKIPZ", "HKIPS"]
            .iter()
            .any(|c| req.has(c))
        {
            if !dialog.authenticated {
                return Err("a payment segment before the dialog is authenticated".into());
            }
            return self.payment_message(m, req, dialog, tan.as_ref());
        }
        if let Some(t) = &tan
            && t.process != "4"
        {
            if codes.len() != 3 {
                return Err(format!(
                    "HKTAN process {} must be alone, got {codes:?}",
                    t.process
                ));
            }
            return continue_tan(m, req, t, dialog, &mut self.payments);
        }
        if let Some(t) = &tan
            && !codes.iter().any(|c| Some(*c) == t.segment_code.as_deref())
        {
            return Err(format!(
                "HKTAN process 4 for {:?}, which is not in the message",
                t.segment_code
            ));
        }

        let mut answers = Vec::new();
        for seg in &req.segments {
            let ParsedSegment::Typed { header, .. } = seg else {
                continue;
            };
            let code = header.code.as_str();
            if matches!(code, "HNSHK" | "HNSHA" | "HKTAN") {
                continue;
            }
            if matches!(code, "HKIDN" | "HKVVB" | "HKSYN") {
                return Err(format!("{code} in a running dialog"));
            }
            if !dialog.authenticated {
                return Err(format!("{code} before the dialog is authenticated"));
            }
            let raw = seg.to_raw().map_err(|e| e.to_string())?;
            if let Some(answer) = self
                .hooks
                .iter_mut()
                .filter(|(c, _)| *c == code)
                .find_map(|(_, hook)| hook(req, &raw))
            {
                answers.push((header.number, answer));
                continue;
            }
            let job = m
                .jobs
                .get(code)
                .ok_or_else(|| format!("{code} is not offered by the bank"))?;
            if !job.versions.contains(&header.version) {
                return Err(format!(
                    "{code} version {} is not in the BPD (offered: {:?})",
                    header.version, job.versions
                ));
            }
            if m.check_order == CheckOrder::PermissionFirst
                && let Some(r) = order_account(code, &raw)?
                && account_for(m, &r, code).is_none()
            {
                answers.push((header.number, not_allowed(m)));
                continue;
            }
            let order_tan = tan
                .as_ref()
                .filter(|t| t.segment_code.as_deref() == Some(code));
            let rule = match job.server_tan {
                TanRule::OnPage(first) if page_of(job, code, &raw)? >= first => TanRule::Required,
                TanRule::OnPage(_) => TanRule::Exempt,
                rule => rule,
            };
            match (rule, order_tan) {
                (TanRule::RejectHktan(c), Some(t)) => {
                    answers.push((t.number, Answer::error(c, "HKTAN nicht zulässig")));
                }
                (rule, None) if needs_tan(rule, job) => {
                    answers.push((header.number, Answer::error("9075", TEXT_SCA_REQUIRED)));
                }
                (TanRule::Required, Some(t)) => {
                    let method = method_for(m, &dialog.security_function)?;
                    let (answer, pending) =
                        self.challenge(method, t.version, Some((code.to_string(), raw)));
                    dialog.pending = Some(pending);
                    answers.push((t.number, answer));
                }
                (_, Some(t)) => {
                    answers.push((header.number, execute(m, dialog, code, &raw)?));
                    answers.push((t.number, exemption(t.version)));
                }
                (_, None) => answers.push((header.number, execute(m, dialog, code, &raw)?)),
            }
        }
        Ok(answers)
    }

    /// A TAN challenge for `method`; returns the answer to HKTAN and what
    /// the bank waits for.
    fn challenge(
        &mut self,
        method: &TanMethod,
        hktan_version: u16,
        order: Option<(String, RawSegment)>,
    ) -> (Answer, Pending) {
        self.next_task += 1;
        let task = format!("TASK{:04}", self.next_task);
        let answer = match &method.kind {
            TanKind::Decoupled { .. } => {
                Answer::code("3955", "Sicherheitsfreigabe erfolgt über anderen Kanal").with_data(
                    hitan(
                        hktan_version,
                        "4",
                        Some(&task),
                        Some(&format!(
                            "Bitte den Auftrag in der App {} freigeben",
                            method.name
                        )),
                    ),
                )
            }
            TanKind::Typed { challenge, .. } => Answer::code(
                "0030",
                "Auftrag empfangen - Sicherheitsfreigabe erforderlich",
            )
            .with_data(hitan(hktan_version, "4", Some(&task), Some(challenge))),
        };
        (
            answer,
            Pending {
                task,
                kind: method.kind.clone(),
                polls: 0,
                order,
                vpa_expected: None,
                wrong_tans: 0,
            },
        )
    }
}

/// The page (1 is the first) of a touchdown job's request, from its
/// touchdown point (`TD-<offset>`, see [`transactions`]).
fn page_of(job: &JobSpec, code: &str, raw: &RawSegment) -> Result<usize, String> {
    let touchdown = match (code, version_of(raw)?) {
        ("HKKAZ", 5) => decode::<HkKaz5>(raw)?.touchdown,
        ("HKKAZ", 6) => decode::<HkKaz6>(raw)?.touchdown,
        ("HKKAZ", _) => decode::<HkKaz7>(raw)?.touchdown,
        _ => {
            return Err(format!(
                "TanRule::OnPage: {code} does not page in the test bank"
            ));
        }
    };
    let size = job.page_size.unwrap_or(usize::MAX).max(1);
    Ok(match touchdown {
        None => 1,
        Some(point) => {
            point
                .as_str()
                .strip_prefix("TD-")
                .and_then(|n| n.parse::<usize>().ok())
                .ok_or_else(|| format!("touchdown point {point} is not the bank's"))?
                / size
                + 1
        }
    })
}

fn needs_tan(rule: TanRule, job: &JobSpec) -> bool {
    match rule {
        TanRule::AsHipins => job.hipins_needs_tan,
        TanRule::Required | TanRule::RequiredExempt => true,
        TanRule::Exempt | TanRule::RejectHktan(_) | TanRule::OnPage(_) => false,
    }
}

fn method_for<'a>(m: &'a BankModel, security_function: &str) -> Result<&'a TanMethod, String> {
    m.tan_methods
        .iter()
        .find(|t| t.function.to_string() == security_function)
        .ok_or_else(|| format!("security function {security_function} is not offered"))
}

/// HKTAN of a request, any version.
fn hktan(req: &Request) -> Option<Hktan> {
    req.segments.iter().find_map(|s| {
        let ParsedSegment::Typed { header, segment } = s else {
            return None;
        };
        let text = |a: &Option<An>| a.as_ref().map(|v| v.as_str().to_string());
        let (process, segment_code, order_reference) = match segment.as_ref() {
            TypedSegment::HkTan5(t) => {
                (&t.process, text(&t.segment_code), text(&t.order_reference))
            }
            TypedSegment::HkTan6(t) => {
                (&t.process, text(&t.segment_code), text(&t.order_reference))
            }
            TypedSegment::HkTan7(t) => {
                (&t.process, text(&t.segment_code), text(&t.order_reference))
            }
            _ => return None,
        };
        Some(Hktan {
            number: header.number,
            version: header.version,
            process: process.as_str().to_string(),
            segment_code,
            order_reference,
        })
    })
}

/// HITAN in the version of the HKTAN it answers.
fn hitan(version: u16, process: &str, reference: Option<&str>, challenge: Option<&str>) -> Reply {
    let reference = reference.map(an);
    let challenge = challenge.map(an);
    match version {
        5 => HiTan5 {
            process: code(process),
            order_hash: None,
            order_reference: reference,
            challenge,
            challenge_hhd_uc: None,
            challenge_valid_until: None,
            tan_list_number: None,
            ben: None,
            tan_medium_name: None,
        }
        .into(),
        6 => HiTan6 {
            process: code(process),
            order_hash: None,
            order_reference: reference,
            challenge,
            challenge_hhd_uc: None,
            challenge_valid_until: None,
            tan_medium_name: None,
        }
        .into(),
        _ => HiTan7 {
            process: code(process),
            order_hash: None,
            order_reference: reference,
            challenge,
            challenge_hhd_uc: None,
            challenge_valid_until: None,
            tan_medium_name: None,
        }
        .into(),
    }
}

/// 3076 with the dummy HITAN of an SCA exemption (PIN/TAN B.4.2: order
/// reference `noref`, challenge `nochallenge`).
fn raw_params(code: &'static str, version: u16, body: &str) -> Reply {
    Reply::Raw {
        code,
        version,
        body: body.to_string(),
    }
}

fn exemption(hktan_version: u16) -> Answer {
    Answer::code("3076", "Starke Kundenauthentifizierung nicht notwendig").with_data(hitan(
        hktan_version,
        "4",
        Some("noref"),
        Some("nochallenge"),
    ))
}

/// HKTAN process 2 or S: the TAN of a pending challenge.
fn continue_tan(
    m: &BankModel,
    req: &Request,
    t: &Hktan,
    dialog: &mut DialogState,
    payments: &mut Vec<Payment>,
) -> Result<Vec<(u16, Answer)>, String> {
    let pending = dialog
        .pending
        .as_mut()
        .ok_or_else(|| format!("HKTAN process {} without a pending TAN", t.process))?;
    if pending.vpa_expected.is_some() {
        return Err("the bank waits for an HKVPA with the TAN, HKTAN came alone".into());
    }
    if t.order_reference.as_deref() != Some(pending.task.as_str()) {
        return Err(format!(
            "HKTAN order reference {:?}, the bank waits for {}",
            t.order_reference, pending.task
        ));
    }
    match &pending.kind {
        TanKind::Decoupled {
            approve_after_polls,
        } => {
            // PIN/TAN B.5.2: process S exists from HKTAN 7; before, the
            // status is asked with process 2.
            let expected = if t.version >= 7 { "S" } else { "2" };
            if t.process != expected {
                return Err(format!(
                    "decoupled status with HKTAN {} uses process {expected}, got {}",
                    t.version, t.process
                ));
            }
            pending.polls += 1;
            if pending.polls < *approve_after_polls {
                return Ok(vec![(
                    t.number,
                    Answer::code("3956", "Starke Kundenauthentifizierung noch ausstehend"),
                )]);
            }
        }
        TanKind::Typed { tan, .. } => {
            if t.process != "2" {
                return Err(format!("a typed TAN uses process 2, got {}", t.process));
            }
            if req.tan.as_deref() != Some(tan.as_str()) {
                pending.wrong_tans += 1;
                return Ok(vec![(t.number, wrong_tan(m, dialog))]);
            }
        }
    }
    let pending = dialog.pending.take().expect("pending TAN");
    let mut answer = Answer::ok(vec![hitan(
        t.version,
        &t.process,
        Some(&pending.task),
        None,
    )]);
    match pending.order {
        None => dialog.authenticated = true,
        Some((code, raw)) if payments::is_payment(&code) => {
            let vop_id = dialog.payment.as_ref().and_then(|c| c.vop_id.clone());
            let order = payments::execute_payment(m, payments, &code, &raw, vop_id)?;
            answer
                .segment
                .extend(order.segment.into_iter().filter(|f| f.code != "0020"));
            answer.data.extend(order.data);
        }
        Some((code, raw)) if code == "HKIPS" => {
            let p = m.payments.as_ref().ok_or("the bank offers no payments")?;
            let ips: fints_client::codec::defs::HkIps1 = decode(&raw)?;
            let order = payments::instant_status_answer(m, p, payments, &ips);
            answer
                .segment
                .extend(order.segment.into_iter().filter(|f| f.code != "0020"));
            answer.data.extend(order.data);
        }
        Some((code, raw)) => {
            let order = execute(m, dialog, &code, &raw)?;
            answer
                .segment
                .extend(order.segment.into_iter().filter(|f| f.code != "0020"));
            answer.data.extend(order.data);
        }
    }
    Ok(vec![(t.number, answer)])
}

/// The answer to a wrong typed TAN, by [`WrongTanModel`]: the order kept
/// for another TAN, deleted, or the dialog ended.
fn wrong_tan(m: &BankModel, dialog: &mut DialogState) -> Answer {
    let rule = m.wrong_tan;
    let wrong = dialog.pending.as_ref().map_or(1, |p| p.wrong_tans);
    let invalid = Feedback::new("9941", "TAN ungültig");
    if wrong <= rule.kept {
        let mut answer = Answer::error("9941", "TAN ungültig");
        if rule.announce_attempts {
            let left = (rule.kept + 1 - wrong).to_string();
            answer = answer.with(
                Feedback::new(
                    "9360",
                    &format!("Sperrung der TAN-Liste nach weiteren {left} Fehlversuchen"),
                )
                .with_params(&[left.as_str()]),
            );
        }
        return answer;
    }
    dialog.pending = None;
    match rule.end {
        WrongTanEnd::OrderDeleted => Answer::default().with(invalid).with(Feedback::new(
            "9210",
            "Auftrag abgelehnt - Eingereichter Auftrag gelöscht",
        )),
        WrongTanEnd::DialogEnded => {
            dialog.ended = true;
            Answer {
                message: vec![
                    Feedback::new("9050", "Die Nachricht enthält Fehler"),
                    Feedback::new("9800", "Dialog abgebrochen"),
                ],
                ..Answer::default().with(invalid)
            }
        }
    }
}

/// An account as an order names it.
struct AccountRef {
    iban: Option<String>,
    number: Option<String>,
    /// The subaccount the order gave (empty is none).
    subaccount: Option<String>,
    /// Whether the order gave the national parts (always for a KTV).
    national: bool,
}

impl AccountRef {
    fn ktv(k: &Ktv3) -> Self {
        Self {
            iban: None,
            number: Some(k.account_number.as_str().to_string()),
            subaccount: k.subaccount.as_ref().map(|v| v.as_str().to_string()),
            national: true,
        }
    }

    fn kti(k: &Kti1) -> Self {
        Self {
            iban: k.iban.as_ref().map(|v| v.as_str().to_string()),
            number: k.account_number.as_ref().map(|v| v.as_str().to_string()),
            subaccount: k.subaccount.as_ref().map(|v| v.as_str().to_string()),
            national: k.account_number.is_some() && k.bank.is_some(),
        }
    }

    /// The account named; with the national parts the subaccount must
    /// match too (accounts may share IBAN and number).
    fn find<'a>(&self, m: &'a BankModel) -> Option<&'a Account> {
        let sub = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_string);
        m.accounts.iter().find(|a| {
            let named = match (&self.iban, &self.number) {
                (Some(iban), _) => a.iban.as_deref() == Some(iban.as_str()),
                (None, Some(n)) => &a.number == n,
                (None, None) => false,
            };
            named
                && (!self.national
                    || sub(self.subaccount.as_deref()) == sub(a.subaccount.as_deref()))
        })
    }
}

fn decode<T: Segment>(raw: &RawSegment) -> Result<T, String> {
    codec::decode_segment::<T>(raw)
        .map(|(_, v)| v)
        .map_err(|e| e.to_string())
}

fn version_of(raw: &RawSegment) -> Result<u16, String> {
    raw.degs
        .first()
        .map(SegmentHeader::decode)
        .transpose()
        .map_err(|e| e.to_string())?
        .map(|h| h.version)
        .ok_or_else(|| "empty segment".into())
}

/// The account and whether the job is allowed for it.
fn account_for<'a>(m: &'a BankModel, r: &AccountRef, job: &str) -> Option<&'a Account> {
    r.find(m).filter(|a| a.allowed_jobs.contains(&job))
}

/// The answer to an order on an account that does not allow it.
fn not_allowed(m: &BankModel) -> Answer {
    Answer::error(m.not_allowed_code, m.not_allowed_text)
}

/// The account an order names, checked as the bank executes it: first the
/// KTI shape ([`BankModel::kti_requires_national`]), then the permission.
fn checked_account<'a>(m: &'a BankModel, r: &AccountRef, job: &str) -> Result<&'a Account, Answer> {
    if m.kti_requires_national && !r.national {
        return Err(Answer::error("9010", "Verarbeitung nicht möglich"));
    }
    account_for(m, r, job).ok_or_else(|| not_allowed(m))
}

/// The account named by an order the test bank executes; `None` for any
/// other job.
fn order_account(job: &str, raw: &RawSegment) -> Result<Option<AccountRef>, String> {
    let version = version_of(raw)?;
    Ok(Some(match (job, version) {
        ("HKSAL", 5) => AccountRef::ktv(&decode::<HkSal5>(raw)?.account),
        ("HKSAL", 6) => AccountRef::ktv(&decode::<HkSal6>(raw)?.account),
        ("HKSAL", 7) => AccountRef::kti(&decode::<HkSal7>(raw)?.account),
        ("HKSAL", 8) => AccountRef::kti(&decode::<HkSal8>(raw)?.account),
        ("HKCAZ", 1) => AccountRef::kti(&decode::<HkCaz1>(raw)?.account),
        ("HKKAZ", 5) => AccountRef::ktv(&decode::<HkKaz5>(raw)?.account),
        ("HKKAZ", 6) => AccountRef::ktv(&decode::<HkKaz6>(raw)?.account),
        ("HKKAZ", 7) => AccountRef::kti(&decode::<HkKaz7>(raw)?.account),
        ("HKWPD", 5) => AccountRef::ktv(&decode::<HkWpd5>(raw)?.depot),
        ("HKWPD", 6) => AccountRef::ktv(&decode::<HkWpd6>(raw)?.depot),
        ("HKSAL" | "HKKAZ" | "HKWPD" | "HKCAZ", v) => {
            return Err(format!("{job} {v} has no answer in the test bank"));
        }
        _ => return Ok(None),
    }))
}

fn ktv_of(m: &BankModel, a: &Account) -> Ktv3 {
    Ktv3 {
        account_number: id(&a.number),
        subaccount: a.subaccount.as_deref().map(id),
        bank: kik(&m.blz),
    }
}

fn kti_of(m: &BankModel, a: &Account) -> Kti1 {
    match &a.iban {
        Some(iban) => Kti1 {
            iban: Some(an(iban)),
            bic: Some(an(m.bic_of(a))),
            account_number: None,
            subaccount: None,
            bank: None,
        },
        None => Kti1 {
            iban: None,
            bic: None,
            account_number: Some(id(&a.number)),
            subaccount: a.subaccount.as_deref().map(id),
            bank: Some(kik(&m.blz)),
        },
    }
}

/// Executes a business order.
fn execute(
    m: &BankModel,
    dialog: &mut DialogState,
    job: &str,
    raw: &RawSegment,
) -> Result<Answer, String> {
    let version = version_of(raw)?;
    let Some(r) = order_account(job, raw)? else {
        return Err(format!("{job} has no answer in the test bank; add a hook"));
    };
    let a = match checked_account(m, &r, job) {
        Ok(a) => a,
        Err(refused) => return Ok(refused),
    };
    match job {
        "HKSAL" => Ok(Answer::ok(vec![balance(m, a, version)])),
        "HKCAZ" => camt_transactions(m, a, &decode::<HkCaz1>(raw)?),
        "HKKAZ" => {
            let touchdown = match version {
                5 => decode::<HkKaz5>(raw)?.touchdown,
                6 => decode::<HkKaz6>(raw)?.touchdown,
                _ => decode::<HkKaz7>(raw)?.touchdown,
            };
            let range = kaz_range(raw, version)?;
            transactions(
                m,
                dialog,
                a,
                version,
                touchdown.as_ref().map(An::as_str),
                range,
            )
        }
        _ => {
            let Some(mt535) = &a.mt535 else {
                return Ok(Answer::code("3010", "Keine Einträge vorhanden"));
            };
            let statement = Bin(response::to_latin1(mt535));
            Ok(Answer::ok(vec![match version {
                5 => HiWpd5 { statement }.into(),
                _ => HiWpd6 { statement }.into(),
            }]))
        }
    }
}

type DateRange = (Option<jiff::civil::Date>, Option<jiff::civil::Date>);

fn kaz_range(raw: &RawSegment, version: u16) -> Result<DateRange, String> {
    let d = |v: Option<Dat>| v.map(Dat::date);
    Ok(match version {
        5 => {
            let s = decode::<HkKaz5>(raw)?;
            (d(s.from_date), d(s.to_date))
        }
        6 => {
            let s = decode::<HkKaz6>(raw)?;
            (d(s.from_date), d(s.to_date))
        }
        _ => {
            let s = decode::<HkKaz7>(raw)?;
            (d(s.from_date), d(s.to_date))
        }
    })
}

fn balance(m: &BankModel, a: &Account, version: u16) -> Reply {
    let booked = sdo(a.balance, a.balance_date, &a.currency);
    let pending = a.pending.last().map(|last| {
        let sum: Decimal = a.pending.iter().map(|b| b.amount).sum();
        sdo(sum, last.booking_date, &a.currency)
    });
    let product = an(&a.product);
    let currency = Cur::new(&a.currency).expect("currency");
    match version {
        5 => HiSal5 {
            account: ktv_of(m, a),
            product_name: product,
            currency,
            booked,
            pending,
            credit_line: None,
            available: None,
            used: None,
            booking_date: None,
            booking_time: None,
            due_date: None,
        }
        .into(),
        6 => HiSal6 {
            account: ktv_of(m, a),
            product_name: product,
            currency,
            booked,
            pending,
            credit_line: None,
            available: None,
            used: None,
            overdraft: None,
            booking_timestamp: None,
            due_date: None,
        }
        .into(),
        7 => HiSal7 {
            account: kti_of(m, a),
            product_name: product,
            currency,
            booked,
            pending,
            credit_line: None,
            available: None,
            used: None,
            overdraft: None,
            booking_timestamp: None,
            due_date: None,
        }
        .into(),
        _ => HiSal8 {
            account: kti_of(m, a),
            product_name: product,
            currency,
            booked,
            pending,
            credit_line: None,
            available: None,
            used: None,
            overdraft: None,
            booking_timestamp: None,
            due_date: None,
            seizable_from_month_change: None,
        }
        .into(),
    }
}

/// HKCAZ 1: one camt.052 document with every booking in the requested
/// range (no paging, no touchdown); `3010` when there is none.
fn camt_transactions(m: &BankModel, a: &Account, req: &HkCaz1) -> Result<Answer, String> {
    let formats = m
        .jobs
        .get("HKCAZ")
        .map(|j| j.formats.clone())
        .unwrap_or_default();
    let formats = if formats.is_empty() {
        vec!["urn:iso:std:iso:20022:tech:xsd:camt.052.001.02"]
    } else {
        formats
    };
    let descriptor = req
        .camt_messages
        .descriptors
        .iter()
        .find(|d| formats.contains(&d.as_str()))
        .ok_or_else(|| {
            format!(
                "HKCAZ asks for {:?}, the BPD lists {formats:?}",
                req.camt_messages.descriptors
            )
        })?
        .clone();
    if req.touchdown.is_some() {
        return Err("HKCAZ with a touchdown point: the test bank never pages camt".into());
    }
    let (from, to) = (req.from_date.map(Dat::date), req.to_date.map(Dat::date));
    let selected: Vec<Booking> = a
        .bookings
        .iter()
        .filter(|b| {
            !m.filter_by_range
                || (from.is_none_or(|f| b.booking_date >= f)
                    && to.is_none_or(|t| b.booking_date <= t))
        })
        .cloned()
        .collect();
    if selected.is_empty() {
        return Ok(Answer::code("3010", "Keine Umsätze vorhanden"));
    }
    let xml = statement::camt052(
        a.iban.as_deref().unwrap_or(&a.number),
        &selected,
        &a.currency,
    );
    Ok(Answer::ok(vec![
        HiCaz1 {
            account: kti_of(m, a),
            camt_descriptor: descriptor,
            booked: BookedCamt1 {
                documents: vec![Bin(xml.into_bytes())],
            },
            pending: None,
        }
        .into(),
    ]))
}

fn transactions(
    m: &BankModel,
    dialog: &mut DialogState,
    a: &Account,
    version: u16,
    touchdown: Option<&str>,
    (from, to): DateRange,
) -> Result<Answer, String> {
    let selected: Vec<(usize, &Booking)> = a
        .bookings
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            !m.filter_by_range
                || (from.is_none_or(|f| b.booking_date >= f)
                    && to.is_none_or(|t| b.booking_date <= t))
        })
        .collect();
    // Pending entries are not filtered by the range: they have no booking
    // date yet.
    let pending: &[Booking] = if m.mt942.is_some() { &a.pending } else { &[] };
    if selected.is_empty() && pending.is_empty() {
        return Ok(Answer::code("3010", "Keine Umsätze vorhanden"));
    }
    // Page `n` carries the `n`-th slice of both lists (see `Mt942Model`).
    let total = selected.len().max(pending.len());
    let key = |point: &str| format!("HKKAZ/{}/{point}", a.number);
    let start = match touchdown {
        None => 0,
        Some(point) => {
            if !dialog.touchdowns.contains(&key(point)) {
                return Err(format!("touchdown point {point} was not issued"));
            }
            point
                .strip_prefix("TD-")
                .and_then(|n| n.parse::<usize>().ok())
                .ok_or_else(|| format!("touchdown point {point} is not the bank's"))?
        }
    };
    let page_size = m.jobs.get("HKKAZ").and_then(|j| j.page_size);
    let end = page_size.map_or(total, |p| (start + p).min(total));
    let slice = |n: usize| start.min(n)..end.min(n);
    let page: Vec<Booking> = selected[slice(selected.len())]
        .iter()
        .map(|(_, b)| (*b).clone())
        .collect();
    // The balance before the page: every booking before its first one, or
    // all selected ones on a page past the booked list.
    let first_index = selected
        .get(start)
        .map_or_else(|| selected.last().map_or(0, |(i, _)| i + 1), |(i, _)| *i);
    let opening = a.opening_balance()
        + a.bookings[..first_index]
            .iter()
            .map(|b| b.amount)
            .sum::<Decimal>();
    let opening_date = page.first().map_or(a.balance_date, |b| b.booking_date);
    let statement_number = page_size.map_or(1, |p| start / p.max(1) + 1);
    let account = format!("{}/{}", m.blz, a.number);
    let text = statement::mt940(
        &account,
        statement_number,
        opening,
        opening_date,
        &page,
        &a.currency,
    );
    let booked = Bin(response::to_latin1(&text));
    let unbooked = m.mt942.as_ref().map(|model| {
        let text = statement::mt942(
            &account,
            statement_number,
            &pending[slice(pending.len())],
            &a.currency,
            model,
        );
        Bin(response::to_latin1(&text))
    });
    let data: Reply = match version {
        5 => HiKaz5 { booked, unbooked }.into(),
        6 => HiKaz6 { booked, unbooked }.into(),
        _ => HiKaz7 { booked, unbooked }.into(),
    };
    if end < total {
        let point = format!("TD-{end}");
        dialog.touchdowns.insert(key(&point));
        Ok(Answer::default()
            .with(
                Feedback::new("3040", "Es liegen weitere Informationen vor").with_params(&[&point]),
            )
            .with_data(data))
    } else {
        Ok(Answer::ok(vec![data]))
    }
}

fn tan_method6(t: &TanMethod) -> TanMethod6 {
    let (zka, version, tan_length) = match t.kind {
        TanKind::Decoupled { .. } => ("Decoupled", "1.0", 0u64),
        TanKind::Typed { .. } => ("HHD", "1.4", 6),
    };
    TanMethod6 {
        security_function: code(&t.function.to_string()),
        tan_process: code("2"),
        technical_id: id(&format!("TEST{}", t.function)),
        zka_method: Some(an(zka)),
        zka_version: Some(an(version)),
        name: an(&t.name),
        max_tan_length: num(tan_length),
        allowed_format: code("1"),
        return_value_text: an("TAN"),
        max_return_value_length: num(6u64),
        multiple_tan_allowed: Jn(false),
        tan_time_dialog_relation: code("1"),
        cancel_allowed: Jn(false),
        sms_charge_account_required: code("0"),
        principal_account_required: code("0"),
        challenge_class_required: Jn(false),
        challenge_structured: Jn(t.challenge_structured),
        initialisation_mode: code("00"),
        tan_medium_name_required: code("0"),
        hhd_uc_response_required: Jn(false),
        active_tan_media: Some(num(0u64)),
    }
}

fn tan_method7(t: &TanMethod, max_polls: u32, wait_secs: u64) -> TanMethod7 {
    let decoupled = t.is_decoupled();
    let poll = |v: u64| decoupled.then(|| num(v));
    TanMethod7 {
        security_function: code(&t.function.to_string()),
        tan_process: code("2"),
        technical_id: id(&format!("TEST{}", t.function)),
        dk_method: Some(an(if decoupled { "Decoupled" } else { "HHD" })),
        dk_version: Some(an(if decoupled { "1.0" } else { "1.4" })),
        name: an(&t.name),
        max_tan_length: (!decoupled).then(|| num(6u64)),
        allowed_format: (!decoupled).then(|| code("1")),
        return_value_text: an(if decoupled { "Freigabe" } else { "TAN" }),
        max_return_value_length: num(if decoupled { 0u64 } else { 6 }),
        multiple_tan_allowed: Jn(false),
        tan_time_dialog_relation: code("1"),
        cancel_allowed: Jn(false),
        sms_charge_account_required: code("0"),
        principal_account_required: code("0"),
        challenge_class_required: Jn(false),
        challenge_structured: Jn(t.challenge_structured),
        initialisation_mode: code("00"),
        tan_medium_name_required: code("0"),
        hhd_uc_response_required: Jn(false),
        active_tan_media: Some(num(0u64)),
        // 0 = unlimited polls (the default); waits default to 0.
        max_status_polls: poll(u64::from(max_polls)),
        wait_before_first_poll: poll(wait_secs),
        wait_before_next_poll: poll(wait_secs),
        manual_confirmation_possible: decoupled.then_some(Jn(false)),
        automated_polling_allowed: decoupled.then_some(Jn(true)),
    }
}

/// The BPD: HIBPA, HITANS, HIPINS and the job parameters.
fn bpd(m: &BankModel) -> Vec<Reply> {
    let mut out: Vec<Reply> = vec![
        HiBpa3 {
            bpd_version: num(m.bpd_version),
            bank: kik(&m.blz),
            bank_name: an(&m.name),
            max_order_kinds: num(m.jobs.len() as u64),
            languages: SupportedLanguages2 {
                languages: vec![code("1")],
            },
            hbci_versions: SupportedHbciVersions2 {
                versions: vec![num(300u64)],
            },
            max_message_size: None,
            min_timeout: None,
            max_timeout: None,
        }
        .into(),
    ];
    if m.hitans6_too && m.hitans_version == 7 {
        out.push(
            HiTans6 {
                max_orders: num(1u64),
                min_signatures: num(1u64),
                security_class: code("1"),
                parameters: TanParameters6 {
                    one_step_allowed: Jn(true),
                    multiple_tan_orders_allowed: Jn(false),
                    order_hash_method: code("0"),
                    methods: m
                        .tan_methods
                        .iter()
                        .filter(|t| matches!(t.kind, TanKind::Typed { .. }))
                        .map(tan_method6)
                        .collect(),
                },
            }
            .into(),
        );
    }
    if !m.tan_methods.is_empty() {
        out.push(match m.hitans_version {
            7 => HiTans7 {
                max_orders: num(1u64),
                min_signatures: num(1u64),
                security_class: code("1"),
                parameters: TanParameters7 {
                    one_step_allowed: Jn(true),
                    multiple_tan_orders_allowed: Jn(false),
                    order_hash_method: code("0"),
                    methods: m
                        .tan_methods
                        .iter()
                        .map(|t| tan_method7(t, m.decoupled_max_polls, m.decoupled_wait_secs))
                        .collect(),
                },
            }
            .into(),
            _ => HiTans6 {
                max_orders: num(1u64),
                min_signatures: num(1u64),
                security_class: code("1"),
                parameters: TanParameters6 {
                    one_step_allowed: Jn(true),
                    multiple_tan_orders_allowed: Jn(false),
                    order_hash_method: code("0"),
                    methods: m.tan_methods.iter().map(tan_method6).collect(),
                },
            }
            .into(),
        });
    }
    out.push(
        HiPins1 {
            max_orders: num(1u64),
            min_signatures: num(1u64),
            security_class: code("0"),
            parameters: PinTanParameters1 {
                min_pin_length: Some(num(5u64)),
                max_pin_length: Some(num(20u64)),
                max_tan_length: Some(num(6u64)),
                user_id_text: Some(an("Benutzerkennung")),
                customer_id_text: Some(an("Kunden-ID")),
                orders: m
                    .jobs
                    .iter()
                    .map(|(c, j)| PinTanOrder1 {
                        segment_code: an(c),
                        tan_required: Jn(j.hipins_needs_tan),
                    })
                    .collect(),
            },
        }
        .into(),
    );
    let one = || num(1u64);
    let class = || code("1");
    for (job, spec) in &m.jobs {
        for v in &spec.versions {
            let turnover = || TurnoverParameters2 {
                storage_period: num(365u64),
                max_entries_allowed: Jn(true),
                all_accounts_allowed: Jn(false),
            };
            let depot = || DepotListParameters2 {
                max_entries_allowed: Jn(false),
                currency_selectable: Jn(false),
                quality_selectable: Jn(false),
            };
            let reply: Option<Reply> = match (*job, v) {
                ("HKSAL", 5) => Some(
                    HiSals5 {
                        max_orders: one(),
                        min_signatures: one(),
                    }
                    .into(),
                ),
                ("HKSAL", 6) => Some(
                    HiSals6 {
                        max_orders: one(),
                        min_signatures: one(),
                        security_class: class(),
                    }
                    .into(),
                ),
                ("HKSAL", 7) => Some(
                    HiSals7 {
                        max_orders: one(),
                        min_signatures: one(),
                        security_class: class(),
                    }
                    .into(),
                ),
                ("HKKAZ", 5) => Some(
                    HiKazs5 {
                        max_orders: one(),
                        min_signatures: one(),
                        parameters: turnover(),
                    }
                    .into(),
                ),
                ("HKKAZ", 6) => Some(
                    HiKazs6 {
                        max_orders: one(),
                        min_signatures: one(),
                        security_class: class(),
                        parameters: turnover(),
                    }
                    .into(),
                ),
                ("HKKAZ", 7) => Some(
                    HiKazs7 {
                        max_orders: one(),
                        min_signatures: one(),
                        security_class: class(),
                        parameters: turnover(),
                    }
                    .into(),
                ),
                ("HKWPD", 5) => Some(
                    HiWpds5 {
                        max_orders: one(),
                        min_signatures: one(),
                        parameters: depot(),
                    }
                    .into(),
                ),
                ("HKWPD", 6) => Some(
                    HiWpds6 {
                        max_orders: one(),
                        min_signatures: one(),
                        security_class: class(),
                        parameters: depot(),
                    }
                    .into(),
                ),
                // A version the engine has no builder for: the parameter
                // segment is announced with a generic body, so a model can
                // offer a job only in versions the client lacks. Requests
                // for it are still violations unless the model serves them.
                ("HKSAL", 8) => Some(
                    HiSals8 {
                        max_orders: one(),
                        min_signatures: one(),
                        security_class: class(),
                        parameters: BalanceParameters1 {
                            max_entries_allowed: Jn(false),
                        },
                    }
                    .into(),
                ),
                ("HKCAZ", 1) => Some(
                    HiCazs1 {
                        max_orders: one(),
                        min_signatures: one(),
                        security_class: class(),
                        parameters: CamtTurnoverParameters1 {
                            storage_period: num(450u64),
                            entry_count_allowed: Jn(false),
                            all_accounts_allowed: Jn(false),
                            formats: if spec.formats.is_empty() {
                                vec![an("urn:iso:std:iso:20022:tech:xsd:camt.052.001.02")]
                            } else {
                                spec.formats.iter().map(|f| an(f)).collect()
                            },
                        },
                    }
                    .into(),
                ),
                ("HKSAL", _) => Some(raw_params("HISALS", *v, "1+1+0")),
                ("HKKAZ", _) => Some(raw_params("HIKAZS", *v, "1+1+0")),
                ("HKCAZ", _) => Some(raw_params("HICAZS", *v, "1+1+0")),
                ("HKWDU", _) => Some(raw_params("HIWDUS", *v, "1+1+0+N")),
                _ => None,
            };
            out.extend(reply);
        }
    }
    if let Some(p) = &m.payments {
        out.extend(payments::parameters(p));
    }
    let hkccs_formats = m
        .payments
        .as_ref()
        .map(|p| p.hkccs_formats.clone())
        .filter(|f| !f.is_empty());
    if m.hispas_national.is_some() || hkccs_formats.is_some() {
        // SEPA parameters (HISPAS 1): single account query allowed,
        // national account allowed or not, structured purpose allowed, the
        // pain.001 formats of HKCCS. The client reads only the second flag.
        let flag = if m.hispas_national == Some(true) {
            "J"
        } else {
            "N"
        };
        let formats = hkccs_formats
            .unwrap_or_else(|| vec!["sepade.pain.001.001.03.xsd"])
            .iter()
            .map(|f| f.replace(':', "?:"))
            .collect::<Vec<_>>()
            .join(":");
        out.push(Reply::Raw {
            code: "HISPAS",
            version: 1,
            body: format!("1+1+0+J:{flag}:J:{formats}"),
        });
    }
    out
}

/// The UPD: HIUPA and one HIUPD 6 per account.
fn upd(m: &BankModel, user: &str) -> Vec<Reply> {
    let mut out: Vec<Reply> = vec![
        HiUpa4 {
            user_id: id(user),
            upd_version: num(m.upd_version),
            upd_usage: code("0"),
            user_name: None,
            extension: None,
        }
        .into(),
    ];
    for a in &m.accounts {
        out.push(
            HiUpd6 {
                account: Some(ktv_of(m, a)),
                iban: a.iban.as_deref().map(an),
                customer_id: id(&m.customer.customer_id),
                account_type: a.kind.map(|k| num(u64::from(k))),
                currency: Some(Cur::new(&a.currency).expect("currency")),
                holder_name_1: an(&a.holder),
                holder_name_2: None,
                product_name: Some(an(&a.product)),
                limit: None,
                allowed_orders: a
                    .allowed_jobs
                    .iter()
                    .map(|j| AllowedOrder2 {
                        segment_code: an(j),
                        signatures_required: num(1u64),
                        limit_kind: None,
                        limit_amount: None,
                        limit_days: None,
                    })
                    .collect(),
                extension: None,
            }
            .into(),
        );
    }
    out
}
