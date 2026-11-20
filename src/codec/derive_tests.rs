//! Tests of the derive macros on synthetic definitions: positions,
//! optional elements, repetition, error locations.

use proptest::prelude::*;

use crate::codec::{
    An, CodecError, DataElementGroup, ErrorKind, Jn, Num, Segment, SegmentHeader, from_wire,
    to_wire,
};

fn wire<T: crate::codec::Segment>(value: &T) -> String {
    String::from_utf8(to_wire(value, 1, None).unwrap()).unwrap()
}

fn parse<T: crate::codec::Segment>(s: &str) -> Result<T, CodecError> {
    from_wire::<T>(s.as_bytes()).map(|(_, v)| v)
}

/// Formals H.1.2, "Mehrfach auftretende optionale Datenstrukturen":
/// DE1 (O, 5) and DE2 (O, 4), each given twice, is
/// `<Segmentkopf>+DE1+DE1++++DE2+DE2'`.
#[derive(Debug, Clone, PartialEq, Segment)]
#[segment(code = "XTEST", version = 1)]
struct RepeatExample {
    #[de(max_count = 5)]
    de1: Vec<An>,
    #[de(max_count = 4)]
    de2: Vec<An>,
}

#[test]
fn formals_h12_repeated_optional_elements_keep_their_positions() {
    let v = RepeatExample {
        de1: vec![An::new("a").unwrap(), An::new("b").unwrap()],
        de2: vec![An::new("c").unwrap(), An::new("d").unwrap()],
    };
    assert_eq!(wire(&v), "XTEST:1:1+a+b++++c+d'");
    assert_eq!(parse::<RepeatExample>("XTEST:1:1+a+b++++c+d'").unwrap(), v);
    // Too many repetitions are an error on both sides.
    let too_many = RepeatExample {
        de1: vec![An::new("a").unwrap(); 6],
        de2: vec![],
    };
    let e = to_wire(&too_many, 1, None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::TooManyRepetitions { max: 5, actual: 6 });
    assert_eq!((e.de, e.field.as_str()), (Some(2), "de1"));
    let e = parse::<RepeatExample>("XTEST:1:1+a+b+c+d+e+f+g+h+i+j'").unwrap_err();
    assert_eq!(e.kind, ErrorKind::TooManyRepetitions { max: 4, actual: 5 });
    assert_eq!(e.field, "de2");
}

#[derive(Debug, Clone, PartialEq, DataElementGroup)]
struct Inner {
    a: Num,
    b: Option<An>,
}

/// A group with an optional nested group in the middle, a repeated group
/// with maximum count in the middle, and an open repetition at the end.
#[derive(Debug, Clone, PartialEq, DataElementGroup)]
struct Outer {
    first: Jn,
    nested: Option<Inner>,
    #[de(min_count = 1, max_count = 2)]
    pairs: Vec<Inner>,
    #[de(max = 3)]
    tail: Vec<An>,
}

#[derive(Debug, Clone, PartialEq, Segment)]
#[segment(code = "XOUTER", version = 2)]
struct OuterSeg {
    #[de(max = 4)]
    count: Num,
    middle: Option<An>,
    group: Outer,
}

fn outer(nested: Option<Inner>, pairs: Vec<Inner>, tail: &[&str]) -> OuterSeg {
    OuterSeg {
        count: Num::new(7),
        middle: None,
        group: Outer {
            first: Jn::YES,
            nested,
            pairs,
            tail: tail.iter().map(|s| An::new(s).unwrap()).collect(),
        },
    }
}

fn inner(a: u64, b: Option<&str>) -> Inner {
    Inner {
        a: Num::new(a),
        b: b.map(|s| An::new(s).unwrap()),
    }
}

