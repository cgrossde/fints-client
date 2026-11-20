# Typed BPD/UPD model: design

The typed model of the bank parameter data (BPD) and user parameter data (UPD), built from the parameter segments. `jobs` reads job limits and formats from it. Code: `src/protocol/params.rs`, re-exported from `protocol`. Jobs and capabilities that use it: [jobs-step-design.md](jobs-step-design.md) §4.

## 1. Where the raw data lives

`BankParams` (`src/protocol.rs`) keeps the BPD and UPD as raw segment sets (`bpd_segments`, `upd_segments`) and the typed `bpd()` / `upd()` built from them. `set_bpd` and `set_upd` rebuild the model whenever a set changes. `BankParams::tan_methods` is derived from the model: only methods of a HITANS version we can build.

`is_bpd_segment` decides which response segments are BPD: HIBPA, HIKOM, HISHV, HIKPV, and every job parameter segment `HI…S` or `DI…S` (Formals D.6). HIKOM, HISHV and HIKPV are kept in the set but not read by the model.

Masking (`mask.rs`) and `trace.rs` read HIUPA/HIUPD on the wire and do not use the model.

## 2. Types

All structs are `pub`, `#[non_exhaustive]`, read-only (public fields or getters), `Debug + Clone + Serialize`.

```rust
pub struct Bpd {
    pub version: u16,                          // HIBPA DE 2
    pub country: String, pub bank_code: Option<Blz>, pub bank_name: String,
    pub max_order_kinds: u16,
    pub languages: Vec<u16>, pub hbci_versions: Vec<u16>,
    pub max_message_size: Option<u32>,
    pub min_timeout: Option<u32>, pub max_timeout: Option<u32>,
    pub pin_tan: Option<PinTanParams>,         // HIPINS: PIN/TAN lengths, texts, tan_required per order
    pub tan: TanParams,                        // HITANS: versions: BTreeMap<u16, TanVersion>
    jobs: BTreeMap<SegmentType, JobParams>,    // key: order code (HKSAL), from HI…S → HK…, DI…S → DK…
    unparsed: Vec<UnparsedParam>,
}

pub struct TanVersion { pub one_step_allowed: Option<bool>, pub methods: Vec<TanMethod> }

pub struct JobParams {
    pub code: SegmentType,                     // HKSAL
    pub versions: BTreeMap<u16, JobVersion>,   // one entry per parameter segment version (Formals D.6)
}

pub struct JobVersion {
    pub max_orders: u16,                       // D.6 DE 2
    pub min_signatures: u8,                    // D.6 DE 3
    pub security_class: Option<u8>,            // D.6 DE 4; absent in pre-FinTS versions (HISALS 5, HIKAZS 5, HIWPDS 5)
    pub detail: JobDetail,
}

pub enum JobDetail {
    None,                                      // no job parameters, or kept elsewhere (HIPINS in `pin_tan`, HITANS in `tan`)
    Sepa(SepaParams),                          // HISPAS 1–3
    Statement(StatementParams),                // HIKAZS 5–7
    Camt(CamtParams),                          // HICAZS 1
    Balance(BalanceParams),                    // HISALS 8
    Holdings(HoldingsParams),                  // HIWPDS 5–6
    Instant(InstantParams),                    // HIIPZS 1–2
    InstantStatus(InstantStatusParams),        // HIIPSS 1
    Vop(VopParams),                            // HIVPPS 1
    Raw(RawSegment),                           // header only: no definition, or the definition rejected it
}

pub struct Upd {
    pub version: u16,                          // HIUPA DE 3; 0 = valid for this dialog only (Formals C.3.2.3)
    pub usage: UpdUsage,                       // HIUPA DE 4: Closed (0, unlisted orders blocked) / Open (1, nothing known)
    pub accounts: Vec<UpdAccount>,
    unparsed: Vec<UnparsedParam>,
}

pub struct UpdAccount {
    pub account_number: Option<String>, pub sub_account: Option<String>, pub blz: Option<Blz>,
    pub iban: Option<Iban>, pub bic: Option<Bic>,
    pub customer_id: String,
    pub account_type: Option<u8>,              // Kontoart; `class() -> Option<AccountClass>`
    pub currency: Option<Currency>, pub owner: Vec<String>, pub product_name: Option<String>,
    pub limit: Option<AccountLimit>,
    pub allowed: BTreeMap<SegmentType, AllowedJob>,   // AllowedJob { min_signatures, limit }
}
```

