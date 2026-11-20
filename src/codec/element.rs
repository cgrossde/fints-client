//! The generic element machinery behind the derives.
//!
//! Every data element type and every derived data element group implements
//! [`Element`]: it reads itself from a [`Cursor`] over the flat,
//! colon-separated group elements of one data element, and writes itself as
//! a run of [`DataElement`]s. A nested group is flattened (Formals H.1.2,
//! "Verwendung von Datenelementgruppen innerhalb von Datenelementgruppen").
//!
//! Positions are what make the format work:
//!
//! - An element of fixed [`Element::WIDTH`] always occupies exactly that many
//!   positions. An absent optional element writes that many empty positions,
//!   so everything after it stays in place (H.1.5, "Auslassen von
//!   Datenelementen").
//! - A repeated element with a maximum count occupies `max × width`
//!   positions; unused repetitions are empty positions (H.1.2,
//!   "Mehrfach auftretende optionale Datenstrukturen").
//! - Only at the end of a data element (and of the segment) are empty
//!   positions cut off, by the serializer (H.1.5, "Abschneiden"). When
//!   reading, a position past the end reads as empty, so truncated and
//!   untruncated input are both accepted (H.1.5, hint to receivers).
//! - A repeated element without a maximum count has open width and must be
//!   the last element of its group; the derive checks this at compile time.

use crate::codec::error::{CodecError, ErrorKind};
use crate::codec::schema::ElementKind;
use crate::codec::wire::is_blank_group;
use crate::parser::{DEG, DataElement};

/// Length rules from a field's `#[de(max = n)]` or `#[de(len = n)]`
/// (Formals B.3.3), counted in bytes before escaping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rules {
    /// Maximum length (`..n` in the specification tables).
    pub max: Option<usize>,
    /// Fixed length (`n` in the specification tables).
    pub len: Option<usize>,
}

impl Rules {
    /// No length rules.
    pub const NONE: Rules = Rules {
        max: None,
        len: None,
    };

    /// Checks a byte length against these rules.
    pub fn check(&self, actual: usize) -> Result<(), ErrorKind> {
        if let Some(expected) = self.len
            && actual != expected
        {
            return Err(ErrorKind::WrongLength { expected, actual });
        }
        if let Some(max) = self.max
            && actual > max
        {
            return Err(ErrorKind::TooLong { max, actual });
        }
        Ok(())
    }
}

static EMPTY: DataElement = DataElement::Empty;

/// Reads the flat group elements of one data element, left to right.
///
/// Positions past the end read as [`DataElement::Empty`].
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    elements: &'a [DataElement],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(elements: &'a [DataElement]) -> Self {
        Cursor { elements, pos: 0 }
    }

    /// 0-based position of the next element.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Returns the next element and advances.
    #[allow(clippy::should_implement_trait)] // never ends, so not an Iterator
    pub fn next(&mut self) -> &'a DataElement {
        let de = self.elements.get(self.pos).unwrap_or(&EMPTY);
        self.pos = self.pos.saturating_add(1);
        de
    }

    /// Skips `n` positions.
    pub fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n);
    }

    /// True if the next `n` positions are all empty.
    pub fn next_empty(&self, n: usize) -> bool {
        let end = self.pos.saturating_add(n).min(self.elements.len());
        self.elements
            .get(self.pos..end)
            .is_none_or(|s| s.iter().all(blank))
    }

    /// True if every remaining position is empty.
    pub fn rest_empty(&self) -> bool {
        self.elements
            .get(self.pos..)
            .is_none_or(|s| s.iter().all(blank))
    }

    /// Moves to the end.
    pub fn skip_rest(&mut self) {
        self.pos = self.pos.max(self.elements.len());
    }
}

fn blank(de: &DataElement) -> bool {
    match de {
        DataElement::Empty => true,
        DataElement::Text(s) => s.is_empty(),
        DataElement::Binary(_) => false,
    }
}

/// A data element or data element group that can be read and written.
///
/// Implemented by the data element types in [`crate::codec::types`], by
/// `Option<T>`, and by `#[derive(DataElementGroup)]`.
pub trait Element: Sized {
    /// Number of flat wire positions; `None` for a group that ends in a
    /// repetition without maximum count.
    const WIDTH: Option<usize>;
    /// True for data elements (which take length rules), false for groups.
    const SCALAR: bool;
    /// Description of the element: its format, or its group fields.
    const KIND: ElementKind;
    /// True for `Option<T>`.
    const OPTIONAL: bool = false;

