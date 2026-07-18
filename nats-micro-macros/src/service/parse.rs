use proc_macro2::TokenStream;
use syn::{
    Attribute, Ident, ImplItem, ItemImpl, LitInt, LitStr, Meta, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

use super::{
    AuthIntent, MethodModel, OperationKind, OperationModel, OperationOptions, ServiceArgs,
    ServiceModel, SubjectModel, classify,
};

pub(crate) fn build_model(args: ServiceArgs, input: TokenStream) -> syn::Result<ServiceModel> {
    let mut item_impl: ItemImpl = syn::parse2(input)?;
    if item_impl.trait_.is_some() {
        return Err(syn::Error::new_spanned(
            &item_impl,
            "#[service] requires an inherent impl block",
        ));
    }
    if !item_impl.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item_impl.generics,
            "service impl blocks cannot be generic",
        ));
    }
    let service_ident = service_ident(&item_impl.self_ty)?;
    let state_type = args.state_type.clone();
    let mut methods = Vec::new();

    for item in &mut item_impl.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        let operation_attrs: Vec<_> = method
            .attrs
            .iter()
            .filter_map(|attr| operation_kind(attr).map(|kind| (kind, attr.clone())))
            .collect();
        if operation_attrs.len() > 1 {
            return Err(syn::Error::new_spanned(
                &method.sig.ident,
                "a method may declare only one service operation attribute",
            ));
        }
        let Some((kind, operation_attr)) = operation_attrs.first() else {
            methods.push(MethodModel::Ordinary(method.clone()));
            continue;
        };
        if method.sig.asyncness.is_none() && *kind != OperationKind::Publish {
            return Err(syn::Error::new_spanned(
                method.sig.fn_token,
                "service handlers must be async",
            ));
        }

        let mut options = parse_operation_options(operation_attr)?;
        options.concurrency.get_or_insert(args.defaults.concurrency);
        if options.queue.is_none() {
            options.queue.clone_from(&args.defaults.queue);
        }
        if options.auth.is_none() && args.defaults.auth != AuthIntent::None {
            options.auth = Some(args.defaults.auth);
        }
        let subject = build_subject(&args, method, *kind, &options)?;
        let arguments =
            classify::classify_arguments(method, &state_type, &subject, args.defaults.codec)?;
        let response = classify::classify_response(method, &options, *kind);

        method.attrs.retain(|attr| operation_kind(attr).is_none());
        classify::clean_parameter_attrs(method);
        let model = OperationModel {
            kind: *kind,
            method: method.clone(),
            options,
            subject,
            arguments,
            response,
            operation_index: None,
            consumer_index: None,
        };
        methods.push(match kind {
            OperationKind::Request => MethodModel::Request(model),
            OperationKind::Publish => MethodModel::Publish(model),
            OperationKind::Subscribe => MethodModel::Subscribe(model),
            OperationKind::Consumer => MethodModel::Consumer(model),
        });
    }

    Ok(ServiceModel {
        args,
        service_ident,
        state_type,
        item_impl,
        methods,
    })
}

pub(crate) fn assign_indexes(model: &mut ServiceModel) {
    let mut operation_index = 0usize;
    let mut consumer_index = 0usize;
    for method in &mut model.methods {
        let operation = match method {
            MethodModel::Request(operation)
            | MethodModel::Publish(operation)
            | MethodModel::Subscribe(operation) => {
                operation.operation_index = Some(operation_index);
                operation_index += 1;
                continue;
            }
            MethodModel::Consumer(operation) => operation,
            MethodModel::Ordinary(_) => continue,
        };
        operation.consumer_index = Some(consumer_index);
        consumer_index += 1;
    }
}

fn service_ident(ty: &Type) -> syn::Result<Ident> {
    let Type::Path(path) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "service impl must target a named type",
        ));
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return Err(syn::Error::new_spanned(
            ty,
            "service impl must target one unqualified type name",
        ));
    }
    Ok(path.path.segments[0].ident.clone())
}

fn operation_kind(attr: &Attribute) -> Option<OperationKind> {
    if attr.path().is_ident("request") {
        Some(OperationKind::Request)
    } else if attr.path().is_ident("publish") {
        Some(OperationKind::Publish)
    } else if attr.path().is_ident("subscribe") {
        Some(OperationKind::Subscribe)
    } else if attr.path().is_ident("consumer") {
        Some(OperationKind::Consumer)
    } else {
        None
    }
}

fn build_subject(
    args: &ServiceArgs,
    method: &syn::ImplItemFn,
    kind: OperationKind,
    options: &OperationOptions,
) -> syn::Result<SubjectModel> {
    let local = match kind {
        OperationKind::Consumer => options
            .filter
            .as_ref()
            .or(options.subject.as_ref())
            .cloned()
            .unwrap_or_default(),
        _ => options
            .subject
            .clone()
            .unwrap_or_else(|| method.sig.ident.to_string()),
    };
    let template = if kind == OperationKind::Consumer {
        local
    } else {
        let major = args.version.split('.').next().unwrap_or("0");
        format!("{}.v{major}.{local}", args.prefix)
    };
    parse_subject_template(&template, kind, method.sig.ident.span())
}

