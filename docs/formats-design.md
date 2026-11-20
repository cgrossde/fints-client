# MT940 and SEPA tags from `:86:`

Source: DFÜ-Abkommen Anlage 3, Spezifikation der Datenformate, V3.5 (07.04.2021), Teil 5 MT94x, §5.1.4 "Strukturierte Belegung des Feldes 86", §5.1.5 (example) and §5.2 (MT942, same field 86), and Teil 6 (GVC). V26.11 (in `spec/`) drops the MT940/MT942 chapter (its chapter 8, "entfallen") and points to V3.8 in the Anlage 3 archive, the last version with it; V3.8 is not in `spec/` ([spec-sources.md](spec-sources.md)). The statement fields come from HBCI 2.2 IX.2.8 (MT940) and IX.2.9 (MT942).

## 1. The parser

`src/formats/mt940.rs` is our own MT940 (booked) and MT942 (pending) parser. `parse_mt940` and `parse_mt942` decode the bank's bytes as Windows-1252 and build one `Transaction` per `:61:`.

- **Statements:** each field starts a line with its tag. Lines end in CR LF, LF or `@@` (some banks write `@@` for the line break inside FinTS binary data, as in MT535). Lines without a tag continue the field before. Field order is not checked, unknown tags are skipped, and separators (`-`, `--`, a `-` glued to the next tag) and blank lines are skipped. Data of separators and blank lines only has no statements and is not an error; other text without a field is.
- **Pages:** the job joins the touchdown pages byte for byte before parsing (banks split at any byte, also inside a field), so a statement may span pages and a page may carry several `:20:` statements. No `:62F:` is needed.
- **Fields read:** `:60F:`/`:60M:` (currency), `:61:`, the `:86:` after each `:61:`, and for MT942 `:34F:` (currency when there is no opening balance), `:13:`/`:13D:` (creation time) and `:90D:`/`:90C:` (count and sum, checked against the bookings; their currency is the last source when there is neither opening balance nor `:34F:`).
- **MT942 (HBCI 2.2 IX.2.9):**
  - `:34F:` once (empty or `D` mark) or twice (second with `C`): only its currency is read. It is the smallest reported amount, not a filter, so it is not checked.
  - `:13:` (HBCI 2.2, `JJMMTThhmm`) and `:13D:` (SWIFT, `YYMMDDhhmm±hhmm`) are read in either form whatever the tag; the year is 20YY. The earliest time of the statements is the list's time. An unreadable time is `None` with a warning; the bookings still count.
  - `:90D:`/`:90C:` (both optional): the count decides completeness. A side whose count differs from its `:61:` bookings fails the call (`Pending::Unreadable` in the job). HBCI 2.2 does not say where reversals count: SWIFT counts RC as a debit entry and RD as a credit entry (the effect on the account), a bank may count them by the letter of the original booking; the counts pass under either reading, and the sums are compared under the one that matched. A sum or currency that does not match is a warning, not an error: with the counts right every booking is listed. A total that does not parse (no count, currency or amount) fails the call.
- **`:86:`:** continuation lines are joined with no separator, whatever length the bank wraps at. The whole body goes to `formats::tag86`, with no length limit (§5.1.4 footnote 30: full use of field 86 needs 568 characters). A `:86:` that does not follow a `:61:` (statement level, after `:62F:`) belongs to no booking and is dropped. A `:61:` without `:86:` gets no SEPA fields.
- **Characters:** the bank's own (`ä`, `ß`), in every field. There is no SWIFT mapping and no DIN 66003 repair of `[\]~`; add one as a bank-profile quirk only if a bank is seen to need it.
- **Dates:** a date that does not exist in its month (29 February outside leap years, 30/31 February for month-end postings) becomes the month's last day, in the value and the entry date.
- **Amount sign:** `D` and `RC` (Storno Haben) are negative, `C` and `RD` (Storno Soll) positive. The optional third currency letter is accepted.
- **Errors:** a malformed `:61:`, a balance without currency, a statement with bookings but no currency, or a field before the first `:20:` fails the call with `FinTSError::Mt940`, naming the line and the field.

