# Version negotiation: design

Rule from [architecture.md](architecture.md) §2.2: send the highest version both the BPD and this client offer, or fail with an error naming both sides. No clamping, no guessed version. The jobs and capabilities built on it are in [jobs-step-design.md](jobs-step-design.md).

Spec basis: Formals C.10 (BPD list every supported version of an order; use the highest common one; this applies to orders only, not to administrative segments), Formals D.6 (one parameter segment per version), Formals "Segmentversion" (order, response and parameter segment share one version, so the HITANS version of a TAN method is its HKTAN version). Bank-side counterpart: return code 9180 ("Auftragsversion ist veraltet").

## 1. How versions are chosen

| Segment | Version | Source |
|---|---|---|
| HKIDN 2, HKVVB 3, HKSYN 3, HKEND 1 | fixed | `Segment::encode` (`src/protocol.rs`); administrative, one version each (Formals C.10) |
| HKTAN 5-7 | method's HITANS version | `BankParams::hktan_version`, `init_hktan_version` |
| every other order | highest common | `BankParams::order_version(Order)` |

`Order` (`src/segments/mod.rs`, `#[non_exhaustive]`, `fints_client::Order`) lists the orders whose version comes from the BPD: balance (HKSAL), transactions (HKKAZ), camt (HKCAZ), holdings (HKWPD), depot transactions (HKWDU), transfer (HKCCS), instant transfer (HKIPZ), instant status (HKIPS), payee check (HKVPP) and its release (HKVPA). `Order::code()` is the segment code. `Order::implemented()` is the explicit ascending list of versions the builders in `segments::request` can build. `Order::ALL` lists every order (test builds only).

## 2. Orders

`BankParams::order_version(order) -> Result<u16, FinTSError>` is pure on the parameters and needs no dialog. It calls `Bpd::highest_common(code, order.implemented())`. Every `Segment::encode` arm for an order calls it, so a request is never built with a guessed version.

- No BPD: `FinTSError::NoBpd { code }`. Orders run only in a dialog opened after synchronisation, so this is a bug or a broken bank, and a guess would be clamping by another name.
- No common version, or the order is not in the BPD: `FinTSError::VersionMismatch(VersionMismatch { code, offered, implemented, formats })`. `offered` is empty when the bank does not offer the order. `formats` names both sides for a data format (pain.001) instead of an order version.

A builder asked for a version `implemented()` does not list returns `FinTSError::Internal` ("HKSAL v9 has no builder"). It is a bug, not a user error.

## 3. HKTAN

The HITANS version of a TAN method is its HKTAN version.

- `BankParams::hktan_version() -> Result<u16, FinTSError>`: the highest version in `request::HKTAN_VERSIONS` (5, 6, 7) among the HITANS versions that list the selected security function. There is no fallback to another method's version, since its HITANS does not list the selected method. When none is buildable: `VersionMismatch` with `code = HKTAN`, `offered` the HITANS versions listing the method (from the full BPD, including versions we cannot build), `implemented` our list.
- No HITANS known (first dialog, before any BPD): 7, the highest implemented. The committed Consorsbank trace (live 2026-11-16) sends `HKTAN:5:7+4+HKIDN` at dialog init and the bank accepts it.
- `BankParams::set_bpd` keeps only methods from buildable HITANS versions in `tan_methods`. Method selection (`select_security_function`) skips a function no remaining method lists and ignores a stored preference for it. 999 is exempt, and with no HITANS everything stays open. `hktan_version` is the backstop.
- `BankParams::init_hktan_version() -> Result<Option<u16>>`: the dialog-init message allows HKTAN 6 and 7 only (PIN/TAN B.4, "Dialoginitialisierung": "ein Segment HKTAN ab Segmentversion #6"). A selected method only in HITANS below 6 gets no HKTAN at init. A pin-only selection (999) with HITANS known has no method to signal and also gets none. Orders still use the method's own version.

## 4. Addressing

The negotiated version decides national (KTV) or international (KTI) account details (Messages B.3). An account the chosen version cannot address is `FinTSError::AccountNotAddressable { code, version, reason }` with `AddressingReason::{NoNationalDetails, NoIbanBic}`. A lower version is never tried.

## 5. Errors and callers

`VersionMismatch` and `AccountNotAddressable` both have `Category::Unsupported` (`FinTSError::kind()`; see [return-codes-design.md](return-codes-design.md)), as has the bank's own 9180. `VersionMismatch` and `NoBpd` are client-side preconditions, never a bank code.

`jobs` call `order_version` before sending: `NoBpd` or `VersionMismatch` fails the job before a message goes out, so no TAN is spent on a job that cannot work. A UPD `No` never blocks. With `Format::Auto`, a `VersionMismatch` for camt selects MT940. `NoBpd` and `AccountNotAddressable` reach the caller. The capabilities value calls `order_version` per `Order` to report "bank offers X, client implements Y" without a dialog.

A per-job maximum version in a `banks` profile (for example a TAN generator that reads only an older HHD) is not part of the design; add it when a bank needs it.

## 6. Tests

- `protocol` unit tests per `Order`: overlap (highest common chosen), no overlap (error lists both sides), order absent from BPD (empty `offered`), no BPD (`NoBpd`).
- HKTAN: selected method in HITANS 6 and 7 gives 7 (not another method's version); only in an unbuildable version gives `VersionMismatch`; no HITANS gives 7; init version per method.
- `segments::tests::implemented_versions_match_the_builders`: every `implemented()` version builds with the right header, the next one is `Internal`, and codec-backed versions are registered. `registry_versions_without_builder_are_not_claimed` pins HKSPA 1 as registered but not an order.
- Test bank (`tests/testbank_test.rs`): a model whose BPD offers HISALS, HIKAZS or HICAZS only in versions the client lacks. E2e tests assert the typed `VersionMismatch` from `Dialog::balance` with no HKSAL on the wire, and that a camt fetch falls back to MT940 with no HKCAZ on the wire.
