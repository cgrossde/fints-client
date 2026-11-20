//! Typed codec errors: which segment, which position, which rule.

use std::fmt;

/// Where in a message the tokenizer failed (byte offset into the input).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("wire syntax error at byte {offset}: {kind}")]
pub struct WireError {
    /// Byte offset into the tokenized input.
    pub offset: usize,
    pub kind: WireErrorKind,
}

/// A violation of the FinTS syntax rules (Formals H.1).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WireErrorKind {
    /// Input ends inside a segment: no segment end character `'` (H.1.2).
    #[error("segment not terminated by '")]
    UnterminatedSegment,
    /// Input ends right after the release character `?` (H.1.3).
    #[error("release character ? at end of input")]
    DanglingEscape,
    /// `@` not followed by `<digits>@` (H.1.4).
    #[error("malformed binary length")]
    BadBinaryLength,
    /// The binary length points past the end of the input (H.1.4).
    #[error("binary data of {declared} bytes runs past the end of the input")]
    BinaryOverrun { declared: usize },
    /// Binary data starts after other characters of the same element.
    #[error("binary marker @ inside a text element")]
    BinaryInsideText,
    /// Something other than a separator follows binary data.
    #[error("binary data not followed by a separator")]
    DataAfterBinary,
}

/// A rule broken by a value, found while parsing or serialising.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A mandatory element is empty or absent (Formals B.3.1, status M).
    #[error("mandatory element missing")]
    Missing,
    /// Longer than the maximum length (B.3.3, `..n`), counted before escaping.
    #[error("too long: {actual} bytes, at most {max} allowed")]
    TooLong { max: usize, actual: usize },
    /// Not the fixed length (B.3.3).
    #[error("wrong length: {actual} bytes, exactly {expected} required")]
    WrongLength { expected: usize, actual: usize },
    /// A character the format does not allow (B.4).
    #[error("character {found:?} not allowed in format {format}")]
    InvalidCharacter { format: &'static str, found: char },
    /// A character outside ISO 8859-1 (B.1); cannot be put on the wire.
    #[error("character {found:?} is outside ISO 8859-1")]
    NotLatin1 { found: char },
    /// `num`/`float` with a leading zero (B.4.1).
    #[error("leading zero not allowed in format {format}")]
    LeadingZero { format: &'static str },
    /// A number that does not fit the Rust type behind the element.
    #[error("number too large")]
    NumberTooLarge,
    /// Not a valid `dat` (B.4.2: JJJJMMTT, an existing date).
    #[error("not a valid date (JJJJMMTT)")]
    InvalidDate,
    /// Not a valid `tim` (B.4.2: hhmmss, a valid time).
    #[error("not a valid time (hhmmss)")]
    InvalidTime,
    /// Not `J` or `N` (B.4.2, `jn`).
    #[error("expected J or N")]
    InvalidYesNo,
    /// Not a valid `float`/`wrt` value (B.4.1: decimal comma obligatory).
    #[error("not a valid value with decimal comma")]
    InvalidValue,
    /// Binary data where the format is text.
    #[error("binary data where text was expected")]
    ExpectedText,
    /// Text where the format is `bin` (H.1.4).
    #[error("text where binary data was expected")]
    ExpectedBinary,
    /// Fewer repetitions than the minimum count (B.3.1).
    #[error("{actual} repetitions, at least {min} required")]
    TooFewRepetitions { min: usize, actual: usize },
    /// More repetitions than the maximum count (B.3.1).
    #[error("{actual} repetitions, at most {max} allowed")]
    TooManyRepetitions { max: usize, actual: usize },
    /// Non-empty elements after the last element the definition knows.
    #[error("unexpected data after the last defined element")]
    UnexpectedData,
    /// The segment header (`code:number:version[:reference]`) is malformed.
    #[error("malformed segment header: {0}")]
    InvalidHeader(String),
    /// The segment is a different code or version than the definition.
    #[error("expected segment {expected_code} v{expected_version}")]
    WrongSegment {
        expected_code: &'static str,
        expected_version: u16,
    },
    /// No definition for this code and version is registered.
    #[error("no definition for this segment code and version")]
    UnknownSegment,
    /// The message could not be tokenized.
    #[error(transparent)]
    Wire(#[from] WireError),
}

/// The segment an error belongs to, as read from its header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentId {
    pub code: String,
    pub version: u16,
    /// Segment number from the header (position in the message).
    pub number: u16,
}

/// A codec error with its location.
///
/// `de` counts data elements of the segment from 1, the segment header being
/// DE 1, as the "Nr." column of the specification tables does. `element`
/// counts the group data elements (GD) inside that DE from 1, after nested
/// groups are flattened, which is the colon-separated position on the wire.
/// `field` is the Rust field path, e.g. `parameters.methods[2].name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError {
    pub segment: Option<SegmentId>,
    pub de: Option<usize>,
    pub element: Option<usize>,
    pub field: String,
    pub kind: ErrorKind,
}

