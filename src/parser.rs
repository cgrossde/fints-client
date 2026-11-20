//! Values for the three nesting levels of a FinTS message (Formals B.1):
//! segments end with `'`, data element groups inside a segment are split by
//! `+`, and data elements inside a group by `:`. `codec::wire` turns bytes
//! into these values and back.

use serde::{Deserialize, Serialize};

/// One data element (DE) of a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DataElement {
    /// Nothing between the separators: the element was left out.
    Empty,
    /// Alphanumeric content, already unescaped and decoded from the wire
    /// character set.
    Text(String),
    /// Content of a binary element (`@len@bytes` on the wire), without the
    /// length prefix.
    Binary(Vec<u8>),
}

impl DataElement {
    /// Text content; `None` for empty and binary elements.
    pub fn as_str(&self) -> Option<&str> {
        if let Self::Text(text) = self {
            Some(text)
        } else {
            None
        }
    }

    /// Content as an owned string. Empty elements give `""`; binary
    /// elements are read as Windows-1252, the superset of ISO 8859-1 the
    /// FinTS character set is taken from.
    pub fn as_text(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(text) => text.clone(),
            Self::Binary(bytes) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
        }
    }

    /// Bytes of a binary element; `None` for empty and text elements.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        if let Self::Binary(bytes) = self {
            Some(bytes)
        } else {
            None
        }
    }

    /// True when the element was left out.
    pub fn is_empty(&self) -> bool {
        *self == Self::Empty
    }
}

/// A data element group (DEG): the elements between two `+`. A plain data
/// element at segment level is a group with one element.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
pub struct DEG(pub Vec<DataElement>);

impl DEG {
    /// Element at `index`; [`DataElement::Empty`] past the end, because the
    /// wire format drops trailing empty elements.
    pub fn get(&self, index: usize) -> &DataElement {
        const MISSING: &DataElement = &DataElement::Empty;
        self.0.get(index).unwrap_or(MISSING)
    }

    /// Element at `index` as text, `""` when absent (see [`DataElement::as_text`]).
    pub fn get_str(&self, index: usize) -> String {
        self.get(index).as_text()
    }

    /// Number of elements actually present on the wire.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the group carries no content: no elements, or only empty ones.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(DataElement::is_empty)
    }
}

/// A segment as a list of groups. Group 0 is the segment head
/// (Formals B.5.1: code, number, version, optional reference segment);
/// the data groups follow from index 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSegment {
    pub degs: Vec<DEG>,
}

/// Positions inside the segment head.
const HEAD_CODE: usize = 0;
const HEAD_NUMBER: usize = 1;
const HEAD_VERSION: usize = 2;
const HEAD_REFERENCE: usize = 3;

impl RawSegment {
    /// Text of one head element, if present.
    fn head(&self, position: usize) -> Option<&str> {
        self.degs.first()?.0.get(position)?.as_str()
    }

    /// Numeric head element; `None` when absent or not a number.
    fn head_number(&self, position: usize) -> Option<u16> {
        self.head(position)?.parse().ok()
    }

    /// Segment code such as `HNHBK` or `HIRMS`; `""` if the head is malformed.
    pub fn segment_type(&self) -> &str {
        self.head(HEAD_CODE).unwrap_or_default()
    }

    /// Position of the segment in its message; 0 if missing or malformed.
    pub fn segment_number(&self) -> u16 {
        self.head_number(HEAD_NUMBER).unwrap_or(0)
    }

    /// Segment version; 0 if missing or malformed.
    pub fn segment_version(&self) -> u16 {
        self.head_number(HEAD_VERSION).unwrap_or(0)
    }

    /// Number of the request segment a bank answer segment refers to.
    pub fn segment_reference(&self) -> Option<u16> {
        self.head_number(HEAD_REFERENCE)
    }

    /// Group at `index` (0 is the head); an empty group past the end.
    pub fn deg(&self, index: usize) -> &DEG {
        static NONE: DEG = DEG(Vec::new());
        self.degs.get(index).unwrap_or(&NONE)
    }

    /// Number of groups including the head.
    pub fn deg_count(&self) -> usize {
        self.degs.len()
    }
}