fn parse_subject_template(
    template: &str,
    kind: OperationKind,
    span: proc_macro2::Span,
) -> syn::Result<SubjectModel> {
    if template.is_empty() {
        return Ok(SubjectModel {
            template: String::new(),
            pattern: String::new(),
            placeholders: Vec::new(),
        });
    }
    let mut pattern = Vec::new();
    let mut placeholders = Vec::new();
    for (index, segment) in template.split('.').enumerate() {
        if segment.is_empty() {
            return Err(syn::Error::new(span, "subject contains an empty segment"));
        }
        if segment.starts_with('{') || segment.ends_with('}') {
            if !segment.starts_with('{') || !segment.ends_with('}') {
                return Err(syn::Error::new(
                    span,
                    format!("invalid subject template segment `{segment}`"),
                ));
            }
            let name = &segment[1..segment.len() - 1];
            syn::parse_str::<Ident>(name).map_err(|_| {
                syn::Error::new(span, format!("invalid subject placeholder `{name}`"))
            })?;
            placeholders.push((name.to_owned(), index));
            pattern.push("*".to_owned());
        } else {
            if segment.contains(['{', '}']) {
                return Err(syn::Error::new(
                    span,
                    format!("invalid subject template segment `{segment}`"),
                ));
            }
            if matches!(kind, OperationKind::Request | OperationKind::Publish)
                && matches!(segment, "*" | ">")
            {
                return Err(syn::Error::new(
                    span,
                    "request and publish templates use `{name}` instead of raw wildcards",
                ));
            }
            pattern.push(segment.to_owned());
        }
    }
    Ok(SubjectModel {
        template: template.to_owned(),
        pattern: pattern.join("."),
        placeholders,
    })
}

fn parse_operation_options(attr: &Attribute) -> syn::Result<OperationOptions> {
    let Meta::List(list) = &attr.meta else {
        return Ok(OperationOptions::default());
    };
    syn::parse2(list.tokens.clone())
}

impl Parse for OperationOptions {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut options = Self::default();
        let mut first = true;
        while !input.is_empty() {
            if first && input.peek(LitStr) {
                options.subject = Some(input.parse::<LitStr>()?.value());
            } else {
                let name: Ident = input.parse()?;
                input.parse::<Token![=]>()?;
                match name.to_string().as_str() {
                    "subject" => options.subject = Some(input.parse::<LitStr>()?.value()),
                    "queue" | "queue_group" => {
                        options.queue = Some(input.parse::<LitStr>()?.value());
                    }
                    "concurrency" | "concurrency_limit" => {
                        options.concurrency =
                            Some(input.parse::<LitInt>()?.base10_parse::<usize>()?);
                    }
                    "auth" => {
                        let value: Ident = input.parse()?;
                        options.auth = Some(match value.to_string().as_str() {
                            "none" => AuthIntent::None,
                            "optional" => AuthIntent::Optional,
                            "required" => AuthIntent::Required,
                            _ => {
                                return Err(syn::Error::new(
                                    value.span(),
                                    "auth must be `none`, `optional`, or `required`",
                                ));
                            }
                        });
                    }
                    "response" => options.response = Some(input.parse()?),
                    "stream" => options.stream = Some(input.parse::<LitStr>()?.value()),
                    "durable" => options.durable = Some(input.parse::<LitStr>()?.value()),
                    "filter" | "filter_subject" => {
                        options.filter = Some(input.parse::<LitStr>()?.value());
                    }
                    "ack_wait" => {
                        let value = input.parse::<LitStr>()?;
                        options.ack_wait_ms = Some(super::args::parse_duration_ms(
                            &value.value(),
                            value.span(),
                        )?);
                    }
                    "max_deliver" => {
                        let negative = input.peek(Token![-]);
                        if negative {
                            input.parse::<Token![-]>()?;
                        }
                        let value = input.parse::<LitInt>()?.base10_parse::<i64>()?;
                        options.max_deliver = Some(if negative { -value } else { value });
                    }
                    "backoff" => {
                        let content;
                        syn::bracketed!(content in input);
                        let values = Punctuated::<LitStr, Token![,]>::parse_terminated(&content)?;
                        options.backoff_ms = values
                            .iter()
                            .map(|value| {
                                super::args::parse_duration_ms(&value.value(), value.span())
                            })
                            .collect::<syn::Result<_>>()?;
                    }
                    _ => {
                        return Err(syn::Error::new(name.span(), "unsupported operation option"));
                    }
                }
            }
            first = false;
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(options)
    }
}