#[test]
fn nested_groups_are_flattened_with_fixed_positions() {
    let cases = [
        (
            outer(Some(inner(1, Some("x"))), vec![inner(2, None)], &["t"]),
            "XOUTER:1:2+7++J:1:x:2::::t'",
        ),
        (
            outer(None, vec![inner(2, Some("y")), inner(3, None)], &[]),
            "XOUTER:1:2+7++J:::2:y:3'",
        ),
        (
            outer(None, vec![inner(2, None)], &["t", "u", "v"]),
            "XOUTER:1:2+7++J:::2::::t:u:v'",
        ),
    ];
    for (value, expected) in cases {
        assert_eq!(wire(&value), expected);
        assert_eq!(parse::<OuterSeg>(expected).unwrap(), value, "{expected}");
    }
}

#[test]
fn untruncated_input_is_accepted() {
    // Formals H.1.5: the receiver accepts both the cut and the uncut form.
    let v = parse::<OuterSeg>("XOUTER:1:2+7++J:::2:::::+++'").unwrap();
    assert_eq!(v, outer(None, vec![inner(2, None)], &[]));
}

#[test]
fn errors_name_position_and_field() {
    // A present nested group with its mandatory element missing.
    let e = parse::<OuterSeg>("XOUTER:1:2+7++J::x:2'").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Missing);
    assert_eq!((e.de, e.element), (Some(4), Some(2)));
    assert_eq!(e.field, "group.nested.a");
    assert_eq!(e.segment.as_ref().map(|s| s.code.as_str()), Some("XOUTER"));

    let e = parse::<OuterSeg>("XOUTER:1:2+7++J:::2::x'").unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::InvalidCharacter {
            format: "num",
            found: 'x'
        }
    );
    assert_eq!((e.de, e.element), (Some(4), Some(6)));
    assert_eq!(e.field, "group.pairs[1].a");

    let e = parse::<OuterSeg>("XOUTER:1:2+12345++J:::2'").unwrap_err();
    assert_eq!(e.kind, ErrorKind::TooLong { max: 4, actual: 5 });
    assert_eq!((e.de, e.field.as_str()), (Some(2), "count"));

    let e = parse::<OuterSeg>("XOUTER:1:2+7++J:::'").unwrap_err();
    assert_eq!(e.kind, ErrorKind::TooFewRepetitions { min: 1, actual: 0 });
    assert_eq!(e.field, "group.pairs");

    let e = parse::<OuterSeg>("XOUTER:1:2+7:8++J:::2'").unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnexpectedData);
    assert_eq!((e.de, e.element), (Some(2), Some(2)));

    let e = parse::<OuterSeg>("XOUTER:1:2+7++J:::2+extra'").unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnexpectedData);
    assert_eq!(e.de, Some(5));

    let e = parse::<OuterSeg>("XOUTER:1:3+7++J:::2'").unwrap_err();
    assert!(matches!(e.kind, ErrorKind::WrongSegment { .. }));
}

#[test]
fn encode_errors_name_position_and_field() {
    let mut v = outer(None, vec![inner(2, None)], &[]);
    v.count = Num::new(12345);
    let e = to_wire(&v, 1, None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::TooLong { max: 4, actual: 5 });
    assert_eq!((e.de, e.field.as_str()), (Some(2), "count"));

    let v = outer(None, vec![], &[]);
    let e = to_wire(&v, 1, None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::TooFewRepetitions { min: 1, actual: 0 });
    assert_eq!((e.de, e.field.as_str()), (Some(4), "group.pairs"));
}

/// A segment without elements after the header.
#[derive(Debug, Clone, PartialEq, Segment)]
#[segment(code = "XEMPTY", version = 1)]
struct Empty;

#[test]
fn unit_segment() {
    assert_eq!(wire(&Empty), "XEMPTY:1:1'");
    assert_eq!(parse::<Empty>("XEMPTY:1:1'").unwrap(), Empty);
    assert_eq!(
        parse::<Empty>("XEMPTY:1:1+x'").unwrap_err().kind,
        ErrorKind::UnexpectedData
    );
}