    /// Reads the value at the cursor. Empty mandatory elements are an error;
    /// optional elements are `Option<T>`.
    fn decode(cur: &mut Cursor<'_>, rules: &Rules) -> Result<Self, CodecError>;

    /// Appends the value's wire elements, exactly `WIDTH` of them when the
    /// width is fixed.
    fn encode(&self, out: &mut Vec<DataElement>, rules: &Rules) -> Result<(), CodecError>;
}

/// An optional element (status O or C in the specification tables).
///
/// Absent when all its positions are empty. Present but incomplete is an
/// error of the inner element (a group with status O whose mandatory
/// elements are missing, Formals B.3.1).
impl<T: Element> Element for Option<T> {
    const WIDTH: Option<usize> = T::WIDTH;
    const SCALAR: bool = T::SCALAR;
    const KIND: ElementKind = T::KIND;
    const OPTIONAL: bool = true;

    fn decode(cur: &mut Cursor<'_>, rules: &Rules) -> Result<Self, CodecError> {
        match T::WIDTH {
            Some(width) if cur.next_empty(width) => {
                cur.skip(width);
                Ok(None)
            }
            None if cur.rest_empty() => {
                cur.skip_rest();
                Ok(None)
            }
            _ => T::decode(cur, rules).map(Some),
        }
    }

    fn encode(&self, out: &mut Vec<DataElement>, rules: &Rules) -> Result<(), CodecError> {
        match self {
            Some(value) => value.encode(out, rules),
            None => {
                pad(out, T::WIDTH.unwrap_or(0));
                Ok(())
            }
        }
    }
}

fn pad(out: &mut Vec<DataElement>, n: usize) {
    out.extend(std::iter::repeat_n(DataElement::Empty, n));
}

/// Reads a repeated element inside a group: up to `max` repetitions (all
/// remaining ones without `max`), skipping empty repetitions.
pub fn decode_repeated<T: Element>(
    cur: &mut Cursor<'_>,
    rules: &Rules,
    min: usize,
    max: Option<usize>,
) -> Result<Vec<T>, CodecError> {
    if T::WIDTH == Some(0) {
        check_count(0, min, max)?;
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut slots = 0usize;
    while max.is_none_or(|m| slots < m) && !cur.rest_empty() {
        slots += 1;
        if let Some(width) = T::WIDTH
            && cur.next_empty(width)
        {
            cur.skip(width);
            continue;
        }
        let before = cur.position();
        let item = T::decode(cur, rules).map_err(|e| e.in_index(items.len()))?;
        if cur.position() == before {
            // A zero-width element would never end the loop.
            break;
        }
        items.push(item);
    }
    // The loop stops early only when the rest is empty: unused repetitions
    // at the end were cut off, which H.1.5 allows.
    check_count(items.len(), min, max)?;
    Ok(items)
}

/// Writes a repeated element inside a group, padded to `max` repetitions.
pub fn encode_repeated<T: Element>(
    items: &[T],
    out: &mut Vec<DataElement>,
    rules: &Rules,
    min: usize,
    max: Option<usize>,
) -> Result<(), CodecError> {
    check_count(items.len(), min, max)?;
    for (i, item) in items.iter().enumerate() {
        item.encode(out, rules).map_err(|e| e.in_index(i))?;
    }
    if let (Some(m), Some(w)) = (max, T::WIDTH) {
        pad(out, (m - items.len()).saturating_mul(w));
    }
    Ok(())
}

fn check_count(actual: usize, min: usize, max: Option<usize>) -> Result<(), CodecError> {
    if actual < min {
        return Err(ErrorKind::TooFewRepetitions { min, actual }.into());
    }
    if let Some(max) = max
        && actual > max
    {
        return Err(ErrorKind::TooManyRepetitions { max, actual }.into());
    }
    Ok(())
}

/// Width of a group: the sum of its fields' widths. Panics (at compile time,
/// where the derive evaluates it) if an open-width field is not the last.
pub const fn seq_width(widths: &[Option<usize>]) -> Option<usize> {
    let mut total = 0usize;
    let mut i = 0;
    while i < widths.len() {
        match widths[i] {
            Some(w) => total += w,
            None => {
                if i + 1 != widths.len() {
                    panic!(
                        "fints-client: a repeated element without max_count (or a group ending \
                         in one) must be the last element of its group"
                    );
                }
                return None;
            }
        }
        i += 1;
    }
    Some(total)
}

/// Width of a repeated field: `max × width`, open without `max`.
pub const fn repeat_width(width: Option<usize>, max: Option<usize>) -> Option<usize> {
    match (width, max) {
        (Some(w), Some(m)) => Some(w * m),
        _ => None,
    }
}

/// Compile-time check behind `#[de(max)]` / `#[de(len)]` on a group.
pub const fn assert_length_rules_allowed(scalar: bool) {
    if !scalar {
        panic!("fints-client: `max`/`len` apply to data elements, not to data element groups");
    }
}

/// Reads one data element per field from a segment body (the data elements
/// after the header).
#[derive(Debug)]
pub struct SlotReader<'a> {
    body: &'a [DEG],
    next: usize,
}

static NO_ELEMENTS: [DataElement; 0] = [];

impl<'a> SlotReader<'a> {
    pub fn new(body: &'a [DEG]) -> Self {
        SlotReader { body, next: 0 }
    }