impl CodecError {
    /// A new error without location.
    pub fn new(kind: ErrorKind) -> Self {
        CodecError {
            segment: None,
            de: None,
            element: None,
            field: String::new(),
            kind,
        }
    }

    /// Prepends a field name to the field path.
    pub fn in_field(mut self, name: &str) -> Self {
        if self.field.is_empty() {
            self.field = name.to_string();
        } else if self.field.starts_with('[') {
            self.field = format!("{name}{}", self.field);
        } else {
            self.field = format!("{name}.{}", self.field);
        }
        self
    }

    /// Prepends a repetition index to the field path.
    pub fn in_index(mut self, index: usize) -> Self {
        if self.field.is_empty() || self.field.starts_with('[') {
            self.field = format!("[{index}]{}", self.field);
        } else {
            self.field = format!("[{index}].{}", self.field);
        }
        self
    }

    /// Sets the 1-based group element position unless already set.
    pub fn at_element(mut self, element: usize) -> Self {
        self.element.get_or_insert(element);
        self
    }

    /// Sets the 1-based data element position unless already set.
    pub fn at_de(mut self, de: usize) -> Self {
        self.de.get_or_insert(de);
        self
    }

    /// Sets the segment unless already set.
    pub fn in_segment(mut self, id: SegmentId) -> Self {
        self.segment.get_or_insert(id);
        self
    }
}

impl From<ErrorKind> for CodecError {
    fn from(kind: ErrorKind) -> Self {
        CodecError::new(kind)
    }
}

impl From<WireError> for CodecError {
    fn from(e: WireError) -> Self {
        CodecError::new(ErrorKind::Wire(e))
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(seg) = &self.segment {
            write!(f, "{} v{} (segment {})", seg.code, seg.version, seg.number)?;
        } else {
            f.write_str("segment")?;
        }
        if let Some(de) = self.de {
            write!(f, ", DE {de}")?;
        }
        if let Some(el) = self.element {
            write!(f, ", element {el}")?;
        }
        if !self.field.is_empty() {
            write!(f, " ({})", self.field)?;
        }
        write!(f, ": {}", self.kind)
    }
}

impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Wire(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_paths_compose_from_the_inside_out() {
        let e = CodecError::new(ErrorKind::Missing)
            .in_field("name")
            .in_index(2)
            .in_field("methods")
            .in_field("parameters");
        assert_eq!(e.field, "parameters.methods[2].name");
    }

    #[test]
    fn display_names_segment_position_field_and_rule() {
        let e = CodecError::new(ErrorKind::TooLong {
            max: 30,
            actual: 31,
        })
        .at_element(12)
        .in_field("name")
        .at_de(5)
        .in_segment(SegmentId {
            code: "HITANS".into(),
            version: 6,
            number: 4,
        });
        assert_eq!(
            e.to_string(),
            "HITANS v6 (segment 4), DE 5, element 12 (name): too long: 31 bytes, at most 30 allowed"
        );
    }
}