CAMT (`src/camt.rs`, `parse_camt052(xml, BookingIdSource)`) fills the same `Transaction` fields; [Model](#2-model) lists both sources.

**Observed shape (Consorsbank, 75 masked `:86:` fields, `testbank/fixtures/replay/consorsbank-76030080.trace.json`):**

- GVCs are legacy codes (005, 008, 016, 027, 052, 053, 059, 066), never 1xx. Teil 6 defines first digit 0/1 as euro payments and 9 as unstructured.
- Every purpose is `?20EREF+…?21…?22KREF+…?23SVWZ+…[?24–?27]`. Each keyword starts a subfield, as §5.1.4 requires, and the EREF value continues in `?21`.
- The bank wraps physical lines at 65 characters, even inside a subfield code or keyword (`?22K⏎REF+`, `?⏎27`).
- `?33` continues `?32`, and one name contains a `+`. There is no `?34` and no `?6x`. The longest field is 316 characters.

## 2. Model

MT940 and CAMT fill the same `Transaction` fields. `raw` keeps the rest.

| `Transaction` field | MT940 | CAMT |
|---|---|---|
| `purpose` | `SVWZ+` value; without it the text before the first keyword, or the whole joined text when there is no keyword | `RmtInf/Ustrd` parts joined with a line break (the bank's lines); none without `Ustrd` (`AddtlNtryInf` is the `posting_text` only) |
| `end_to_end_id` | `EREF+` | `EndToEndId` |
| `mandate_id` | `MREF+` | `MndtId` |
| `creditor_id` | `CRED+` | `Cdtr` (or `Cdtr/Pty`) `Id/PrvtId/Othr/Id`, where Anlage 3 V26.11 §7.1.7.4 puts the SEPA creditor identifier; camt.052 has no `CdtrSchmeId` |
| `debtor_id` | `DEBT+` | `Dbtr` `Id` (`PrvtId` or `OrgId` `Othr/Id`) |
| `purpose_code` | `PURP+` (not in §5.1.4; listed in architecture.md §2.4) | `Purp/Cd` |
| `customer_reference` | `:61:` customer reference | always `None` |
| `bank_reference` | `:61:` bank reference (after `//`) | `AcctSvcrRef`: transaction level, then entry level, then `Refs/Prtry/Ref` |
| `booking_id` | always `None` (the `:61:` bank reference is often `NONREF` or repeated) | per `BankProfile.booking_id`: `CamtEntryAcctSvcrRef` takes the entry `AcctSvcrRef` of a booked entry (DKB); `None` otherwise |
| `reversal: bool` | `:61:` mark `RC` or `RD` | `Ntry/RvslInd` true |
| `ultimate_debtor` | `ABWA+` of a transfer, `ABWE+` of a direct debit | `UltmtDbtr/Nm` |
| `ultimate_creditor` | `ABWE+` of a transfer, `ABWA+` of a direct debit | `UltmtCdtr/Nm` |
| `original_amount: Option<(Decimal, Currency)>` | `OAMT+` (`12,50`) in the statement currency: the keyword has no currency | `AmtDtls/InstdAmt/Amt` with its `Ccy` |
| `fees: Option<(Decimal, Currency)>` | `COAM+`, statement currency | `Chrgs/TtlChrgsAndTaxAmt`, else the first `Chrgs/Rcrd/Amt` (entry or details level) |
| `return_reason` | four-letter code after `REJECT`/`RETURN` in `SVWZ+` | `RtrInf/Rsn/Cd` |
| `sequence_type` | none (MT940 has no field) | `PmtTpInf/SeqTp` |
| `applicant_name`, `applicant_iban`, `applicant_bic`, `posting_text`, `gvc`, `prima_nota` | `?32`+`?33`, `?31`, `?30`, `?00`, GVC, `?10` | camt party fields and bank transaction code |

- **Dates.** `date` is the booking date, else the value date. A pending CAMT entry with neither takes the day of the report's `CreDtTm` (comdirect sends such entries, [banks.md](banks.md)); a booked entry without a date is an error.
- **One `TxDtls`.** A CAMT entry maps its first `TxDtls` only (the three banks send one per entry); `Transaction` documents it.
- **`purpose` is the `SVWZ+` value only,** as CAMT's `Ustrd`: one meaning across formats. The keywords have their own fields, and the joined `:86:` is in `raw["tag86"]`.
- **Placeholders are absent.** One shared `formats::absent(&str) -> bool` covers the empty string, `NOTPROVIDED` (the EPC placeholder for the end-to-end ID; §5.1.4 says it is not put after `EREF+`, but Consorsbank sends it) and `NONREF` (the SWIFT "no reference" placeholder; Consorsbank `KREF+NONREF`). Values are trimmed and compared case-insensitively. It applies to reference values: `EREF`, `KREF`, `MREF`, `CRED`, `DEBT`, CAMT's `EndToEndId`, `MndtId` and the creditor and debtor IDs, and the `:61:` customer reference. For text values (`SVWZ`, `ABWA`, `ABWE`, `PURP`, `OAMT`, `COAM`), only empty means absent.
- **Ultimate parties.** `ABWA+` is the ultimate party on the initiator's side: the debtor of a transfer, the creditor of a direct debit. A booking is a direct debit when `MREF+` or `CRED+` is present. An R-transaction's keywords refer to the original payment (§5.1.4 footnote 33), so the roles do not depend on the booking's direction. The caller picks `ultimate_debtor` or `ultimate_creditor` by the sign of `amount`.
- **`OAMT+` and `COAM+`** occur in SEPA R-transactions: the original payment's amount and the compensation amount (interest compensation). SEPA is EUR-only, so the statement currency is right. Both are positive; a value that is not a plain decimal (`12,50` or `12.50`) stays in `raw["sepa"]` only. Unverified live (no R-transaction in the local traces).
- **R-transactions:** `SVWZ+` starting `REJECT`, `RETURN`, `REFUND` or `RUECKLASTSCHRIFT` (uppercase, §5.1.4 footnote 32) is a marker. `reversal` comes from the `:61:` mark alone.
- **`raw`** (MT940): `tag86` (the complete `:86:` after the line join, so no text is lost), `sepa` (every keyword value as sent, placeholders included), `text_key_extension` (`?34`), `other_subfields` (unknown codes), and the `:61:` and `:86:` values (`customer_ref`, `bank_ref`, `funds_code`, `transaction_type`, `supplementary_details`, `gvc`, `prima_nota`, `applicant_*`, `posting_text`, dates, amount, currency). `KREF+` is in `raw["sepa"]` only. There is no `SepaFields` struct: the flat fields exist and CAMT fills the same ones.

## 3. Parsing rules (`formats::tag86`)

`tag86::parse(&str) -> Tag86` is pure, never fails and never panics.

1. **Lines:** remove CR/LF inside the field and insert nothing, because the 65-character wrap can split a subfield code.
2. **GVC:** three digits followed by a subfield marker `?nn` (the separator is always `?`, §5.1.4). If the field has no `?nn` after the GVC, or the GVC starts with `9`, the text after the GVC is the purpose (unstructured), and the keyword scan runs over it (rule 6). A field without a GVC is purpose as a whole.
3. **Known subfields:** `00` posting text, `10` primanota, `20–29` and `60–63` purpose, `30` BIC/BLZ (cut at the first space), `31` IBAN/account, `32`+`33` name, `34` text key extension. An empty known subfield gives `None`. An unknown code is kept in `other`. Nothing fails the parse.
4. **Purpose join:** `?20…?29` then `?60…?63`, in code order, with no separator ("Fortführung", §5.1.4). Subfield values are not trimmed before the join, so a trailing space between words survives. Final values are trimmed.
5. **Keywords:** an exact uppercase four-letter token followed by `+`, from the known set (`EREF KREF MREF CRED DEBT COAM OAMT SVWZ ABWA ABWE PURP`).
   - A keyword counts at the start of a purpose subfield, so text such as `XYZ+` inside SVWZ never splits.
   - A value runs to the next keyword, across subfields, and repeats of a keyword are joined.
   - Text before the first keyword, or a purpose with no keyword at all, is plain purpose (non-SEPA booking).
   - An unknown `ABCD+` at a subfield start is not a keyword: it stays in the current value.
6. **Keywords inside a subfield:** if the purpose starts with a known keyword (structured or not), a keyword that comes later in §5.1.4 order than the current one also splits inside a subfield. SVWZ text can then only be ended by `ABWA+`/`ABWE+`. `PURP` is not in §5.1.4 and counts only at a subfield start.
7. **GVC is not used for SEPA detection,** since Consorsbank sends legacy codes. The keywords decide.

## 4. Where it lives

- `src/formats/mod.rs`: modules and `absent` (`absent.rs`).
- `src/formats/tag86.rs`: the `:86:` parser. `Tag86` holds the GVC, the subfields, `purpose` (joined text with keywords), `plain_purpose`, the keyword values and the return marker. `raw(Keyword)` gives a value as sent, `value(Keyword)` gives `None` for an absent one.
- `src/formats/mt940.rs`: the MT940/MT942 parser (§1) and `build_transaction`.
- `src/camt.rs`: CAMT.052, using `formats::absent`. `drop_shared_booking_ids` clears a `booking_id` that occurs on more than one entry of a statement.
- MT942 pending bookings go through the same `:86:` path (§5.2 uses the same field 86).

## 5. Tests

- **`tag86` unit tests**, with synthetic values only (`DE00…` test IBANs, the `DE98ZZZ09999999999` test creditor ID, invented names): the two §5.1.5 bookings, a value spanning `?20`→`?21`, SVWZ in `?2x` and `?6x` including `?63`, a keyword split by a physical line wrap, placeholders, no keywords, GVC 999, an unknown `ABCD+`, a `+` inside a name, empty and unknown subfields, a 568-character field, R-transactions, and a Consorsbank-shaped field with 65-character wraps.
- **Property tests** (`proptest`): `tag86::parse` never panics on arbitrary strings, and the purpose text is kept whatever line wraps are inserted.
- **`mt940` tests:** `:61:` fields, marks and optional parts, error messages, a `:61:` without `:86:` next to one with it, a statement-level `:86:` ignored, a long `:86:` kept whole, MT942 totals (counts, reversals on either side, sum and currency warnings), creation time in both forms, currency from the totals, `@@` line breaks, separator-only data, month-end dates, and a never-panics property on random bytes.
- **`camt` tests:** the DKB-shaped report, booking IDs per profile rule, and shared references.
