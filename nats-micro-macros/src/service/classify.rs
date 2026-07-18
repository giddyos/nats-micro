use quote::ToTokens;
use syn::{Attribute, FnArg, GenericArgument, ImplItemFn, Pat, PathArguments, ReturnType, Type};

use super::{OperationKind, OperationOptions, SubjectModel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireCodec {
    Json,
    Protobuf,
    Raw,
    Utf8,
    Empty,
}

impl WireCodec {
    pub(crate) fn parse_ident(ident: &syn::Ident) -> syn::Result<Self> {
        match ident.to_string().as_str() {
            "json" => Ok(Self::Json),
            "protobuf" | "proto" => Ok(Self::Protobuf),
            "raw" => Ok(Self::Raw),
            "utf8" | "text" => Ok(Self::Utf8),
            "empty" => Ok(Self::Empty),
            _ => Err(syn::Error::new(
                ident.span(),
                "codec must be `json`, `protobuf`, `raw`, `utf8`, or `empty`",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wrapper {
    Direct,
    Json,
    Proto,
    Body,
    Text,
    Response,
}

#[derive(Debug, Clone)]
pub(crate) struct PayloadModel {
    pub codec: WireCodec,
    pub wrapper: Wrapper,
    pub optional: bool,
    pub encrypted: bool,
    pub decoded_type: Option<Type>,
}

#[derive(Debug, Clone)]
pub(crate) enum ArgumentKind {
    State,
    StateRef(Type),
    Subject { name: String, segment: usize },
    Header { name: String, optional: bool },
    Headers,
    RequestMeta,
    Auth { claims: Type, optional: bool },
    Payload(PayloadModel),
}

#[derive(Debug, Clone)]
pub(crate) struct ArgumentModel {
    pub ident: syn::Ident,
    pub ty: Type,
    pub kind: ArgumentKind,
}

#[derive(Debug, Clone)]
pub(crate) struct ResponseModel {
    pub ok_type: Type,
    pub error_type: Option<Type>,
    pub wire_type: Type,
    pub codec: WireCodec,
    pub wrapper: Wrapper,
    pub optional: bool,
    pub encrypted: bool,
}

pub(crate) fn classify_arguments(
    method: &ImplItemFn,
    state_type: &Type,
    subject: &SubjectModel,
    default_codec: WireCodec,
) -> syn::Result<Vec<ArgumentModel>> {
    let mut arguments = Vec::new();
    let mut payload_seen = false;

    for input in &method.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "service operations cannot take a receiver",
            ));
        };
        let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &pat_type.pat,
                "operation arguments must be simple identifiers",
            ));
        };
        let ident = pat_ident.ident.clone();
        let ty = pat_type.ty.as_ref().clone();
        let header = parse_header_attr(&pat_type.attrs)?;
        let placeholder = subject
            .placeholders
            .iter()
            .find(|(name, _)| ident == name.as_str());

        let kind = if let Some((name, segment)) = placeholder {
            if header.is_some() {
                return Err(syn::Error::new_spanned(
                    &ident,
                    "a subject parameter cannot also be a header parameter",
                ));
            }
            ArgumentKind::Subject {
                name: name.clone(),
                segment: *segment,
            }
        } else if let Some(name) = header {
            let optional = option_inner(&ty).is_some();
            let value_type = option_inner(&ty).unwrap_or(&ty);
            if !is_str_reference(value_type) {
                return Err(syn::Error::new_spanned(
                    &ty,
                    "header parameters must be `&str` or `Option<&str>`",
                ));
            }
            ArgumentKind::Header { name, optional }
        } else if is_state_reference(&ty, state_type)? {
            ArgumentKind::State
        } else if last_ident(&ty).as_deref() == Some("StateRef") {
            let inner = last_type_argument(&ty).ok_or_else(|| {
                syn::Error::new_spanned(&ty, "StateRef requires a projected state type")
            })?;
            ArgumentKind::StateRef(inner.clone())
        } else if last_ident(&ty).as_deref() == Some("Headers") {
            ArgumentKind::Headers
        } else if last_ident(&ty).as_deref() == Some("RequestMeta") {
            ArgumentKind::RequestMeta
        } else if let Some((claims, optional)) = auth_inner(&ty) {
            ArgumentKind::Auth {
                claims: claims.clone(),
                optional,
            }
        } else {
            if last_ident(&ty).as_deref() == Some("SubjectParam") {
                return Err(syn::Error::new_spanned(
                    &ty,
                    "subject arguments are inferred by matching their name to a `{placeholder}`",
                ));
            }
            if matches!(
                ty,
                Type::BareFn(_)
                    | Type::ImplTrait(_)
                    | Type::Infer(_)
                    | Type::Macro(_)
                    | Type::Never(_)
                    | Type::Ptr(_)
                    | Type::Slice(_)
                    | Type::TraitObject(_)
            ) {
                return Err(syn::Error::new_spanned(
                    &ty,
                    "unsupported payload argument shape",
                ));
            }
            if payload_seen {
                return Err(syn::Error::new_spanned(
                    &ty,
                    "service operations may have at most one payload argument",
                ));
            }
            payload_seen = true;
            ArgumentKind::Payload(classify_payload(&ty, default_codec)?)
        };

        arguments.push(ArgumentModel { ident, ty, kind });
    }

    Ok(arguments)
}