`AccountLimit { kind: LimitKind, amount, currency, days }`. `UpdAccount::key()` gives the account's `AccountKey` (BLZ, number and subaccount, or IBAN and subaccount); `Upd::find(&Account)` looks an entry up. `jobs::AccountInfo` (`src/jobs/accounts.rs`) is built from an `UpdAccount`.

HIPINS and HITANS are jobs too (`HKPIN`, `HKTAN`, with their D.6 header and `JobDetail::None`); their details are in `Bpd::pin_tan` and `Bpd::tan`. A HITANS above version 7 has its methods read by a remainder parser, with `one_step_allowed: None`.

**Typed sources.** The model reads codec definitions: `HiBpa3`, `HiPins1`, `HiTans1..7`, `HiSpas1..3`, `HiSals5..8`, `HiKazs5..7`, `HiCazs1`, `HiWpds5..6`, `HiCcss1`, `HiVpas1`, `HiIpzs1..2`, `HiIpss1`, `HiVpps1`, `HiUpa4`, `HiUpd4`, `HiUpd6`. HIUPD 5 has no published table: it is read by a remainder parser, so no account is lost. Such an account has an empty `customer_id` and no `allowed` entries.

**Raw fallback (AGENTS.md).** A parameter segment the codec cannot read still yields a `JobVersion`:
- No definition (HIPROS, DIPINS, HISALS 3–4): read the generic D.6 header (DE 2–4) with one shared codec group, `detail: Raw(seg)`. The security class is a guess (a lone digit 0 to 4 after the signature count).
- A definition that rejects the segment: same, plus an entry in `unparsed` with the segment and its `CodecError`. `read::report` logs it.
- Not even a D.6 header: the segment goes only to `unparsed`, and the job does not appear as offered.
- A HIUPD the codec rejects goes to `Upd::unparsed`.

`Bpd::unparsed()` and `Upd::unparsed()` report these entries to the caller, and the capabilities job shows them.

## 3. Construction and update rules

```rust
impl Bpd { pub fn from_segments(segs: &[RawSegment]) -> Result<Bpd, ParamsError> }   // needs a readable HIBPA
impl Upd { pub fn from_segments(segs: &[RawSegment]) -> Result<Upd, ParamsError> }   // needs a readable HIUPA
```

`ParamsError` is `Missing(&'static str)` or `Unreadable(&'static str, Box<CodecError>)`.

The raw sets are the source of truth. The update rules are crate-private pure functions: `bpd_after(current, response) -> Option<Vec<RawSegment>>` and `upd_after(response) -> Option<Vec<RawSegment>>` (`None` = unchanged). Formals C.3.2.2/C.3.2.3:
- A response with HIBPA replaces the BPD completely. The BPD is always sent complete, and code 3050 ("BPD nicht mehr aktuell") is informational. The trigger is HIBPA being present.
- Parameter segments without HIBPA are merged into the BPD, replacing the segment of the same code and version, with a `warn!`. The spec does not foresee this case; merging is harmless.
- A response with HIUPA replaces the UPD completely.
- Code 3072 drops the UPD (`BankParams::invalidate_upd`) unless the response carries HIUPA. This stays with `ingest_response`, because it depends on the response codes.
- A HIBPA the codec cannot read: no model and BPD version 0, so the next dialog fetches the BPD again. A BPD set without HIBPA also leaves no model. The raw set is kept for the trace and the log.
- UPD version 0 (`Upd::is_dialog_only`): the bank sends the UPD for this dialog only. The dialog uses it, but `SessionState::capture` stores version 0 and no UPD, so the next dialog asks again. `SessionState::accounts()` and `upd()` return nothing for such a bank; `Dialog::capabilities` and the jobs read the dialog's own `BankParams`.

## 4. Queries

