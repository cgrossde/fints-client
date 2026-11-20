# Specification sources

The FinTS PDFs are not redistributed in this repository: `spec/` is gitignored and each developer fetches the files from the URLs below. Segment definitions are written from these documents.

All documents are from the official publisher site (https://www.fints.org/de/spezifikation, Deutsche Kreditwirtschaft). Download date for all: 2026-11-15. Download links on that page are time-limited (`/securedl/...` tokens), so the stable source is the page itself (current FinTS 3.0) or https://www.fints.org/de/spezifikation/archiv (HBCI 2.x and previous FinTS 3.0 Messages). Each PDF has a `.txt` sibling made with `pdftotext -layout`.

| Title | Version / date on title page | Source | Local file (`spec/`) | SHA-256 |
|---|---|---|---|---|
| Formals | 3.0-FV, 06.10.2017, Final Version | https://www.fints.org/de/spezifikation (FinTS 3.0 section) | `FinTS_3.0_Formals_2017-10-06_final_version.pdf` | `6b4809acd43acd2c6166c486964dee4b84b488a6a6902b29d1221458eaed7239` |
| Sicherheitsverfahren PIN/TAN (HKTAN 1-7, decoupled) | 3.0-FV Rel. 2020, 10.07.2020 | same | `FinTS_3.0_Security_Sicherheitsverfahren_PINTAN_2020-07-10_final_version.pdf` | `77bb5724cb391434cd7188924a1ff89bff040bf340dc49a09c92cdeb4ee7dc3a` |
| Sicherheitsverfahren HBCI (defines HNSHK/HNSHA/HNVSK/HNVSD, shared with PIN/TAN) | 3.0-FV, 11.06.2024, Final Version | same | `FinTS_3.0_Security_Sicherheitsverfahren_HBCI_Rel_2024-06-11_final_version.pdf` | `37a82f7e51386f605154f1d5f47d64dc97ca3cab7161bc8769b9c7ca0f32735e` |
| Alternative Sicherheitsverfahren | 3.0, 06.06.2025, Final Version | same | `FinTS_3.0_Security_Alternative_Sicherheitsverfahren_Rel._2025-06-06_FV.pdf` | `3b778445225e5918ba39fe09836bfa07621426bc51468ebb83a4b8abce1235f8` |
| Messages: Multibankfähige Geschäftsvorfälle | 3.0, 15.04.2022, FV | same | `FinTS_3.0_Messages_Geschaeftsvorfaelle_2022-04-15_final_version.pdf` | `a3db32dc27b596f67dea0d4447d07011d8945dc589aafcacb869018ea7d94afd` |
| Messages: Geschäftsvorfälle, Verification of Payee | V1.02, 19.10.2026, FV | same | `FinTS_3.0_Messages_Geschaeftsvorfaelle_VOP_1.02_2026_10_19_FV.pdf` | `54d959478d157922e092900c82d84553edd97ceb390b6d97484355962e026729` |
| Rückmeldungscodes (supplement to Formals) | 3.0 / 4.x FV, 03.02.2026 | same | `FinTS_Rueckmeldungscodes_2026-02-03_FV.pdf` | `f0ab2a40c93921a31b715c6006e683ebd9ca7a4d3a842120b8c75d2d535c7b2d` |
| HBCI Homebanking-Computer-Interface (Schnittstellenspezifikation) | Version 2.2, Stand 10.05.2000 | https://www.fints.org/de/spezifikation/archiv (`HBCI_V2.x_FV.zip`, SHA-256 `4adb3b0b5778aa82cb5e36ef1163e5a08cadca4defb5a1e594aa9c4517611ebc`) | `HBCI22 Final.pdf` (txt: `HBCI22_Final.txt`) | `df1e375d54b1c1a1530787bb9d3f62bea88634aceec656b4b216300f725357e8` |
| Messages: Multibankfähige Geschäftsvorfälle (predecessor of 2022) | 3.0, 07.08.2015, FV | https://www.fints.org/de/spezifikation/archiv (`FinTS_V3.0_2022-04-29-FV-RM.zip`, SHA-256 `053c5c4c71a6710e88a0491f97fb27907751d18731f536f691076c24ef043b38`) | `FinTS_3.0_Messages_Geschaeftsvorfaelle_2015-08-07_final_version.pdf` | `64461578bcf3fd7fa4ed9c591e78f61d86ec62afa1f3ae8947fc5e4c573eec46` |
| DFÜ-Abkommen Anlage 3: Spezifikation der Datenformate (SEPA pain.001 2.2.1, SCT Inst 2.2.4, pain.002 2.2.3, VoP status report 2.2.6, character set 2.1, simple types 2.3) | V26.11, 09.04.2026, Final Version, in force 15.11.2026 | https://www.ebics.de/de/datenformate (downloaded 2026-11-17) | `Anlage_3_Datenformate_V26.11.pdf` | `f24aa4b83665155c4a744970b7ed71e6dd827e39b8d7f47ea34a7ed4dcc75f3e` |
| DK schemas pain.001.001.09 GBIC_5 and ISO pain.002.001.10 | GBIC_5 (xsd dated 2025-01-04) | https://www.ebics.de/de/datenformate/ergaenzende-dokumente (`DK-TVS_SEPA_GBIC_5zzglISO_Originale.zip`, downloaded 2026-11-17) | `pain/DK-TVS_SEPA_GBIC_5zzglISO_Originale.zip`, unpacked in `pain/gbic5/` | `dabcb6e0329a746726bad92dda2a6f7aec1ddad0d5796d643bbc40d896206ace` |
| DK schema pain.001.003.03 (with Anlage 3 V2.9) | V2.9 archive | https://www.ebics.de/de/datenformate/archiv (`Anlage3_Archiv_V2_9.zip`, downloaded 2026-11-17) | `pain/Anlage3_Archiv_V2_9.zip`, unpacked in `pain/v29/` | `801c9d962d43fdabce1b13f9c300d4f835be2374cfd429ec59e68ec65cb51fc3` |

The HBCI security document is in `spec/` although RDH/chip-card security is out of scope: it is the only place that defines the HNSHK, HNSHA, HNVSK and HNVSD layouts (the PIN/TAN document refers to it as `[HBCI]`). Read only its message-format chapter B.5 for PIN/TAN work.

## Missing

- English versions: the site offers none (`/en/specification` returns 404; the specification page is German only).
- Separate FinTS 3.0 Data Dictionary: not offered. Only a FinTS 4.1 Data Dictionary exists and was not fetched. The data element and group definitions (DE/DEG) for 3.0 are inside Formals and the Messages document.
- HKSAL / HKKAZ / HKWPD version 5: not in Messages 2022 (starts at 6) and not in Messages 2015 (also starts at 6). Tables are in HBCI 2.2 (VII.2.2, VII.2.1.1, VII.4.3.1). HBCI 2.1 (`Gesamtdok_HBCI21o.pdf` in the same zip) only has HKSAL/HKKAZ 4 and HKWPD 2/3.
- HIUPD version 5: no table in any document available (Formals 2017 has HIUPD 6 and only shows version 5 in examples; HBCI 2.2 has version 4; the fints.org archive page offers only FinTS 4.x, the FinTS 3.0 2022 change-mark volumes and HBCI 2.x). It stays on the hand-written reader. Same for HITANS/HITAN above 7 and HIUPD above 6 (unpublished).
- Not fetched on purpose (FinTS 3.0 extras on the same page): FinTS 4.1 documents, Echtzeitbenachrichtigungen, change lists. chipTAN/HHD is public; see [Related specifications](#related-specifications-not-in-spec).

## Related specifications (not in `spec/`)

Payloads inside FinTS binary DEs, and HHD-UC, are not defined by the FinTS segment tables. Stable pages (download links on them are time-limited `securedl` tokens). Of these, only Anlage 3 V26.11 and the pain schemas are in `spec/` (table above).

| Need | Document | Version / date | Stable source |
|---|---|---|---|
| chipTAN / HHD-UC (flicker, QR, photoTAN matrix) | Belegungsrichtlinien TANve1.5; GV→visualization mapping | FV 16.04.2018; mapping 18.07.2013 | https://www.fints.org/de/spezifikation/chiptan |
| HHD device (SECCOS) | DK HandHeldDevice spec | — | https://www.dk-download.de/ (login) |
| camt.052 / pain.001 / pain.002 filling (DK) | DFÜ-Abkommen Anlage 3, Spezifikation der Datenformate | **V26.11**, FV 09.04.2026, in force 15.11.2026 | https://www.ebics.de/de/datenformate |
| camt XSDs, GVC↔ISO BTC, camt examples | `ISO-Schemacamt05x_Version_2019.zip`; `Anlage3_Anhang1_Mapping_GVC_V26.11.zip`; camt implementation-help zip | ISO 2019 / Jul 2025 help | https://www.ebics.de/de/datenformate/ergaenzende-dokumente |
| pain TVS (SCT / SCT Inst) + original ISO pain.002 | `DK-TVS_SEPA_GBIC_5zzglISO_Originale.zip` | GBIC_5 | same Ergänzende-Dokumente page |
| ISO 20022 message definitions | camt.052 BankToCustomerAccountReport; pain.001 CustomerCreditTransferInitiation; pain.002 CustomerPaymentStatusReport | catalogue | https://www.iso20022.org/iso-20022-message-definitions (also linked from https://www.fints.org/de/links) |
| SWIFT MT940 / MT942 / MT535 / MT536 | Filling notes already in **HBCI 2.2 IX.2.8 / 2.9 / 2.4 / 2.5** (PDF in the table above). Official SWIFT User Handbooks are Knowledge Centre (paid). fints.org/links: the SWIFT formats FinTS needs are in the FinTS annex. | 10.05.2000 | this repo's HBCI 2.2 PDF |
| DKKKU / DKWDH | none | — | **Not implemented** (Sparkasse / FI proprietary). No public spec. Prefer HKKKU / CR0538 if card bookings are needed. |


## Where to find what

Page numbers are physical PDF pages (pdftotext form feeds), not the printed page numbers, which can differ by a page or two. Section numbers are as printed. Search the `.txt` with `grep -n "Kennung:  *HKSAL"` to hit each definition.

| Segment | Document | Section | PDF page |
|---|---|---|---|
| HNHBK v3 | Formals | B.5.2 Nachrichtenkopf | 23 |
| HNHBS v1 | Formals | B.5.3 Nachrichtenabschluss | 23-24 |
| HNVSK v3 | HBCI security | B.5.3 Verschlüsselungskopf | 45 |
| HNVSD v1 | HBCI security | B.5 Formate für Signatur und Verschlüsselung | 46 |
| HNSHK v4 | HBCI security | B.5.1 Signaturkopf | 41 |
| HNSHA v2 | HBCI security | B.5.2 Signaturabschluss | 44 |
| HKIDN v2 | Formals | C.3.1.2 Segment: Identifikation | 51 |
| HKVVB v3 | Formals | C.3.1.3 Segment: Verarbeitungsvorbereitung | 53 |
| HKSYN v3 / HISYN v4 | Formals | C.8.1.2 / C.8.2.2 Synchronisierung | 76 / 77 |
| HKEND v1 | Formals | C.4.2.2 Segment: Dialogende | 62 |
| HKTAN 6 | PIN/TAN | B.5.1 Segmentversion #6 | 57 |
| HKTAN 7 | PIN/TAN | B.5.2 Segmentversion #7 (decoupled: B.4.2.2, B.4.3.4) | 64 |
| HKTAN 1-5 | PIN/TAN | E.1.1-E.1.5 (older versions; HKTAN 5 defined from E.1.5 a) | 178-197 |
| HITAN / HITANS 1-5 | PIN/TAN | E.1.1-E.1.5 b / c; DEGs in D "Parameter Zwei-Schritt-TAN-Einreichung" and "Verfahrensparameter Zwei-Schritt-Verfahren", Elementversion #1-#5 | 178-197; D |
| HIUPD v4 | HBCI 2.2 | V.3 Kontoinformation | — |
| HITAN 6 / 7 | PIN/TAN | B.5.1 / B.5.2 | 61 / 67 |
| HITANS 6 / 7 | PIN/TAN | B.5.1 / B.5.2 | 63 / 69 |
| HIPINS | PIN/TAN | Erweiterung der Bank- und Userparameterdaten | 75 |
| HIUPA v4 | Formals | E.2 Userparameter allgemein | 95 |
| HIUPD v6 | Formals | E.3 Kontoinformation | 96 |
| HIBPA v3 | Formals | D.2 Bankparameter allgemein | 87 |
| HIRMG v2 | Formals | B.7.2 Rückmeldungen zur Gesamtnachricht | 32 |
| HIRMS v2 | Formals | B.7.3 Rückmeldungen zu Segmenten | 33 |
| Return code lists | Rückmeldungscodes; Formals B.7.5 | code tables | whole doc; Formals 35 |
| HKSPA / HISPA v1-3 | Messages 2022 | C.10.1.3 Segmentversion 1-3 | 407-414 |
| HKSAL / HISAL / HISALS 5 | HBCI 2.2 | VII.2.2; BPD format IV.6 (ohne Parameter) | 279 / 280 / 282 |
| HKKAZ / HIKAZ / HIKAZS 5 | HBCI 2.2 | VII.2.1.1; `ktv` II.5.3.3 (p.32); `sdo` II.5.3.4 (p.34) | 271 / 273 / 274 |
| HKWPD / HIWPD / HIWPDS 5 | HBCI 2.2 | VII.4.3.1 (table covers versions 4 and 5; v5 response is MT 535) | 362 / 363 / 364 |
| HKSAL / HISAL 6, 7, 8 | Messages 2022 | C.2.1.2.1 / .2 (SEPA) / .3 | 73-82 |
| HISALS 6 / 7 / 8 | Messages 2022 | C.2.1.2.x; HISALS 8 parameter DEG in D "Parameter Saldenabfrage" | 76 / 79 / 82 |
| HKKAZ / HIKAZ 6, 7 | Messages 2022 | C.2.1.1.1.1 Segmentversion 6; version 7 follows | 39-44 |
| HIKAZS 6 / 7 | Messages 2022 | C.2.1.1.x | 41 / 44 |
| HKCAZ / HICAZ 1 | Messages 2022 | C.2.3.1.1.1 Kontoumsätze/Zeitraum (camt) | 101-103 |
| HICAZS 1 | Messages 2022 | C.2.3.1.x | 104 |
| HKWPD / HIWPD 6 | Messages 2022 | Wertpapiere, Depotaufstellung (2022 doc: version 6 only) | 210 |
| HIWPDS 6 | Messages 2022 | same | 211 |
| MT940 / MT942 filling | HBCI 2.2 | IX.2.8 / IX.2.9 | — |
| MT535 / MT536 filling | HBCI 2.2 | IX.2.4 / IX.2.5 | — |
| HKWDU / HIWDU 5, HIWDUS 5 | Messages 2022 | Wertpapiere, Wertpapierdaten | 212-213 |
| HKCCS 1, HKIPZ 1/2 | Messages 2022 | C.10.2.1 Einzelüberweisung; C.10.2.9.1 | 415; 526-528 |
| HKVPP 1 / HIVPP 1 / HIVPPS 1 / HKVPA 1 | VOP 1.02 | C.10.7 Namensabgleich (Verification of Payee) | 10 / 11 / 13 / 14 |
| pain.001.001.09 / SCT Inst / VoP pain.002.001.10 / character set | Anlage 3 V26.11 | 2.2.1 / 2.2.4.1 / 2.2.6 / 2.1 (`grep -n "^2.2.6 "`) | printed 91 / 207 / 237 / 84 |

The index covers every segment listed. HKSAL 5, HKKAZ 5 and HKWPD 5 come from HBCI 2.2, not from Messages 2022.

## Bank directory

`data/fints_institutes.csv` (name, BIC and PIN/TAN URL per bank code; `build.rs` generates the directory from it) is a copy of the **FinTS-Bankenliste** of Deutsche Kreditwirtschaft, maintained by SIZ GmbH.

- **Official source.** https://www.fints.org/de/hersteller/bankenliste
- **Decision (2026-11-20).** The list stays in the repository for now.
