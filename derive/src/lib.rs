//! Derive macros for fints-client's declarative FinTS segments.
//!
//! `#[derive(Segment)]` and `#[derive(DataElementGroup)]` generate the parse
//! and serialise code for one segment version or one data element group. The
//! generated code calls into `fints_client::codec`; the rules it implements
//! (positions, optional elements, repetition, trailing elements) are
//! documented there and in `docs/codec.md`.
//!
//! Attributes:
//!
//! - on a segment struct: `#[segment(code = "HKSAL", version = 7)]`
//! - on a field: `#[de(max = 35)]` (maximum length), `#[de(len = 12)]`
//!   (fixed length), `#[de(min_count = 1, max_count = 98)]` (repetition,
//!   field type `Vec<T>`).
//!
//! Optional elements are `Option<T>` fields; nothing else marks them.
//!
//! Both derives also generate a static description of the definition
//! (`Segment::SCHEMA`, `Element::KIND`; `fints_client::codec::schema`).

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, LitInt, LitStr, PathArguments, Type,
    parse_macro_input,
};

/// Derives `fints_client::codec::Segment` for one segment version.
///
/// The struct lists the data elements after the segment header, in wire
/// order. The header (`code:number:version[:reference]`) is handled by the
/// codec and is not a field.
#[proc_macro_derive(Segment, attributes(segment, de))]
pub fn derive_segment(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_segment(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derives `fints_client::codec::Element` for a data element group (DEG).
///
/// The fields are the group's elements in wire order. A group nested in
/// another group is flattened on the wire (`a:b:c:d`), as FinTS requires.
#[proc_macro_derive(DataElementGroup, attributes(de))]
pub fn derive_data_element_group(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_group(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// One field of a derived struct, with its `#[de(...)]` attributes.
struct FieldSpec {
    ident: Ident,
    ty: Type,
    /// `Some(T)` for a `Vec<T>` field (a repeated element).
    repeated: Option<Type>,
    max: Option<usize>,
    len: Option<usize>,
    min_count: Option<usize>,
    max_count: Option<usize>,
    span: Span,
}

impl FieldSpec {
    fn name(&self) -> String {
        self.ident.to_string()
    }

    fn rules(&self) -> TokenStream2 {
        let max = opt_usize(self.max);
        let len = opt_usize(self.len);
        quote! { __p::Rules { max: #max, len: #len } }
    }

    /// The type that carries the length rules: `T` for `Vec<T>`, else the field type.
    fn element_type(&self) -> &Type {
        self.repeated.as_ref().unwrap_or(&self.ty)
    }

    /// Compile-time check that `max`/`len` sit on a data element, not on a group.
    fn scalar_check(&self) -> TokenStream2 {
        if self.max.is_none() && self.len.is_none() {
            return TokenStream2::new();
        }
        let ty = self.element_type();
        quote_spanned! {self.span=>
            const _: () = __p::assert_length_rules_allowed(<#ty as __p::Element>::SCALAR);
        }
    }

    /// The field's `__p::FieldSchema` entry (see `fints_client::codec::schema`).
    fn schema(&self) -> TokenStream2 {
        let name = self.name();
        let max = opt_usize(self.max);
        let len = opt_usize(self.len);
        let (ty, min_count, max_count, repeated) = match &self.repeated {
            Some(inner) => {
                let min = self.min_count.unwrap_or(0);
                (inner, quote! { #min }, opt_usize(self.max_count), true)
            }
            None => {
                let ty = &self.ty;
                (
                    ty,
                    quote! { if <#ty as __p::Element>::OPTIONAL { 0 } else { 1 } },
                    quote! { ::core::option::Option::Some(1) },
                    false,
                )
            }
        };
        quote! {
            __p::FieldSchema {
                name: #name,
                kind: <#ty as __p::Element>::KIND,
                max: #max,
                len: #len,
                min_count: #min_count,
                max_count: #max_count,
                repeated: #repeated,
            }
        }
    }

    fn width(&self) -> TokenStream2 {
        match &self.repeated {
            Some(inner) => {
                let max = opt_usize(self.max_count);
                quote! { __p::repeat_width(<#inner as __p::Element>::WIDTH, #max) }
            }
            None => {
                let ty = &self.ty;
                quote! { <#ty as __p::Element>::WIDTH }
            }
        }
    }
}

fn opt_usize(v: Option<usize>) -> TokenStream2 {
    match v {
        Some(n) => quote! { ::core::option::Option::Some(#n) },
        None => quote! { ::core::option::Option::None },
    }
}

/// `Some(T)` if `ty` is `<path>::Name<T>` with the given last segment name.
fn generic_arg_of<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() {
        return None;
    }
    let last = p.path.segments.last()?;
    if last.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    match args.args.first()? {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

fn parse_fields(input: &DeriveInput) -> syn::Result<Vec<FieldSpec>> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "fints-client: generic segment and group structs are not supported",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "fints-client: only structs with named fields can be derived",
        ));
    };
    let fields = match &data.fields {
        Fields::Named(named) => named.named.iter().collect::<Vec<_>>(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(f) => {
            return Err(syn::Error::new(
                f.span(),
                "fints-client: tuple structs are not supported; name every element",
            ));
        }
    };

    let count = fields.len();
    let mut specs = Vec::with_capacity(count);
    for (i, field) in fields.into_iter().enumerate() {
        let Some(ident) = field.ident.clone() else {
            return Err(syn::Error::new(field.span(), "fints-client: unnamed field"));
        };
        let mut spec = FieldSpec {
            ident,
            ty: field.ty.clone(),
            repeated: generic_arg_of(&field.ty, "Vec").cloned(),
            max: None,
            len: None,
            min_count: None,
            max_count: None,
            span: field.span(),
        };
        for attr in &field.attrs {
            if !attr.path().is_ident("de") {
                continue;
            }
            attr.parse_nested_meta(|meta| {
                let slot = if meta.path.is_ident("max") {
                    &mut spec.max
                } else if meta.path.is_ident("len") {
                    &mut spec.len
                } else if meta.path.is_ident("min_count") {
                    &mut spec.min_count
                } else if meta.path.is_ident("max_count") {
                    &mut spec.max_count
                } else {
                    return Err(meta.error(
                        "fints-client: unknown `de` attribute; expected `max`, `len`, \
                         `min_count` or `max_count`",
                    ));
                };
                let lit: LitInt = meta.value()?.parse()?;
                let value: usize = lit.base10_parse()?;
                if slot.is_some() {
                    return Err(meta.error("fints-client: attribute given twice"));
                }
                *slot = Some(value);
                Ok(())
            })?;
        }
        validate_field(&spec, i + 1 == count)?;
        specs.push(spec);
    }
    Ok(specs)
}

fn validate_field(spec: &FieldSpec, is_last: bool) -> syn::Result<()> {
    let err = |msg: &str| Err(syn::Error::new(spec.span, format!("fints-client: {msg}")));
    if spec.max.is_some() && spec.len.is_some() {
        return err("`max` and `len` exclude each other: a length is either a maximum or fixed");
    }
    if spec.max == Some(0) || spec.len == Some(0) {
        return err("a length of 0 is not allowed");
    }
    if generic_arg_of(&spec.ty, "Option")
        .is_some_and(|inner| generic_arg_of(inner, "Vec").is_some())
    {
        return err("use `Vec<T>` with `min_count = 0` instead of `Option<Vec<T>>`");
    }
    match &spec.repeated {
        None => {
            if spec.min_count.is_some() || spec.max_count.is_some() {
                return err("`min_count`/`max_count` need a `Vec<T>` field");
            }
        }
        Some(inner) => {
            if generic_arg_of(inner, "Option").is_some() || generic_arg_of(inner, "Vec").is_some() {
                return err("a repeated element is `Vec<T>` with T a data element or group");
            }
            if spec.max_count == Some(0) {
                return err("`max_count` must be at least 1");
            }
            if let (Some(min), Some(max)) = (spec.min_count, spec.max_count)
                && min > max
            {
                return err("`min_count` is larger than `max_count`");
            }
            if !is_last && spec.max_count.is_none() {
                return err(
                    "a repeated element that is not the last element needs `max_count`: \
                     its unused repetitions are kept as empty positions (Formals H.1.2)",
                );
            }
        }
    }
    Ok(())
}

fn expand_group(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let ident_name = ident.to_string();
    let fields = parse_fields(input)?;
    if fields.is_empty() {
        return Err(syn::Error::new(
            ident.span(),
            "fints-client: a data element group needs at least one element",
        ));
    }

    let mut decode = Vec::new();
    let mut encode = Vec::new();
    let mut widths = Vec::new();
    let mut checks = Vec::new();
    let mut names = Vec::new();
    let mut schema = Vec::new();
    for f in &fields {
        let fid = &f.ident;
        let name = f.name();
        let rules = f.rules();
        names.push(fid.clone());
        schema.push(f.schema());
        widths.push(f.width());
        checks.push(f.scalar_check());
        match &f.repeated {
            Some(inner) => {
                let min = f.min_count.unwrap_or(0);
                let max = opt_usize(f.max_count);
                decode.push(quote! {
                    let #fid = __p::decode_repeated::<#inner>(__cur, &#rules, #min, #max)
                        .map_err(|e| e.in_field(#name))?;
                });
                encode.push(quote! {
                    __p::encode_repeated::<#inner>(&self.#fid, __out, &#rules, #min, #max)
                        .map_err(|e| e.in_field(#name))?;
                });
            }
            None => {
                let ty = &f.ty;
                decode.push(quote! {
                    let #fid = <#ty as __p::Element>::decode(__cur, &#rules)
                        .map_err(|e| e.in_field(#name))?;
                });
                encode.push(quote! {
                    <#ty as __p::Element>::encode(&self.#fid, __out, &#rules)
                        .map_err(|e| e.in_field(#name))?;
                });
            }
        }
    }

    Ok(quote! {
        const _: () = {
            use ::fints_client::codec::__private as __p;

            impl __p::Element for #ident {
                const WIDTH: ::core::option::Option<usize> = __p::seq_width(&[#(#widths),*]);
                const SCALAR: bool = false;
                const KIND: __p::ElementKind = __p::ElementKind::Group {
                    name: #ident_name,
                    fields: &[#(#schema),*],
                };

                fn decode(
                    __cur: &mut __p::Cursor<'_>,
                    _rules: &__p::Rules,
                ) -> ::core::result::Result<Self, __p::CodecError> {
                    #(#decode)*
                    ::core::result::Result::Ok(Self { #(#names),* })
                }

                fn encode(
                    &self,
                    __out: &mut ::std::vec::Vec<__p::DataElement>,
                    _rules: &__p::Rules,
                ) -> ::core::result::Result<(), __p::CodecError> {
                    #(#encode)*
                    ::core::result::Result::Ok(())
                }
            }

            // Evaluated at compile time: rejects an open-ended repetition that
            // is not the last element of the group.
            const _: ::core::option::Option<usize> = <#ident as __p::Element>::WIDTH;
            #(#checks)*
        };
    })
}

struct SegmentAttr {
    code: String,
    version: u16,
}

fn parse_segment_attr(input: &DeriveInput) -> syn::Result<SegmentAttr> {
    let mut code: Option<LitStr> = None;
    let mut version: Option<LitInt> = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("segment") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("code") {
                if code.is_some() {
                    return Err(meta.error("fints-client: `code` given twice"));
                }
                code = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("version") {
                if version.is_some() {
                    return Err(meta.error("fints-client: `version` given twice"));
                }
                version = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error(
                    "fints-client: unknown `segment` attribute; expected `code` or `version`",
                ));
            }
            Ok(())
        })?;
    }
    let missing = || {
        syn::Error::new(
            input.ident.span(),
            "fints-client: `#[segment(code = \"HKSAL\", version = 7)]` is required",
        )
    };
    let code = code.ok_or_else(missing)?;
    let version = version.ok_or_else(missing)?;
    let code_value = code.value();
    // Formals F, "Segmentkennung": an ..6. Segment codes are upper-case letters
    // (first letter: H, B, D, G, V, X, I per Formals A).
    if code_value.is_empty()
        || code_value.len() > 6
        || !code_value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err(syn::Error::new(
            code.span(),
            "fints-client: a segment code is 1 to 6 upper-case letters or digits",
        ));
    }
    let version_value: u16 = version.base10_parse()?;
    if version_value == 0 || version_value > 999 {
        return Err(syn::Error::new(
            version.span(),
            "fints-client: a segment version is a number from 1 to 999 (num ..3)",
        ));
    }
    Ok(SegmentAttr {
        code: code_value,
        version: version_value,
    })
}

fn expand_segment(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let attr = parse_segment_attr(input)?;
    let fields = parse_fields(input)?;
    let code = &attr.code;
    let version = attr.version;

    let count = fields.len();
    let mut decode = Vec::new();
    let mut encode = Vec::new();
    let mut checks = Vec::new();
    let mut names = Vec::new();
    let mut schema = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        let fid = &f.ident;
        let name = f.name();
        let rules = f.rules();
        let is_last = i + 1 == count;
        names.push(fid.clone());
        schema.push(f.schema());
        checks.push(f.scalar_check());
        match &f.repeated {
            Some(inner) => {
                let min = f.min_count.unwrap_or(0);
                let max = opt_usize(f.max_count);
                decode.push(quote! {
                    let #fid = __slots.repeated::<#inner>(&#rules, #min, #max, #is_last)
                        .map_err(|e| e.in_field(#name))?;
                });
                encode.push(quote! {
                    __writer.repeated::<#inner>(&self.#fid, &#rules, #min, #max, #is_last)
                        .map_err(|e| e.in_field(#name))?;
                });
            }
            None => {
                let ty = &f.ty;
                decode.push(quote! {
                    let #fid = __slots.single::<#ty>(&#rules)
                        .map_err(|e| e.in_field(#name))?;
                });
                encode.push(quote! {
                    __writer.single::<#ty>(&self.#fid, &#rules)
                        .map_err(|e| e.in_field(#name))?;
                });
            }
        }
    }

    let construct = if count == 0 {
        quote! { Self {} }
    } else {
        quote! { Self { #(#names),* } }
    };
    // A unit struct is written `Self`, a braced struct `Self {}`.
    let construct = match &input.data {
        Data::Struct(s) if matches!(s.fields, Fields::Unit) => quote! { Self },
        _ => construct,
    };

    Ok(quote! {
        const _: () = {
            use ::fints_client::codec::__private as __p;

            impl __p::Segment for #ident {
                const CODE: &'static str = #code;
                const VERSION: u16 = #version;
                const SCHEMA: &'static [__p::FieldSchema] = &[#(#schema),*];

                fn decode_body(
                    __body: &[__p::DEG],
                ) -> ::core::result::Result<Self, __p::CodecError> {
                    #[allow(unused_mut)]
                    let mut __slots = __p::SlotReader::new(__body);
                    #(#decode)*
                    __slots.finish()?;
                    ::core::result::Result::Ok(#construct)
                }

                fn encode_body(
                    &self,
                ) -> ::core::result::Result<::std::vec::Vec<__p::DEG>, __p::CodecError> {
                    #[allow(unused_mut)]
                    let mut __writer = __p::SlotWriter::new();
                    #(#encode)*
                    ::core::result::Result::Ok(__writer.finish())
                }
            }
            #(#checks)*
        };
    })
}
