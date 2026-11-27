# Codec: how to add a segment

The `codec` module turns FinTS wire bytes into typed segment structs and back. A segment version is one struct with `#[derive(Segment)]`; a data element group (DEG) is one struct with `#[derive(DataElementGroup)]`. The derive generates parsing and serialising. The design is in [architecture.md](architecture.md#21-codec-declarative-segments-in-rust); spec documents are listed in [spec-sources.md](spec-sources.md).

Write every definition from the specification text in `spec/`, never from another FinTS client ([AGENTS.md](../AGENTS.md), "Clean room").

## Steps

1. Find the table: `grep -n "Kennung:  *HKSAL" spec/*.txt`, then read the "Format" table of the version you need, and the data dictionary entry of every DEG it uses.
2. Write the struct in `src/codec/defs/<code>.rs` (one file per segment code, all versions in it). Put DEGs used by several segments in `src/codec/defs/common.rs`.
3. Export it in `src/codec/defs/mod.rs` and add it to the `registry!(...)` list in `src/codec/registry.rs`. That makes it a `TypedSegment` variant and visible to `lookup(code, version)` and `versions(code)`.
4. Write the tests (below). Run `cargo test`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.

## Mapping a specification table

Skip row 1 "Segmentkopf": the header `code:number:version[:reference]` is handled by the codec. Every further row becomes one field, in table order.

**Typ / Format → field type**

| Spec | Rust | Rules the type enforces (Formals) |
|---|---|---|
| DE `an` | `An` | ISO 8859-1 without CR/LF (B.4.1) |
| DE `id` | `Id` | `an`, at most 30 (B.4.2) |
| DE `code` | `Code` | `an`; the value list is not checked (B.4.2) |
| DE `num` | `Num` (`u64`) | digits, no leading zeros (B.4.1) |
| DE `dig` | `Dig` (text) | digits, leading zeros kept (B.4.1) |
| DE `ctr` | `Dig` + `#[de(len = 3)]` | `dig` length 3 (B.4.2) |
| DE `jn` | `Jn` | `J` or `N` (B.4.2) |
| DE `dat` | `Dat` | `JJJJMMTT`, existing date (B.4.2) |
| DE `tim` | `Tim` | `hhmmss`, valid time (B.4.2) |
| DE `cur` | `Cur` | 3 upper-case letters (B.4.2) |
| DE `wrt`, `float` | `Wrt` (`Decimal`) | decimal comma, at most 15, written canonically (B.4.1, B.4.2) |
| DE `bin` | `Bin` | `@len@` binary only (B.4.1, H.1.4) |
| DEG `btg` | `Btg` | `Wrt` + `Cur` (Messages B.1) |
| DEG `kti` | `defs::Kti1` | Messages B.3.2 |
| DEG `kik` | `defs::Kik1` | Messages B.2 |
| any other DEG | a new `#[derive(DataElementGroup)]` struct | |

Not yet implemented: `txt` (like `an` but CR/LF allowed), `vdat` (31 days in every month; `Dat` rejects 31.04.), `dta`. Add them as new types in `src/codec/types.rs` (implement `Scalar`, add to the `scalar_element!` list) when a segment needs them.

**Länge → attribute** (counted in bytes before escaping, Formals B.3.3)

| Spec | Attribute |
|---|---|
| `..35` | `#[de(max = 35)]` |
| `12` | `#[de(len = 12)]` |
| `#`, `..` or empty | none (the type's own limit applies) |

**Status → optionality** (Formals B.3.1)

| Spec | Rust |
|---|---|
| `M` | `T` |
| `O`, `C` | `Option<T>` |
| `N` | the field still exists; `C` with "N: sonst" is `Option<T>` |

A conditional rule (`C`, "M: bei ...") is not checked by the codec. Copy it into the field's doc comment; the protocol or job layer enforces it.

**Anzahl → repetition** (Formals B.3.1)

| Spec | Rust |
|---|---|
| `1` | `T` or `Option<T>` |
| `O m` | `#[de(max_count = m)] Vec<T>` |
| `O n` (unbounded) | `Vec<T>` (only as the last field) |
| `M n` | `#[de(min_count = 1)] Vec<T>` (only as the last field) |
| `M m` (exactly m) | `#[de(min_count = m, max_count = m)] Vec<T>` |
| `M l..m` | `#[de(min_count = l, max_count = m)] Vec<T>` |

Restrictions (value lists, ranges like `>0`, "1..2048") are not checked. Put them in the doc comment.

## Attribute reference

```rust
#[derive(Segment)]                                 // on a segment version
#[segment(code = "HKSAL", version = 7)]            // both required; code 1-6 upper-case letters/digits, version 1-999
#[derive(DataElementGroup)]                        // on a DEG; at least one field

#[de(max = 35)]                                    // maximum length in bytes (spec "..35"); data elements only
#[de(len = 12)]                                    // fixed length in bytes (spec "12"); data elements only
#[de(min_count = 1, max_count = 98)]               // repetition counts; field type Vec<T>
Option<T>                                          // optional element (status O or C)
```

`max` and `len` exclude each other. `max`/`len` on a `Vec<T>` or `Option<T>` field apply to each `T`. On a group type they are a compile error. `min_count` defaults to 0; `max_count` defaults to unbounded, which is allowed only for the last field. Generic structs, tuple structs and enums are rejected. Field types resolve where the struct is defined; the generated code reaches the codec through `::fints_client::codec`, inside the crate as well (`extern crate self as fints_client` in `lib.rs`).

## What the codec does with positions

These rules come from Formals H.1.2 and H.1.5; the derive implements them, nothing in a definition needs to.

- **Nested groups are flattened.** A DEG inside a DEG is a run of colon-separated elements: KTI with its KIK is `IBAN:BIC:number:sub:280:blz`.
- **Every position is kept.** An absent optional element (or group) leaves its positions empty: `HKSAL:3:7+IBAN:BIC+N++T1'`. A repetition with `max_count` always takes `max_count × width` positions; unused ones are empty (`+DE1+DE1++++DE2+DE2'`, H.1.2).
- **Only trailing empties are cut**, at the end of each data element and of the segment, when writing. When reading, missing trailing positions read as empty, and extra empty positions are accepted, so cut and uncut input both parse.
- **An open repetition must come last** in its group (a group ending in one must itself come last). The derive checks this at compile time.
- **Unknown trailing data is an error** (`UnexpectedData`): a non-empty element after the last one the definition knows.

## How the protocol uses the codec

- **Outgoing.** `message.rs` builds the envelope (HNHBK, HNVSK, HNVSD, HNSHK, HNSHA, HNHBS) from the definitions; `segments::request` fills the definitions of HKIDN, HKVVB, HKSYN, HKEND, HKTAN 6/7 and the jobs (HKSAL 5–7, HKKAZ 5–7, HKCAZ 1, HKWPD 5–6, HKWDU 5). Values from the caller or the bank go in verbatim (`segments::verbatim`: no trimming, format rules still apply). Segments are serialised with `codec::wire`.
- **Incoming.** Responses are tokenized with `codec::wire`; `segments::read` reads each segment it needs with `codec::read_segment` and maps the typed value to the domain types (`ReturnCode`, `TanMethod`, `UpdAccount`, `AccountBalance`, MT940/camt/MT535/MT536 payloads). A segment its definition rejects is logged and yields nothing; `Response::decode_segments()` / `segment_errors()` show it to the caller. Two exceptions: an unreadable HITAN fails with `FinTSError::SegmentUnreadable` (the challenge cannot be guessed), and an unreadable HIRMG or HIRMS makes `check_errors` fail. HITAN and HISAL versions the client never requests are not read. HISPAS and HIWDUS have no definitions (HIWDUS is read for its version only).
- **Remainder.** Versions without a definition stay hand-written and are marked as such: `segments::response` (HITANS above 7, read with the version 7 layout; HIUPD 1–3, 5 and above 6). These have no published table. Adding a definition for one of them means removing it there.

## Reading and writing

```rust
use fints_client::codec::{self, defs::HkSal7};

let (header, hksal) = codec::from_wire::<HkSal7>(bytes)?;          // one segment
let bytes = codec::to_wire(&hksal, header.number, header.reference)?;
let parsed = codec::parse_message(message_bytes)?;                  // Vec<ParsedSegment>
codec::versions("HKSAL");                                           // implemented versions, ascending
codec::lookup("HKSAL", 7);                                          // Option<&SegmentDef>
```

`parse_message` returns `ParsedSegment::Typed { header, segment }` or `ParsedSegment::Raw { segment, error }`. `segment` in `Raw` is the tokenized `RawSegment` (the raw type `parser.rs` defines); `error` is a `CodecError` that names the segment (code, version, number), the DE position (header = DE 1, as in the spec tables), the group element position (after flattening, 1-based), the Rust field path (`parameters.methods[1].name`) and the rule (`ErrorKind`). A segment without a definition is `Raw` with `ErrorKind::UnknownSegment`. `protocol::Response::decode_segments()` and `segment_errors()` give the same for a received response.

## Tests every definition needs

In the definition's file, `#[cfg(test)] mod tests`:

1. **Wire round trip**: a synthetic wire string (no real names, IBANs, account numbers, user IDs) parses and serialises back to the identical string. Include the optional elements once present and once absent.
2. **The protocol's use**: if the protocol sends the segment, add a function to `src/segments/request.rs` that fills the definition and a wire-string test there (and in the definition's file, compare its output with the wire the segment had before, if it was hand-written). If the protocol reads it, add the reader to `src/segments/read.rs` with a test of the fields it extracts. For an order (HKSAL, HKKAZ, …), add the version to `Order::implemented` beside the builders (`src/segments/mod.rs`) together with the builder arm; registering a definition alone does not make it sendable. The test `segments::tests::implemented_versions_match_the_builders` builds every listed version and checks that codec-backed versions are registered.
3. **One rule violation** with the expected `ErrorKind`, `de`, `element` and `field`.
4. Optionally a property test (see `defs/hitans.rs`).

## Worked example: HKSAL version 7

Messages 2022, C.2.1.2.2 (`grep -n "Kennung:  *HKSAL" spec/FinTS_3.0_Messages_*.txt`):

```text
Nr. Name                          Typ  Format Länge Status Anzahl Restriktionen
 1  Segmentkopf                   DEG               M     1
 2  Kontoverbindung international DEG  kti     #    M     1
 3  Alle Konten                   DE   jn      #    M     1
 4  Maximale Anzahl Einträge      DE   num    ..4   C     1      >0; O: BPD "Eingabe Anzahl Einträge erlaubt" = J
 5  Aufsetzpunkt                  DE   an     ..35  C     1      M: after a touchdown
```

Row 1 is the header. Row 2 is the `kti` group, already in `defs/common.rs` as `Kti1` (Messages B.3.2). Rows 4 and 5 are `C`, so `Option`; their lengths become `max`.

```rust
// src/codec/defs/hksal.rs
use super::common::Kti1;
use crate::codec::{An, Jn, Num, Segment};

/// HKSAL version 7, customer order. Messages 2022, C.2.1.2.2.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKSAL", version = 7)]
pub struct HkSal7 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Maximale Anzahl Einträge (>0; only if the BPD allows it).
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 5 Aufsetzpunkt (only after the bank sent a touchdown point).
    #[de(max = 35)]
    pub touchdown: Option<An>,
}
```

Then `pub use hksal::HkSal7;` in `defs/mod.rs`, `HkSal7` in `registry!(...)`, and the test (with `use crate::codec::{from_wire, to_wire};`):

```rust
#[test]
fn wire_round_trips() {
    let wire = "HKSAL:3:7+DE00120300009900012345:BYLADEM1001+N++T1'";
    let (header, v) = from_wire::<HkSal7>(wire.as_bytes()).unwrap();
    assert_eq!(v.touchdown.as_ref().map(An::as_str), Some("T1"));
    let back = to_wire(&v, header.number, header.reference).unwrap();
    assert_eq!(String::from_utf8(back).unwrap(), wire);
}
```

A repeated group inside a group looks like HITANS v6 (`defs/hitans.rs`): all TAN methods sit in the one parameter group, so the group struct ends with `#[de(min_count = 1, max_count = 98)] pub methods: Vec<TanMethod6>`, and `TanMethod6` is a 21-field `DataElementGroup`.

## Schema: a static description of each definition

Both derives also generate a description of the definition, from the same field list and attributes as the parse/serialise code (`codec::schema`; no wire effect):

- `Segment::SCHEMA: &'static [FieldSchema]`: the elements after the header, in wire order; also `SegmentDef::schema()` from the registry.
- `Element::KIND: ElementKind`: `Scalar { format }` (`Scalar::FORMAT`: `an`, `num`, ...) or `Group { name, fields }`; `Element::OPTIONAL` is true for `Option<T>`.
- `FieldSchema { name, kind, max, len, min_count, max_count, repeated }`: `T` is `1..1`, `Option<T>` is `0..1`, `Vec<T>` is `min_count..max_count` (`None`: unbounded).

```rust
let hksal = fints_client::codec::lookup("HKSAL", 7).unwrap();
assert_eq!(hksal.schema()[0].name, "account");
```

## Naming

- Segment structs: code in camel case plus version, `HkSal7`, `HiTans6`, `HnHbk3`.
- Group structs: an English name plus the element version, `Kti1`, `Kik1`, `TanParameters6`, `TanMethod6`.
- Fields: English snake case. The doc comment starts with the spec's row number and German name (`/// 4 Maximale Anzahl Einträge.`).

## Design notes

- **Attributes.** One namespace `de` for fields of both derives; optional is `Option<T>` and nothing else.
- **Types.** `Num` is `u64`; `Dig` keeps its digits as text (exact round trip of `000000000123`), `ctr` is `Dig` with `len = 3`; header number, version and reference are `u16`. `Wrt` is `rust_decimal::Decimal`, never negative. `Btg` is itself a derived group.
- **Registry.** A `macro_rules!` list in `src/codec/registry.rs` (one line per definition) builds the `#[non_exhaustive]` `TypedSegment` enum and the lookup table; no `inventory`/`linkme`. `ParsedSegment::Typed` holds a `Box<TypedSegment>`.
- **Module names.** Definitions live in `codec::defs`; `extern crate self as fints_client;` in `lib.rs` lets generated code use one path inside and outside the crate.
- **Credentials.** `UserDefinedSignature1` (PIN, TAN) has a hand-written `Debug` that prints `<hidden>`.
- **Compile-fail tests** are `compile_fail` doctests (`codec::CompileFailTests`, `cfg(doctest)`), not trybuild. They pass on any compile error, so check a new one by hand that it fails with the intended message.
- **Lints.** `src/codec/mod.rs` turns on a stricter lint set for the codec (`#![warn(...)]` at its top); `Cursor::next` carries one `#[allow(clippy::should_implement_trait)]` (it never ends, so it is not an iterator).

## Limitations

- No checks for value lists, value ranges, conditional status or rules across elements; these belong in the protocol or job layer.
- Codes are `Code` strings; there are no typed code enums yet (a typed code can implement `Scalar`).
- `Wrt` accepts trailing zeros after the comma (`100,50`) and writes canonically (`100,5`), so such input does not round-trip byte for byte.
- Text read from the wire is kept untrimmed; `An::new` and friends trim, as B.4.1 requires for outgoing data.
- An element that a bank sends beyond the definition is an error, so a segment with extra trailing data stays raw.
- Strict reading where the spec is clear, since the raw fallback keeps a rejected segment visible: non-empty elements beyond the definition are `UnexpectedData`; `an` rejects CR/LF; `num` rejects leading zeros; fixed lengths are exact. The lenient points are the `Wrt` trailing zeros, the untrimmed text above, and the Rückmeldungstext of HIRMG and HIRMS, read without its `..80` limit (DKB sent 93 bytes on a failed login). Whether banks send any of this is unknown.
- A message with no definition for its code and version is `Raw` with `ErrorKind::UnknownSegment`; `Response::segment_errors()` leaves those out so it reports only real breakage.
- On read, a repeated group at the end of a group (HITANS) shows a 99th method as `UnexpectedData` at its position; at segment level it is `TooManyRepetitions`. Writing always reports `TooManyRepetitions`.
- The codec types are not `serde`-serialisable yet.
- `codec::wire` decodes ISO 8859-1 exactly, and the protocol uses it for every message.