pub(crate) fn classify_response(
    method: &ImplItemFn,
    options: &OperationOptions,
    kind: OperationKind,
) -> ResponseModel {
    let declared: Type = match &method.sig.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, ty) => ty.as_ref().clone(),
    };
    let (ok_type, error_type) = result_types(&declared)
        .map_or_else(|| (declared.clone(), None), |(ok, error)| (ok, Some(error)));
    let (optional, value_type) = if let Some(inner) = option_inner(&ok_type) {
        (true, inner.clone())
    } else {
        (false, ok_type.clone())
    };
    let (encrypted, value_type) = encryption_inner(&value_type)
        .map_or((false, value_type.clone()), |inner| (true, inner.clone()));
    let wire_type = options
        .response
        .clone()
        .unwrap_or_else(|| response_wire_type(&value_type));

    if kind == OperationKind::Consumer {
        return ResponseModel {
            ok_type,
            error_type,
            wire_type,
            codec: WireCodec::Empty,
            wrapper: Wrapper::Direct,
            optional: false,
            encrypted,
        };
    }

    let (codec, wrapper) = classify_response_value(&value_type);
    ResponseModel {
        ok_type,
        error_type,
        wire_type,
        codec,
        wrapper,
        optional,
        encrypted,
    }
}

fn classify_payload(ty: &Type, default_codec: WireCodec) -> syn::Result<PayloadModel> {
    let (optional, value) = option_inner(ty).map_or((false, ty), |inner| (true, inner));
    let (encrypted, value) = encryption_inner(value).map_or((false, value), |inner| (true, inner));
    let ident = last_ident(value);
    let (codec, wrapper, decoded_type) = match ident.as_deref() {
        Some("Json") => (
            WireCodec::Json,
            Wrapper::Json,
            Some(required_type_argument(value, "Json")?),
        ),
        Some("Proto") => (
            WireCodec::Protobuf,
            Wrapper::Proto,
            Some(required_type_argument(value, "Proto")?),
        ),
        Some("Body") => (WireCodec::Raw, Wrapper::Body, None),
        Some("Text") => (WireCodec::Utf8, Wrapper::Text, None),
        _ => (default_codec, Wrapper::Direct, Some(value.clone())),
    };
    Ok(PayloadModel {
        codec,
        wrapper,
        optional,
        encrypted,
        decoded_type,
    })
}