#[test]
fn header_number_and_reference_come_from_the_framework() {
    let (h, _) = from_wire::<Empty>(b"XEMPTY:7:1:3'").unwrap();
    assert_eq!(
        h,
        SegmentHeader {
            code: "XEMPTY".into(),
            number: 7,
            version: 1,
            reference: Some(3)
        }
    );
    assert_eq!(to_wire(&Empty, 7, Some(3)).unwrap(), b"XEMPTY:7:1:3'");
}

fn arb_an() -> impl Strategy<Value = An> {
    "[a-zA-Z0-9+:'?@ ]{1,3}".prop_filter_map("trimmed", |s| {
        An::new(&s).ok().filter(|a| !a.as_str().is_empty())
    })
}

fn arb_inner() -> impl Strategy<Value = Inner> {
    (any::<u16>(), proptest::option::of(arb_an())).prop_map(|(a, b)| Inner {
        a: Num::new(u64::from(a)),
        b,
    })
}

fn arb_outer() -> impl Strategy<Value = OuterSeg> {
    (
        0u64..10_000,
        proptest::option::of(arb_an()),
        any::<bool>(),
        proptest::option::of(arb_inner()),
        proptest::collection::vec(arb_inner(), 1..=2),
        proptest::collection::vec(arb_an(), 0..5),
    )
        .prop_map(|(count, middle, first, nested, pairs, tail)| OuterSeg {
            count: Num::new(count),
            middle,
            group: Outer {
                first: Jn(first),
                nested,
                pairs,
                tail,
            },
        })
}

proptest! {
    #[test]
    fn derived_definitions_round_trip(v in arb_outer()) {
        let bytes = to_wire(&v, 1, None).unwrap();
        let (_, back) = from_wire::<OuterSeg>(&bytes).unwrap();
        prop_assert_eq!(&back, &v);
        prop_assert_eq!(to_wire(&back, 1, None).unwrap(), bytes);
    }

    #[test]
    fn derived_parser_never_panics(input in proptest::collection::vec(
        prop_oneof![Just(b'+'), Just(b':'), Just(b'?'), Just(b'@'), Just(b'J'),
                    Just(b'1'), Just(b'x'), Just(b'0')], 0..48)) {
        let mut bytes = b"XOUTER:1:2+".to_vec();
        bytes.extend(input);
        bytes.push(b'\'');
        let _ = from_wire::<OuterSeg>(&bytes);
        let _ = from_wire::<RepeatExample>(&bytes);
    }
}

/// The generated schema mirrors the field list and attributes.
#[test]
fn schema_describes_fields_in_order() {
    use crate::codec::{Element, ElementKind, FieldSchema};

    let seg = <OuterSeg as Segment>::SCHEMA;
    let names: Vec<_> = seg.iter().map(|f| f.name).collect();
    assert_eq!(names, ["count", "middle", "group"]);
    assert_eq!(seg[0].kind, ElementKind::Scalar { format: "num" });
    assert_eq!((seg[0].max, seg[0].len), (Some(4), None));
    assert_eq!((seg[0].min_count, seg[0].max_count), (1, Some(1)));
    assert!(seg[1].optional() && !seg[1].repeated);
    let ElementKind::Group { name, fields } = seg[2].kind else {
        panic!("group expected");
    };
    assert_eq!(name, "Outer");
    assert_eq!(seg[2].kind, <Outer as Element>::KIND);
    assert_eq!(fields.len(), 4);
    let pairs: &FieldSchema = &fields[2];
    assert_eq!(
        (pairs.min_count, pairs.max_count, pairs.repeated),
        (1, Some(2), true)
    );
    let tail = &fields[3];
    assert_eq!(
        (tail.min_count, tail.max_count, tail.max),
        (0, None, Some(3))
    );
    assert!(fields[1].optional());
    assert!(matches!(
        fields[1].kind,
        ElementKind::Group { name: "Inner", .. }
    ));
    assert!(
        <RepeatExample as Segment>::SCHEMA
            .iter()
            .all(|f| f.max_count == Some(5) || f.max_count == Some(4))
    );
}