    /// Position in specification terms: the header is DE 1.
    fn de_number(index: usize) -> usize {
        index.saturating_add(2)
    }

    fn slot(&self, index: usize) -> &'a [DataElement] {
        self.body
            .get(index)
            .map_or(&NO_ELEMENTS[..], |d| d.0.as_slice())
    }

    fn decode_at<T: Element>(&self, index: usize, rules: &Rules) -> Result<T, CodecError> {
        let mut cur = Cursor::new(self.slot(index));
        let value = T::decode(&mut cur, rules)?;
        if !cur.rest_empty() {
            return Err(CodecError::new(ErrorKind::UnexpectedData)
                .at_element(cur.position().saturating_add(1)));
        }
        Ok(value)
    }

    /// Reads a field that is one data element (or one group, or `Option`).
    pub fn single<T: Element>(&mut self, rules: &Rules) -> Result<T, CodecError> {
        let index = self.next;
        self.next += 1;
        self.decode_at(index, rules)
            .map_err(|e| e.at_de(Self::de_number(index)))
    }

    /// Reads a repeated field: `max` data elements, or all remaining ones if
    /// it is the last field. Empty data elements are absent repetitions.
    pub fn repeated<T: Element>(
        &mut self,
        rules: &Rules,
        min: usize,
        max: Option<usize>,
        is_last: bool,
    ) -> Result<Vec<T>, CodecError> {
        let start = self.next;
        let end = if is_last {
            self.body.len().max(start)
        } else {
            start.saturating_add(max.unwrap_or(0))
        };
        self.next = end;
        let mut items = Vec::new();
        for index in start..end {
            if self.body.get(index).is_none_or(is_blank_group) {
                continue;
            }
            let item = self
                .decode_at(index, rules)
                .map_err(|e| e.in_index(items.len()).at_de(Self::de_number(index)))?;
            items.push(item);
        }
        check_count(items.len(), min, max).map_err(|e| e.at_de(Self::de_number(start)))?;
        Ok(items)
    }

    /// Fails if data elements beyond the definition carry data.
    pub fn finish(self) -> Result<(), CodecError> {
        match self
            .body
            .iter()
            .enumerate()
            .skip(self.next)
            .find(|(_, d)| !is_blank_group(d))
        {
            Some((index, _)) => {
                Err(CodecError::new(ErrorKind::UnexpectedData).at_de(Self::de_number(index)))
            }
            None => Ok(()),
        }
    }
}

/// Writes one data element per field of a segment body.
#[derive(Debug, Default)]
pub struct SlotWriter {
    body: Vec<DEG>,
}

impl SlotWriter {
    pub fn new() -> Self {
        SlotWriter::default()
    }

    fn de_number(&self) -> usize {
        self.body.len().saturating_add(2)
    }

    fn encode_one<T: Element>(value: &T, rules: &Rules) -> Result<DEG, CodecError> {
        let mut out = Vec::new();
        value.encode(&mut out, rules)?;
        if out.is_empty() {
            out.push(DataElement::Empty);
        }
        Ok(DEG(out))
    }

    /// Writes a field that is one data element (or one group, or `Option`).
    pub fn single<T: Element>(&mut self, value: &T, rules: &Rules) -> Result<(), CodecError> {
        let de = self.de_number();
        let deg = Self::encode_one(value, rules).map_err(|e| e.at_de(de))?;
        self.body.push(deg);
        Ok(())
    }

    /// Writes a repeated field, one data element per repetition, padded with
    /// empty data elements to `max` unless it is the last field.
    pub fn repeated<T: Element>(
        &mut self,
        items: &[T],
        rules: &Rules,
        min: usize,
        max: Option<usize>,
        is_last: bool,
    ) -> Result<(), CodecError> {
        check_count(items.len(), min, max).map_err(|e| e.at_de(self.de_number()))?;
        for (i, item) in items.iter().enumerate() {
            let de = self.de_number();
            let deg = Self::encode_one(item, rules).map_err(|e| e.in_index(i).at_de(de))?;
            self.body.push(deg);
        }
        if !is_last && let Some(m) = max {
            for _ in items.len()..m {
                self.body.push(DEG(vec![DataElement::Empty]));
            }
        }
        Ok(())
    }

    /// The written data elements (without the header).
    pub fn finish(self) -> Vec<DEG> {
        self.body
    }
}
