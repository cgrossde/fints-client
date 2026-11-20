//! Placeholders that a bank sends instead of a reference value.

/// Whether a reference value means "no value".
///
/// True for the empty string, `NOTPROVIDED` (the EPC placeholder for a
/// missing end-to-end ID) and `NONREF` (the SWIFT "no reference" value).
/// The value is trimmed and compared case-insensitively.
///
/// Use it for reference values only (`EREF`, `KREF`, `MREF`, `CRED`, `DEBT`,
/// CAMT `EndToEndId`/`MndtId`). For text values (`SVWZ`, `ABWA`, `ABWE`)
/// only the empty string means absent.
pub(crate) fn absent(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || value.eq_ignore_ascii_case("NOTPROVIDED")
        || value.eq_ignore_ascii_case("NONREF")
}

#[cfg(test)]
mod tests {
    use super::absent;

    #[test]
    fn placeholders_are_absent() {
        for value in [
            "",
            "   ",
            "NOTPROVIDED",
            "notprovided",
            " NONREF ",
            "NonRef",
        ] {
            assert!(absent(value), "{value:?}");
        }
    }

    #[test]
    fn real_values_are_present() {
        for value in ["REF-0001", "NOTPROVIDED1", "NON REF", "0"] {
            assert!(!absent(value), "{value:?}");
        }
    }
}
