# Trace tooling

A trace is a masked, anonymous record of one dialog with a bank. `fints_client::trace` (feature `trace`) records, masks, reads and summarises traces. `fints_client_testbank::replay` plays a trace back to the client as an offline regression test. Test bank profiles are written by hand with a trace as their evidence. [testbank/README.md](../testbank/README.md) has the commands and the tables (mask fields, recording, `.expect`); this document is the design.

## Rules

- **Traces are anonymous.** BLZ and BIC stay. Account numbers, IBANs, names, amounts, transaction texts (purposes, booking texts) and account product names (HIUPD, HISAL) do not survive. Accounts are named by alias, class, Kontoart and allowed jobs, never by product name.
- **The raw messages are the only truth.** Every other field of an exchange is derived for readers and checked on load.
- **What a trace still shows of an account:** how many accounts of which kind there are; how many bookings each has, how many days lie between them, and for each one whether it is a credit or a debit and what kind it is (card payment, direct debit, standing order: the bank's code); how many lines a purpose has; which bookings carry the same reference, mandate or counterparty account (equal stand-ins); how many holdings a depot has and the code of the exchange each price is from. It does not show names, counterparties or their banks, texts or their length, the size of an amount or whether it is a round sum, whether a balance is negative, or the calendar days: every date is moved by the same 3 to 30 days, which the trace does not record, and every time of day reads 12:00:00. The move is weak protection: days without bookings still show where the weekends are, and whoever knows roughly when a trace was recorded (a commit date) can work the move out. Whoever records a trace decides with that in mind whether to share the file; the summary table alone holds none of it ([adding-a-bank.md](adding-a-bank.md), step 4, has the list for people who report a bank).
- **A trace never reaches the repository unreviewed.** Live recordings stay in `testbank/fixtures/replay/local/`, which git ignores. A human reads a trace before it is committed (section "Before committing a trace").
- **Profiles never reach the network.** Profile tests use the in-process test bank (`TestBank::in_process()`, no socket), so a real BLZ is safe there. The binary serves a profile under the synthetic BLZ 12345678.
- **The log is more masked than the trace.** A trace is a file a human reviews. A log goes to any sink.

## What a trace is

One pretty-printed JSON document per trace, `format` `fints-client-trace/4` (`Trace::to_json`, `Trace::from_json`, files `*.trace.json`).

- `format`, `comment` (lines), `client`, `bank` (`blz`, `bic`), `accounts`, `exchanges`.
- `client` has `version` (the fints-client version that recorded the trace) and `outcome`: `completed`, or the kind of error that stopped the run or a job (`Bank 9010`, `Transport Tls`, `SegmentUnreadable InvalidDate at booked[0]`). A transfer trace (`examples/transfer.rs`) also has `cancelled` (the person stopped at the login or the payee check) and `stopped` (the example ended the run, for example no account may send a transfer). It never holds an error text.
- `accounts` lists each account once: alias (`#1`, `#2`, ...), class, Kontoart, allowed jobs.
- Each exchange has `index`, `account` (alias or null), `retry_of`, `sent` (`jobs` as `CODE:version`, `tan` with `process` and `for`), `answer` (`message`: HIRMG codes with texts; `segments`: HIRMS codes with texts and the job they refer to), `failed`, `error` (transport error or null), `notes`, and `request` and `response` as arrays with one string per segment.
- **Encoding.** Each ISO 8859-1 byte is one Unicode character, so the joined strings are the bytes again. There is no base64, because binary content is masked. HNVSD is split too: its header up to `@len@` is one string, each inner segment has its own, and the closing `'` is a string of its own.
- **Derived fields.** `index`, `retry_of`, `sent`, `answer`, `failed` and `bank` are computed from the raw messages. `from_json` recomputes them and rejects a file whose derived fields differ, naming the exchange and the field. It also rejects unknown fields, another `format`, an `account` that `accounts` does not list, and a character outside ISO 8859-1. `account`, `error` and `notes` are recorded, not derived.
- `notes` say what the client made of an answer, without any value of it: `HIKAZ:7: not read, InvalidDate at booked` for a segment its definition rejected, and `HIKAZ:7: 3 characters outside the SWIFT character set`. The recorder computes them from the answer before masking; the summary table prints them under the exchange.
- `retry_of` is the index of the previous exchange when that one was answered 9075 and this one sends the same jobs again with HKTAN.
- One JSON document, no JSON Lines: scripts and tools read the JSON, people read the summary table.

[testbank/README.md](../testbank/README.md) section "What a trace keeps" has an example.

## Masking

The `Recorder` masks each exchange as it passes, so credentials never sit in memory unmasked. A failed send is recorded with a short, content-free `error`. The mask lives in `src/mask.rs` and `src/mask/`; the table of masked fields per format is in [testbank/README.md](../testbank/README.md).

- **Stars:** PIN and TAN are always eight stars, so their length does not show. One `*` per character for subaccounts, the TAN challenge, the HKVVB product ID, every unknown segment after its header (section "Unknown segments"), and every binary element that is not a statement, a pain message or an ID (keys, certificates).
- **Shape masking** (`src/mask/shape.rs`): values lose their content and keep what a parser needs to see.
  - An amount is `#,##` (`#.##` in XML) whatever its size and however many decimals were sent (`1234,56`, `100,` and `0,5` all become `#,##`): neither its size nor a round sum shows. A count is `#`. The debit mark of a balance (MT940 `:60F:` to `:65:`, camt `Bal/CdtDbtInd`, HISAL) is written as credit, so an overdrawn account does not show; the mark of a booking stays.
  - A name (holder, user, product, counterparty, security, address line) is eight `#` whatever its length; a name longer than its format allows is limit + 1 characters, so a bank that breaks the limit stays visible.
  - Other free text (purposes, booking texts, BICs, ISINs) is eight `#` for each run of it, up to the next piece of markup or line break, whatever its length; a run of spaces is one space. The length of a purpose would link a booking to its counterpart in another account or another trace. What stays is the layout: MT940 subfield codes and line breaks, the number of `Ustrd` elements of a camt entry.
  - Dates move: every date of a trace is moved by the same number of days, 3 to 30 back, drawn per recorder and written nowhere (`src/mask/dates.rs`). This covers statements (MT940 `:60F:` to `:65:`, `:61:` value and entry date, `:13D:`; camt `Dt`, `DtTm`, `CreDtTm`, `FrDtTm`, `ToDtTm`; MT535 `:98A:`, `:98C:`, `:69A:`, `:70E:`), the period of HKKAZ, HKCAZ and HKWDU, the date of a HISAL balance, the validity of a VOP-ID and the security timestamp of HNSHK and HNVSK. A time of day beside such a date is written as noon and a UTC offset as zero (`2026-01-02T10:15:30+01:00` → `…T12:00:00+00:00`): traces recorded in one sitting would otherwise show it by their clock times.
  - Markup and codes stay: MT940 tags, `?nn` subfield codes, SEPA keywords, the `:61:` mark and transaction type, GV codes, currencies, dates, camt element and attribute names and code values. There is one list of masked fields per format: MT940 (HIKAZ), camt (HICAZ), MT535/536 (HIWPD, HIWDU), HISAL, HIUPA and HIUPD. `@len@` is the length of the shaped data.
- **Aliases and keyed pseudonyms** for identity values (section "Aliases and keyed pseudonyms"): per-trace aliases for account identifiers, keyed pseudonyms for references and IDs. Dialog IDs and order references are numbered per trace (see "What does not get a pseudonym").
- **Anonymised by `Trace::from_exchanges`:** dialog IDs and order references become `DIALOG0001`, `TASK0001`, ...
- **Statements are shape-masked by the trace only** (`mask::mask_for_trace`). The log (`mask_for_log`, `redacted_text`, `debug`) stars them. Names, product names and amounts in HIUPA, HIUPD and HISAL are shape-masked in both.
- **Leak check.** `examples/record_trace.rs` searches the finished trace for the user ID, PIN, product ID and every IBAN, account number and holder name of the run (values of four characters or more; each word of a name on its own). It ignores case, line breaks and the `?nn` subfield codes of MT940, so a value the bank wrapped is found too. If one is found, it writes nothing. `examples/transfer.rs` does the same for its payment data and keeps the trace as `<name>.leak` beside the target.

## Aliases and keyed pseudonyms

A trace replaces every identity value by a stand-in. The user's own account identifiers and the IDs of a dialog keep their length and character classes; what comes from other people has one fixed form each, because the length and pattern of such a value names its issuer: a counterparty's IBAN is always `DE`, check digits, bank code `00000000` and ten digits; a creditor ID always `DE`, check digits, `ZZZ` and eleven digits; a booking reference, end-to-end ID, message or payment information ID and mandate reference always twelve characters of `0-9A-Z` (`NOTPROVIDED` and `NONREF` stay). Equal values get equal stand-ins within a trace, so a trace answers questions such as "is this booking reference unique", "does the pending entry keep its reference once booked", "does the bank echo our end-to-end ID", "does the order's account equal the UPD account" and "does the client send back exactly the VOP-ID or polling ID the bank sent", without holding any of the values. There are two kinds of stand-in:

- **Aliases** for account identifiers: IBAN, account number, user and customer ID, creditor ID. An alias is a number given in order of first appearance in this trace; it has no mathematical link to the value, and it links to no other trace.
- **Keyed pseudonyms** for references and IDs: booking reference, end-to-end ID, message and payment information ID, mandate reference, instant order ID, VOP-ID, polling ID, system ID. A pseudonym is computed from the value and a salt, so traces recorded with the same salt give a reference the same pseudonym.

Code: `src/mask/pseudonym.rs` (`Identities`, one per recorder), applied by `mask::mask_for_trace` (text elements, binary IDs, feedback texts) and `src/mask/shape.rs` (statements and pain messages).

**Which values get a stand-in, and their classes.** The same thing gets the same class wherever it occurs.

| Class | Stand-in | Where |
|---|---|---|
| own IBAN | alias of the account part | KTI (HKSAL, HKKAZ, HKCAZ, HKTAN, HKCCS, HKIPZ, HKIPS and the answers), KTZ (HISPA), HIUPD 2; MT940 `:25:`; the statement's own account in camt (`…/Acct/Id/IBAN`); the debtor account of a pain.001 |
| IBAN of anyone else | alias of the whole BBAN | HIVPP payee IBAN, IBAN-like words in HIRMG/HIRMS texts; MT940 `:86:` `?31` (also when the bank wrapped it over two lines) and `IBAN+`; every other camt and pain `IBAN` |
| account number | alias | KTV, KTI and KTZ number; MT940 `:25:` and `:86:` `?31` when not an IBAN; MT535 `:97A:` depot number; camt `…Acct/Id/Othr/Id` |
| user ID | alias | user and customer ID: HKIDN 2, HNSHK and HNVSK key name, HIUPA 1, HIUPD 3 |
| creditor ID | alias | `CdtrSchmeId/…/Id`; MT940 `CRED+` |
| system ID | pseudonym | HKIDN 3, HNSHK and HNVSK security identification, HISYN 1 |
| booking reference | pseudonym | camt `AcctSvcrRef`, MT940 `:61:` bank reference (after `//`) |
| end-to-end ID | pseudonym | pain and camt `EndToEndId`, pain.002 `OrgnlEndToEndId`, MT940 `EREF+` and `:61:` customer reference (other than `NONREF`) |
| message ID, payment information ID | pseudonym | pain `MsgId`/`OrgnlMsgId` (and a camt's own `MsgId`), `PmtInfId`/`OrgnlPmtInfId` |
| mandate reference | pseudonym | `MndtId`; MT940 `MREF+` |
| order ID | pseudonym | instant payment order ID: HKIPS 3, HIIPS 4, HIIPZ 1 |
| VOP-ID, polling ID | pseudonym | binary: HIVPP 1 and 3, HKVPA 1, HKVPP 2 |

An MT940 `EREF+`, `MREF+`, `CRED+` or `IBAN+` value is read in the purpose subfields (`?20` to `?29`, `?60` to `?63`) as one text up to the next SEPA keyword, so a value split over two subfields gets one stand-in, written back over both.

**What gets no stand-in.** Amounts, names, purposes, booking texts and other free text are shapes (`#`, see "Masking"): they have few possible values, so equal stand-ins would show patterns. Dates are moved and times of day are noon (see "Masking"). BICs and bank codes stay as they are in KTI/KTV and are text (`########`) in statements. PIN, TAN, the TAN challenge, subaccounts, the CID, the VoP touchdown point, the HIVPP payee name, other ID and reasons, keys, certificates, order hashes and every unknown segment are starred. Dialog IDs and order references become `DIALOG0001`, `TASK0001`, … per trace (`Trace::from_exchanges`): they live for one dialog, so the trace shows whether the client sends back the value the bank sent, and nothing links them across traces. The log (`mask_for_log`, `redacted_text`, `debug`) has no stand-ins: it stars all of these.

**Aliases.** There are three numberings: accounts (IBAN and account number share one), users (user and customer ID), creditors. Each starts at a random number drawn per recorder; the n-th new value of a numbering gets start + n, written from the right over the value's ASCII letters and digits (a digit in base 10, a letter in base 26, case kept); every other character stays. What does not fit is dropped, so a value with very few letters and digits can share an alias with another. The table from value to number lives only in the recorder's memory and is never written. Two recorders, even with the same salt, give the same value unrelated aliases.

- **Account number**: leading zeros stay; the rest is the key, and its first character is not `0`. So `0001234567` (KTV) and `1234567` (MT940 `:25:`) of the same account get the same digits after the zeros: `0004821937` and `4821937`.
- **Own IBAN**: the country code and the bank part of the BBAN stay (the bank code is in clear in the KTI anyway; DE, ES, PL: 8 characters; AT, CH, LI: 5; NL: 4; BE, LU: 3; FR, MC, GB: 10; IT, SM: 11; other countries: none), the account part gets the account number's alias, and the check digits are computed again (ISO 13616 mod 97). `DE89370400440532013000` becomes for example `DE3537040044` + `0` + `482193705`: a valid IBAN whose account part equals the alias of the same account's KTV number `0532013000` (`0482193705`). A value in an IBAN place that is not of the IBAN form gets an account number's alias.
- **IBAN of anyone else**: only the country code stays. The whole BBAN gets an alias and the check digits are computed again, so the trace does not say at which bank a counterparty is. An IBAN that is the user's own account at another bank is a counterparty here.
- **User ID**: every letter and digit is replaced: `testuser1` becomes for example `qbmfzxkd4`.
- **Creditor ID**: the country code and the business code stay, the national identifier gets the alias, and the check digits are computed again (mod 97 over the national identifier, the country and `00`; the business code does not count): `DE98ZZZ09999999999` becomes for example `DE91ZZZ04718203946`.

**Pseudonyms.** For a value `v` of class `c`: `k = HMAC-SHA256(salt, c ‖ 0x00 ‖ v)`, then the key stream `HMAC-SHA256(k, 0) ‖ HMAC-SHA256(k, 1) ‖ …` (32-bit big-endian counter). Each ASCII digit of `v` becomes the next stream byte below 250 modulo 10, each upper-case letter the next byte below 234 modulo 26 (upper case), lower case likewise; every other character (`-`, `.`, `/`, space, non-ASCII letters) stays. Each character depends on the whole value, so the pseudonym of a prefix says nothing about the pseudonym of the whole value. Two different values of the same pattern collide with probability one in the number of values of that pattern (far less than 10^10 for a four-digit reference).

- **System ID** `0` (not yet synchronised) stays `0`.
- **SEPA references and IDs** keep their character set: only letters and digits are replaced, by letters and digits of the same case.
- **Binary IDs** (VOP-ID, polling ID) that are not printable ASCII are mapped byte by byte over all 256 values.
- Lengths never change, so `@len@` headers and field length limits stay valid.

**Salt.** 32 bytes. `Recorder::new()` draws one from the operating system's random number generator, keeps it in memory and drops it with the recorder: the trace's pseudonyms link to no other trace. `Recorder::with_salt(salt)` takes a salt from the consumer, who keeps it, so the references of its traces link to each other. `examples/record_trace.rs` and `examples/transfer.rs` use `FINTS_TRACE_SALT` (64 hex digits, from the environment or the git-ignored `.env`, see `.env.example`) when it is set, else `Recorder::new()`: a developer who sets it can compare traces recorded on different days. The library writes no salt. A leaked salt reveals references only: account identifiers are aliases, which no salt reverses.

**Replay.** The replay needs no salt and no alias table: an alias or pseudonym is a valid value and is served as recorded (the client logs in with the replay's own user ID; the UPD, statements and IDs are the recorded aliases and pseudonyms). A value that is all stars is filled with a synthetic one. The replay tells stars from aliases and pseudonyms per value, so no format marker is needed.

**Threat model.** Account identifiers have little entropy: a German IBAN is the bank code, which is in clear, plus ten digits, and user IDs and creditor IDs are short. Any function of such a value can be inverted by trying every value, so an account identifier is never written as a function of itself: its alias is a counter from a random start, and the table that links the two is never written. A trace therefore says nothing about an account identifier beyond its length, its character classes, and (for an IBAN) its country and bank code; neither brute force nor a membership test ("is IBAN X in this trace?") is possible, with or without any salt. References and IDs keep keyed pseudonyms, safe only while the salt is secret: with the salt, anyone holding a trace can test candidate references. With `Recorder::new()` the salt is gone when the recording ends. A consumer that passes a salt keeps it out of traces, logs and repositories; if it leaks, it reveals reference values only (booking references, end-to-end IDs, mandate references, …), never an account identifier, a user ID or a creditor ID. Traces made with different salts cannot be compared.

**Analysis.** `cargo run -q --features trace --example trace_identities -- testbank/fixtures/replay/local/*.trace.json` (`fints_client_testbank::analysis`) reads one or more traces and reports booking references that repeat within one statement of an account (one trace), references of pending entries in one trace that are on a booked entry in another trace, and end-to-end IDs sent in a pain.001 (HKCCS, HKIPZ) that come back in a pain.002 (HIVPP, HIIPS) or on a statement entry. Accounts are named by their alias in their own trace and are never matched across traces; references match across traces by the reference alone, and only between traces recorded with the same salt (`Recorder::with_salt`), so traces of `record_trace` and `transfer` link across runs only when `FINTS_TRACE_SALT` was set for all of them. It prints counts, aliases and pseudonyms only; values that are shapes or stars (traces without stand-ins) are counted and never compared.

## Unknown segments

A segment the library has no field list for is what a new bank adds, and nobody knows what it holds. A trace therefore shows only its header and the length of each element, as stars, unless a person looked at it:

- The `Recorder` holds such segments as sent until a trace is taken. `Recorder::unknown_segments` lists them, one entry per code and version with the first one as an example.
- `examples/record_trace.rs` and `examples/transfer.rs` print each example and ask whether the trace may keep it readable. The answer is per segment code; the default is no. Without a terminal nothing is kept.
- `Recorder::trace_keeping(codes)` writes the chosen codes as sent, with no masking at all. `Recorder::trace` keeps none.
- A HIUPD in a version without a field list is never offered: it holds the account and its holder.

## Summary and account aliases

`Trace::summary` gives one `ExchangeSummary` per exchange. `Trace::summary_table` renders a legend line, the client version and run outcome, the accounts, then one line per exchange with its `notes` below it: jobs, HKTAN, return codes, `retry of #NN`, the alias, and `FAILED` on a transport error or any 9xxx code. The summary reads raw segments and never fails: a message that does not tokenize prints as unreadable.

- `examples/trace_summary.rs <file>...` prints the table offline. `examples/record_trace.rs` prints it after every run, successful or not, and the account next to each failed fetch.
- **Aliases come from the recorder, before masking.** After masking the identifiers cannot be matched. The recorder reads each unmasked HIUPD and numbers the accounts in order of first appearance. It reads the account element (data element 1, KTI or KTV) of HKSAL, HKKAZ, HKCAZ, HKWPD, HKWDU, HKSPA, HKCCS and HKIPZ, and takes the first account it knows.
- The recorder keeps only keyed hashes: IBAN, and number without leading zeros with subaccount and BLZ. The key is random per recorder and never written.
- `class` is the Kontoart's class, or `depot` when there is no Kontoart and the jobs allow HKWPD and none of HKSAL, HKKAZ, HKCAZ. It is null otherwise.
- Lookup is limited to those positions of those jobs, so a date that looks like an account number is never a hit.


## Replay

`fints_client_testbank::replay` plays the bank side of a trace back to the client over `replay::standard_run`, the same function `record_trace` runs. `tests/replay_test.rs` replays every `fixtures/replay/*.trace.json`; `*.transfer.trace.json` is skipped, because the standard run sends no payment. With `REPLAY_LOCAL=1` it replays `local/` too.

- **Structure, not values.** For each request the replay compares segment codes in order, versions, numbers, the count of data elements and group elements, and for HKTAN the process and the segment it refers to. Values are stars, shapes or change per run. A difference is a `Violation`; the recorded answer is sent all the same, so one run shows every difference.
- **Valid data from shapes.** A shape-masked value is made valid again (`trace::unshape_text`, `trace::unshape_statement`): `#` becomes `1` in amounts and `X` in text, so the client parses the bank's own layout with as many entries as were recorded. Starred values are filled from the client's own request or with a synthetic value; testbank/README.md lists which.
- **Expected differences.** When the client's requests change on purpose, the trace stays as recorded, because it is evidence of what the bank answered to the old request. A sidecar `<name>.expect` lists the expected differences, one `Violation` per line, with a first comment line that names the reason. The test compares the actual violations with the sidecar as an exact multiset. A new difference fails, and an expected one that went away fails too, so a sidecar never goes stale. The exchange count, their order and the success of the run are always checked.
- **Bless.** `REPLAY_BLESS=1 cargo test --test replay_test` rewrites the sidecars from the actual violations and keeps comment lines. A sidecar of a clean run is deleted. It refuses when the run went out of step (`the client stopped after`, `the client sent one more`, or more `segments differ` lines than job exchanges): move that trace to `fixtures/replay/stale/` (not replayed) and re-record it. A bless commit explains each changed line, because a real regression looks the same.
- To clear a sidecar, re-record to the same path (`record_trace` reminds you when `<out>.expect` exists) and delete the sidecar once the new trace replays clean.

## Test bank profiles from traces

A profile is a `BankModel` in `testbank/src/profiles.rs` (`consorsbank()`, `dkb()`, `comdirect()`), written by hand. The design of the profiles is in [bank-profiles-design.md](bank-profiles-design.md).

- **Hand-written, with the trace as evidence.** A generator would have to infer rules (the check order, what provokes a 9010) from one masked run. Those are judgments, not data. The replay already plays back the literal recording.
- **Evidence test** (`tests/bank_profiles_test.rs`): for each profile with a trace, the profile's BPD and UPD must agree with the trace's first BPD and UPD answers (HIPINS flags, job versions, HITANS, HISPAS flag, allowed jobs and Kontoart presence per account). Parameter data is not masked, so this works on the masked trace. The test reads the committed fixture or, under `REPLAY_LOCAL=1`, `local/<bank>-<BLZ>.trace.json`, and skips a bank whose trace is absent.
- **Behaviour tests** run `standard_run` against a profile and check what the client sends and learns.
- **Rules without evidence** carry an `unverified` doc comment.
- Bank-specific behaviour is in the profile and in [banks.md](banks.md), never in a branch on the bank code in `protocol` or `jobs`.
- The binary serves a profile under the synthetic BLZ 12345678: `fints-testbank --profile consorsbank`.

## Where traces live

- `testbank/fixtures/replay/*.trace.json`: committed, replayed by default. These are `synthetic-decoupled.trace.json` (from the synthetic test bank) and the read traces of DKB, comdirect and Consorsbank: `dkb-12030000.trace.json`, `comdirect-20041155.trace.json` and `consorsbank-76030080.trace.json`. The transfer recordings beside them (`*.transfer.trace.json`, and `dkb-12030000.transfer.cancelled.json`, cancelled at the payee check) are evidence for the bank pages and are not replayed.
- `testbank/fixtures/replay/local/`: git ignores it. Live recordings (`scripts/try-bank.sh`, with `--transfer` for a payment) and failed runs (`<bank>-<BLZ>.failed-<time>.json`, kept beside the trace, never replacing it, not replayed) are here. A trace from a user, for example a diagnostic trace from an app, is never committed.
- `fixtures/replay/stale/`: traces that went out of step; not replayed.

## Before committing a trace

A human does this; a trace is its recorder's private data until it passes.

1. Run `trace_summary` on it and read the story: jobs, codes, `FAILED` lines.
2. Grep the file for your name, IBAN and account number fragments, and amounts.
3. Read every HIRMG and HIRMS text. They stay as the bank sent them, and a bank may put a name or an amount there.
4. Check that unknown segments are starred.
5. Run `REPLAY_LOCAL=1 cargo test --test replay_test`. Add an `.expect` sidecar only with a reason line.
6. Move the file from `local/` to `fixtures/replay/`. Give a failed or cancelled run a name without its recording time (`<bank>-<BLZ>.transfer.cancelled.json`): the time in the name would undo the moved dates.

A trace recorded with earlier masking rules is masked again first: `cargo run --features trace --example trace_remask -- <files>` (`Trace::remask`) applies the current rules to what the file holds, makes new stand-ins and moves the dates once more.
