//! A static description of each derived definition: its elements in wire
//! order, their formats, length rules and repetition counts.
//!
//! The derives generate it next to the parse and serialise code, from the
//! same field list and attributes, so it cannot drift from what the codec
//! does. It changes nothing on the wire. Uses: comparing the definitions
//! with other descriptions of FinTS, documentation and tooling.
//!
//! ```
//! use fints_client::codec::{self, ElementKind};
//!
//! let hksal = codec::lookup("HKSAL", 7).unwrap();
//! let account = &hksal.schema()[0];
//! assert_eq!(account.name, "account");
//! assert!(matches!(account.kind, ElementKind::Group { name: "Kti1", .. }));
//! assert_eq!((account.min_count, account.max_count), (1, Some(1)));
//! ```

/// One field of a segment or data element group, in wire order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSchema {
    /// Rust field name.
    pub name: &'static str,
    /// Data element format or nested group.
    pub kind: ElementKind,
    /// Maximum length in bytes (`#[de(max = n)]`); per repetition.
    pub max: Option<usize>,
    /// Fixed length in bytes (`#[de(len = n)]`); per repetition.
    pub len: Option<usize>,
    /// Minimum number of occurrences: 1 for `T`, 0 for `Option<T>`,
    /// `min_count` for `Vec<T>`.
    pub min_count: usize,
    /// Maximum number of occurrences: `Some(1)` for `T` and `Option<T>`,
    /// `max_count` for `Vec<T>` (`None`: unbounded).
    pub max_count: Option<usize>,
    /// True for a `Vec<T>` field.
    pub repeated: bool,
}

impl FieldSchema {
    /// True if the element may be absent (`Option<T>`, or `Vec<T>` with
    /// `min_count = 0`).
    pub const fn optional(&self) -> bool {
        self.min_count == 0
    }
}

/// What a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    /// A data element; `format` is [`Scalar::FORMAT`](crate::codec::Scalar::FORMAT)
    /// (`an`, `num`, `jn`, ...).
    Scalar { format: &'static str },
    /// A data element group: the Rust type name and its fields.
    Group {
        name: &'static str,
        fields: &'static [FieldSchema],
    },
}
