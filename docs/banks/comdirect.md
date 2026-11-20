# comdirect

Part of the [bank list](../banks.md). Anything not confirmed by a live run or the bank itself is marked **unverified**.

Bank codes 20041111, 20041133, 20041144, 20041155, 20041166, 20041177, 20041188 and 20041199 (the bank directory lists these eight, each with its own BIC; one per login, see below), server `https://fints.comdirect.de/fints`, BIC `COBADEHDXXX`.

## Logins

- **One customer can have several logins**, each with its own clearing account (Verrechnungskonto) and depot. Each login belongs to one bank code, and two logins can share a code.
- State stored per bank code would mix two logins at one code. Store session state per login, not per bank code.

## Login and SCA

- Methods 901 *mobileTAN-Verfahren* and 902 *photoTAN-Verfahren* (HITANS 6 only, live trace); 3920 allows 902. In fints-client's run (live 2026-11-17) the login got `3076` and so did HKSAL and HKCAZ, each sent with `HKTAN:6`; no app approval was asked. Whether the bank requires that HKTAN is open.
- A wrong PIN gives `9942 PIN ungültig`. Typing the credentials of another login looks exactly like a wrong PIN.
- comdirect's bank codes span 20041111–20041199. A code that does not belong to the user gives, in one HIRMG of the sync, `9050`, `9800`, `9210 Bankleitzahl passt nicht zum Benutzer.` and `9340 Ungueltige Auftragsnachricht: Ungueltige Signatur.` (seen live 2026-11-17). The client reports 9210, the cause; 9340 is its consequence ([return-codes-design.md](../return-codes-design.md) section 3).

## Account list (UPD)

- **No account type** in the UPD. The depot can only be recognised by its allowed jobs: HKWPD and none of HKSAL/HKKAZ/HKCAZ (generic rule `AccountInfo::class`, no profile entry).
- **The clearing account and the depot share IBAN and account number**; only the subaccount differs. On a first sync the depot had no IBAN. Keyed by IBAN, the two accounts would overwrite each other. fints-client keys them by the KTV, subaccount included (`AccountKey`, generic, no profile entry); the UPD allowance and the camt/MT940 choice come from each account's own entry (test `accounts_sharing_iban_and_number_stay_apart`, offline).
- BPD job versions are old: HISALS v5, HIKAZS v5, HICAZS v1. HICAZS offers `camt.052.001.02` and `camt.052.001.02.xsd`; the bank answers with descriptor `camt.052.001.08` and a document in the `camt.052.001.08` namespace, a version its BPD does not list (live 2026-11-17, exchange 4). fints-client requests `…camt.052.001.02` (highest version, plain descriptor over `.xsd`); parsing is by element name, and a differing version is logged. Both forms get that answer: the `.xsd` form (live 2026-11-17) and the plain `.02` descriptor (live 2026-11-20: HKCAZ 0020, descriptor and namespace `camt.052.001.08`). The proprietary DIKKUS/DIKKSS/DIKAUS are offered.

## Balances

- Clearing account: booked balance only. No available balance, no credit line.

## Transactions

- **Storage period:** HIKAZS v5 540 days, HICAZS v1 9999 days (live trace). The library clamps an older `from` to it (`ClientWarning::RangeClamped`).
- The clearing account's bookings come as CAMT (HKCAZ) but are thin: no counterparty, no booking text, no business transaction code, no references, no purpose code. Only dates, amount, a free-text purpose of a few lines and the bank's booking reference (entry-level `AcctSvcrRef`, plus an `NtryRef`; trace 2026-11-17, values masked). The bookings seen look like securities settlements and similar bank-internal postings (**unverified**), so the purpose text is the only description.
- **Pending bookings** come in the HICAZ pending document (live 2026-11-19: a transfer sent the same evening). The entry has `Sts` `PDNG`, amount, counterparty name and IBAN and `Ustrd`, but **no `BookgDt` and no `ValDt`**; fints-client dates it with the report's `CreDtTm` day.
- The purpose comes as several `Ustrd` elements of differing length (25 to 35 characters seen): they are lines, not fragments of one line. fints-client joins them with a line break.
- Live 2026-11-19: 11 entries (10 booked, 1 pending), the balance and 19 holdings were read. The clearing account comes without Kontoart and its product name fits no account type.

## Holdings

- HKWPD returns MT535 and works. Every position has ISIN, WKN, name, quantity, price with currency and date, value and purchase price.

## Credit cards