| Query | Used by |
|---|---|
| `Bpd::job(code) -> Option<&JobParams>`, `Bpd::jobs()`, `JobParams::{offered, highest}` | version choice, capabilities |
| `Bpd::highest_common(code, implemented) -> Result<u16, VersionMismatch>` | `BankParams::order_version` (all jobs) |
| `Bpd::tan_required(code) -> Option<bool>` (HIPINS) | `order_tan()` takes `Option<bool>` |
| `Bpd::tan_methods()`, `TanParams::{hktan_versions, methods}` | `hktan_version`, `select_security_function` |
| `Bpd::sepa() -> Option<&SepaParams>` (highest HISPAS version) | `BankParams::national_account_allowed`; HKKAZ/HKSAL/HKWPD builders |
| `Bpd::sepa_formats(Order)` | pain formats of transfers and instant orders |
| `Bpd::camt_formats()` | `BankParams::camt052_format` (HKCAZ) |
| `Bpd::vop()`, `Bpd::instant()` | payee check and instant payments (payments-design) |
| `JobVersion::{max_orders, min_signatures, security_class}` | `jobs` |
| `Upd::accounts()`, `Upd::find(&Account)`, `UpdAccount::{key, class}` | `jobs::AccountInfo`, `SessionState::accounts` |
| `Upd::allows(account, code) -> Allowance { Yes, No, Unknown }` | capabilities |

`VersionMismatch { code, offered, implemented, formats }` reads `HKSAL: no common version (bank offers 5, 6, 7, client implements 8)`; `offered` is empty when the BPD does not list the job. `BankParams::order_version` turns it into `FinTSError::VersionMismatch`, and into `FinTSError::NoBpd` when no BPD is known.

`Allowance` is `Yes` when the account lists the order, `No` when it does not and the usage is closed, `Unknown` when it does not and the usage is open. It is information only and never blocks an order: UPDs are often incomplete (for example HKCAZ missing although the bank accepts it), and the bank's answer is authoritative. A `banks` profile can opt into refusing before sending.

## 5. Serialisation

`SessionState` stores the raw BPD/UPD wire segments and rebuilds the model on each use (`to_params`, `SessionState::bpd()` / `upd()`; the two getters are public with the `tooling` feature). The typed model is `Serialize` for inspection and the capabilities output, but it is not the stored form.

| | Raw (stored) | Typed |
|---|---|---|
| Round trip | exact; what the bank sent | lossy wherever `JobDetail::Raw` or `unparsed` is used |
| Codec improves | old state reads better at once | stale until the next BPD fetch, or a format bump plus resync |
| Format stability | only the wire format; `SessionState.format` covers the envelope | every model change is a format change |
| Load cost | parse ~50 segments per load (microseconds) | none |

Raw wins because the typed model keeps growing, and each change would otherwise force callers to resync.

## 6. Tests

- **Codec wire tests:** `HiSpas1..3` and the generic D.6 header group, each with `SAMPLES` from the spec examples.
- **Unit tests in `params.rs`:** hand-written BPD/UPD wire strings: several versions of one job, a job without a definition, a definition that rejects its segment, a segment without a header, HIPINS orders, TAN methods per version, highest HISPAS, HIBPA replacement versus merge, UPD usage 0 and 1, dialog-only UPD, an unreadable HIUPD, HIVPPS and instant parameters.
- **Consorsbank trace:** `consorsbank_trace` builds `Bpd`/`Upd` from the recorded sync response (`testbank/fixtures/replay/consorsbank-76030080.trace.json`). The bank sends HIBPA 3, HIPINS 1, HITANS 7, HISPAS 1, HISALS 3–6, HIKAZS 3–7, HIWPDS 2–6, HICCSS 1, HIIPZS 1, HIPROS 3–4, DIPINS 1, and an UPD with HIUPA version 0, usage 0 and seven HIUPD 6 accounts. HISALS 3–4, HIKAZS 3–4, HIWPDS 2–4, HIPROS and the bank's own DIPINS, XIADAS and XIADSS have no definition and are read header-only.
- **Session:** a `session.rs` unit test covers the dialog-only UPD in `capture`.
- **Test bank:** `typed_bpd_and_upd_match_the_bank_model` (`tests/testbank_test.rs`) checks the model built from the synthetic bank's BPD/UPD. The Consorsbank profile serves UPD version 4, not the live version 0.
- **Proptest:** the `BankParams` response proptests also feed the model, which must never panic on arbitrary parameter segments.