fn classify_response_value(ty: &Type) -> (WireCodec, Wrapper) {
    if is_unit(ty) {
        return (WireCodec::Empty, Wrapper::Direct);
    }
    match last_ident(ty).as_deref() {
        Some("Response") => (WireCodec::Raw, Wrapper::Response),
        Some("Json") => (WireCodec::Json, Wrapper::Json),
        Some("Proto") => (WireCodec::Protobuf, Wrapper::Proto),
        Some("Bytes" | "String") => (WireCodec::Raw, Wrapper::Direct),
        Some("Vec") if is_vec_u8(ty) => (WireCodec::Raw, Wrapper::Direct),
        _ if is_str_reference(ty) => (WireCodec::Utf8, Wrapper::Direct),
        _ => (WireCodec::Json, Wrapper::Direct),
    }
}

fn response_wire_type(ty: &Type) -> Type {
    match last_ident(ty).as_deref() {
        Some("Json" | "Proto") => last_type_argument(ty)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        _ => match ty {
            Type::Reference(reference) => reference.elem.as_ref().clone(),
            _ => ty.clone(),
        },
    }
}

fn parse_header_attr(attrs: &[Attribute]) -> syn::Result<Option<String>> {
    let mut header = None;
    for attr in attrs {
        if !attr.path().is_ident("header") {
            continue;
        }
        if header.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "duplicate #[header] attribute",
            ));
        }
        header = Some(attr.parse_args::<syn::LitStr>()?.value());
    }
    Ok(header)
}

fn is_state_reference(ty: &Type, state_type: &Type) -> syn::Result<bool> {
    let Type::Reference(reference) = ty else {
        return Ok(false);
    };
    if reference.mutability.is_some() && same_type(reference.elem.as_ref(), state_type) {
        return Err(syn::Error::new_spanned(
            ty,
            "application state must be borrowed as `&State`, not `&mut State`",
        ));
    }
    Ok(reference.mutability.is_none() && same_type(reference.elem.as_ref(), state_type))
}

pub(crate) fn same_type(left: &Type, right: &Type) -> bool {
    compact(left) == compact(right)
}

fn compact(value: &impl ToTokens) -> String {
    value.to_token_stream().to_string().replace(' ', "")
}

pub(crate) fn last_ident(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub(crate) fn last_type_argument(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

fn required_type_argument(ty: &Type, wrapper: &str) -> syn::Result<Type> {
    last_type_argument(ty)
        .cloned()
        .ok_or_else(|| syn::Error::new_spanned(ty, format!("{wrapper} requires one type argument")))
}

pub(crate) fn option_inner(ty: &Type) -> Option<&Type> {
    (last_ident(ty).as_deref() == Some("Option"))
        .then(|| last_type_argument(ty))
        .flatten()
}

pub(crate) fn encryption_inner(ty: &Type) -> Option<&Type> {
    (last_ident(ty).as_deref() == Some("Encrypted"))
        .then(|| last_type_argument(ty))
        .flatten()
}

fn auth_inner(ty: &Type) -> Option<(&Type, bool)> {
    if last_ident(ty).as_deref() == Some("Auth") {
        return last_type_argument(ty).map(|claims| (claims, false));
    }
    option_inner(ty)
        .filter(|inner| last_ident(inner).as_deref() == Some("Auth"))
        .and_then(last_type_argument)
        .map(|claims| (claims, true))
}

fn result_types(ty: &Type) -> Option<(Type, Type)> {
    if last_ident(ty).as_deref() != Some("Result") {
        return None;
    }
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let mut types = arguments.args.iter().filter_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    });
    Some((types.next()?, types.next()?))
}

pub(crate) fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn is_str_reference(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Reference(reference)
            if matches!(
                reference.elem.as_ref(),
                Type::Path(path) if path.path.is_ident("str")
            )
    )
}

fn is_vec_u8(ty: &Type) -> bool {
    last_ident(ty).as_deref() == Some("Vec")
        && last_type_argument(ty)
            .is_some_and(|inner| matches!(inner, Type::Path(path) if path.path.is_ident("u8")))
}

pub(crate) fn clean_parameter_attrs(method: &mut ImplItemFn) {
    for input in &mut method.sig.inputs {
        if let FnArg::Typed(pat_type) = input {
            pat_type
                .attrs
                .retain(|attr| !attr.path().is_ident("header"));
        }
    }
}