- **Not read.** The BPD offers the bank's own DIKKUS (credit card transactions); per a user report both comdirect Visa cards are readable that way (**unverified**). The segment has no public specification and fints-client does not send it. A layout would have to come from a trace of an own card. Open: whether the UPD lists the card (Kontoart 50) and which jobs it allows.

## Return code 3905

- `3905` is outside the spec's list (3900–3999 is for the bank's own codes). Text seen: "Bitte senden Sie die Anfrage erneut". It came next to `3945` in our transfer trace and next to `3010` in a user report. It is a generic hint: the library passes it on as a notice and never retries on it.

## For the bank profile

- Profile entry: id only (`src/banks/table.rs`, all eight codes); no behaviour fields, no user ID rule (nothing to complete).
- No booking ID: the camt `AcctSvcrRef` is kept in `bank_reference`, but whether it is unique and stable is unknown, so `booking_id` is `None` (generic `BookingIdSource::None`).
- Offline model: `testbank/src/profiles.rs` (`comdirect()`, code 20041155); behaviour tests `comdirect_standard_run_takes_the_depot_from_its_allowed_jobs` and `comdirect_wrong_bank_code_is_reported_as_9210` in `tests/bank_profiles_test.rs`.
- Account identity: `AccountInfo::key` (KTV with subaccount), never IBAN alone; generic, not a profile entry.
- Account type by allowed jobs when the UPD has none, and balances only where HKSAL is allowed: generic, from the UPD (`Capabilities`, `standard_run`).
- Clearing-account bookings need a display fallback (purpose text) since counterparty and booking text are empty: the caller's job.

## Payments (live 2026-11-18, 1.00 EUR test)

- **Reads:** login with `HKTAN(4, HKIDN)` gets `3076` (no SCA). `HKSAL:5 + HKTAN` and `HKCAZ:1 + HKTAN` get `0020` on the order and `3076` on HKTAN. `HKWPD:5` goes without HKTAN and gets `0020`, with 19 holdings. Every answer carries HIRMG `3060`.
- **Transfer (HKCCS):** message 1 `HKVPP:1 + HKCCS:1 + HKTAN(4, HKCCS)` gets HIRMG `3060` and `3905`, `3090` on HKVPP and `3945` on HKTAN; the pain.002 result is `RVMC` (close match). Message 2 `HKVPA:1 + HKCCS:1 + HKTAN(4, HKCCS)` gets `0020` on HKVPA, `0010` on HKCCS and `3076` on HKTAN: the order was executed **without any TAN**. Receipt status `Executed`.
- The missing TAN was observed twice: 2026-11-18 for a payee the user may have known (own name or an earlier payee, **unverified**), and 2026-11-19 for a 1.00 EUR transfer with a clean match (message 1 `HKVPP:1 + HKCCS:1 + HKTAN(4, HKCCS)`: `0020` on HKVPP, `3091` and `0010` on HKCCS, `3076` on HKTAN; executed in one message). The sending account is a Verrechnungskonto that can only pay to its reference account (Auszahlungskonto), which is the likely reason (account holder, 2026-11-19); a comdirect Girokonto paying another payee is not yet seen. A wrong TAN could therefore not be tried at 1.00 EUR. The library handles it generically: `3076` on HKTAN after HKVPA ends the transfer without a TAN step.
- **Not seen yet:** `RVNM`, `RVNA`, `RCVC`, an instant payment (HKIPZ), a transfer to a payee other than the reference account. Per a 2026-02 user report the latter asks for photoTAN, an image challenge (**unverified**); fints-client reads it as `TanMedia::Image`, untested live.
- HKCCS 1 with pain.001.003.03 only (also with the `.xsd` file name); HKIPZ 1 with pain.001.001.09 and .003.01 and purpose codes; HIIPSS minimum wait 99 seconds. HIPINS: HKCCS and HKIPZ need a TAN, HKIPS does not, HKVPP and HKVPA are not listed.
- Payee check (HIVPPS): delivery `Stepwise`, structured explanation text (markup allowed), report descriptor `sepade.pain.002.001.10.xsd`, required for 9 orders including HKCCS and HKIPZ.
- **Unobserved; model assumes** that a clean match (`RCVC`, so no HKVPA round) with an HKCCS that needs no TAN also executes at the HKVPP message without a TAN, the same as after HKVPA. Only the close-match path was seen live.
- Offline model: `profiles::comdirect()` (`message_notice` 3905, `vpa_answer` 0020, `order_answer_after_vpa` 0010 on HKCCS, HKCCS exempt); test `comdirect_close_match_executes_without_tan` in `tests/payments_test.rs`.
