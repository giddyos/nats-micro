use std::collections::{BTreeMap, BTreeSet};

use quote::ToTokens;
use syn::Type;

use super::{
    ArgumentKind, AuthIntent, OperationKind, OperationModel, ServiceModel, Wrapper, classify,
};

pub(crate) fn validate_service(model: &ServiceModel) -> syn::Result<()> {
    validate_version(&model.args.version, model.service_ident.span())?;
    validate_token(&model.args.name, "service name", model.service_ident.span())?;
    validate_subject(
        &model.args.prefix,
        "service prefix",
        model.service_ident.span(),
    )?;
    if model.args.defaults.concurrency == 0 {
        return Err(syn::Error::new(
            model.service_ident.span(),
            "default concurrency must be greater than zero",
        ));
    }

    let mut rust_names = BTreeSet::new();
    let mut subjects = BTreeMap::<String, proc_macro2::Span>::new();
    for method in &model.methods {
        let Some(operation) = method.operation() else {
            continue;
        };
        let name = operation.method.sig.ident.to_string();
        if !rust_names.insert(name.clone()) {
            return Err(syn::Error::new(
                operation.method.sig.ident.span(),
                format!("duplicate normalized operation name `{name}`"),
            ));
        }
        validate_operation(model, operation)?;
        if operation.kind != OperationKind::Consumer
            && let Some(previous) = subjects.insert(
                operation.subject.pattern.clone(),
                operation.method.sig.ident.span(),
            )
        {
            let mut error = syn::Error::new(
                operation.method.sig.ident.span(),
                format!(
                    "duplicate operation subject `{}`",
                    operation.subject.pattern
                ),
            );
            error.combine(syn::Error::new(previous, "first declared here"));
            return Err(error);
        }
    }
    Ok(())
}

fn validate_operation(model: &ServiceModel, operation: &OperationModel) -> syn::Result<()> {
    let span = operation.method.sig.ident.span();
    let concurrency = operation
        .options
        .concurrency
        .unwrap_or(model.args.defaults.concurrency);
    if concurrency == 0 {
        return Err(syn::Error::new(
            span,
            "concurrency must be greater than zero",
        ));
    }
    if let Some(queue) = operation
        .options
        .queue
        .as_ref()
        .or(model.args.defaults.queue.as_ref())
    {
        validate_token(queue, "queue group", span)?;
    }
    validate_subject(&operation.subject.pattern, "subject", span)?;

    let mut placeholders = BTreeSet::new();
    for (name, _) in &operation.subject.placeholders {
        if !placeholders.insert(name) {
            return Err(syn::Error::new(
                span,
                format!("duplicate subject placeholder `{name}`"),
            ));
        }
        if !operation.arguments.iter().any(|argument| {
            matches!(
                &argument.kind,
                ArgumentKind::Subject {
                    name: argument_name,
                    ..
                } if argument_name == name
            )
        }) {
            return Err(syn::Error::new(
                span,
                format!("subject placeholder `{name}` requires a matching handler argument"),
            ));
        }
    }

    for argument in &operation.arguments {
        match &argument.kind {
            ArgumentKind::Payload(_) if matches!(argument.ty, Type::Reference(_)) => {
                return Err(syn::Error::new_spanned(
                    &argument.ty,
                    format!(
                        "state reference does not match configured state `{}`",
                        model.state_type.to_token_stream()
                    ),
                ));
            }
            ArgumentKind::Auth { .. }
            | ArgumentKind::State
            | ArgumentKind::StateRef(_)
            | ArgumentKind::Subject { .. }
            | ArgumentKind::Header { .. }
            | ArgumentKind::Headers
            | ArgumentKind::RequestMeta
            | ArgumentKind::Payload(_) => {}
        }
        if !cfg!(feature = "macros_encryption_feature")
            && classify::last_ident(&argument.ty).as_deref() == Some("Encrypted")
        {
            return Err(syn::Error::new_spanned(
                &argument.ty,
                "encrypted operations require the `encryption` feature",
            ));
        }
    }

    validate_auth(model, operation)?;
    validate_response(operation)?;
    if model.args.napi {
        for argument in &operation.arguments {
            validate_napi_type(&argument.ty)?;
        }
        validate_napi_type(&operation.response.wire_type)?;
    }

    if operation.kind == OperationKind::Consumer {
        validate_consumer(operation)?;
    }

    Ok(())
}

fn validate_consumer(operation: &OperationModel) -> syn::Result<()> {
    let span = operation.method.sig.ident.span();
    let stream = operation
        .options
        .stream
        .as_deref()
        .ok_or_else(|| syn::Error::new(span, "consumer requires `stream = \"...\"`"))?;
    let durable = operation
        .options
        .durable
        .as_deref()
        .ok_or_else(|| syn::Error::new(span, "consumer requires `durable = \"...\"`"))?;
    if operation.subject.pattern.is_empty() {
        return Err(syn::Error::new(
            span,
            "consumer requires `filter = \"...\"`",
        ));
    }
    validate_token(stream, "consumer stream", span)?;
    validate_token(durable, "consumer durable", span)?;
    if operation.options.ack_wait_ms == Some(0) {
        return Err(syn::Error::new(
            span,
            "consumer ack_wait must be greater than zero",
        ));
    }
    if operation.options.backoff_ms.contains(&0) {
        return Err(syn::Error::new(
            span,
            "consumer backoff durations must be greater than zero",
        ));
    }
    Ok(())
}

fn validate_napi_type(ty: &Type) -> syn::Result<()> {
    let rendered = ty.to_token_stream().to_string();
    if ["HashMap", "BTreeMap", "Rc", "Arc", "RequestMeta", "Headers"]
        .iter()
        .any(|unsupported| rendered.contains(unsupported))
    {
        Err(syn::Error::new_spanned(ty, "unsupported N-API wire type"))
    } else {
        Ok(())
    }
}

fn validate_auth(model: &ServiceModel, operation: &OperationModel) -> syn::Result<()> {
    let span = operation.method.sig.ident.span();
    let required_count = operation
        .arguments
        .iter()
        .filter(|argument| {
            matches!(
                argument.kind,
                ArgumentKind::Auth {
                    optional: false,
                    ..
                }
            )
        })
        .count();
    let optional_count = operation
        .arguments
        .iter()
        .filter(|argument| matches!(argument.kind, ArgumentKind::Auth { optional: true, .. }))
        .count();
    let inferred = if required_count > 0 {
        AuthIntent::Required
    } else if optional_count > 0 {
        AuthIntent::Optional
    } else {
        AuthIntent::None
    };
    let declared = operation.options.auth.unwrap_or_else(|| {
        if model.args.defaults.auth == AuthIntent::None {
            inferred
        } else {
            model.args.defaults.auth
        }
    });
    match declared {
        AuthIntent::Required if required_count == 0 => Err(syn::Error::new(
            span,
            "`auth = required` requires an `Auth<T>` argument",
        )),
        AuthIntent::Optional if optional_count == 0 || required_count > 0 => Err(syn::Error::new(
            span,
            "`auth = optional` requires `Option<Auth<T>>` and no required auth argument",
        )),
        AuthIntent::None if required_count + optional_count > 0 => Err(syn::Error::new(
            span,
            "`auth = none` cannot be combined with auth arguments",
        )),
        _ => Ok(()),
    }
}

fn validate_response(operation: &OperationModel) -> syn::Result<()> {
    let response = &operation.response;
    if response.optional && response.wrapper == Wrapper::Response {
        return Err(syn::Error::new_spanned(
            &response.ok_type,
            "Option<Response> is unsupported; return Response directly",
        ));
    }
    if let Type::Reference(reference) = if response.optional {
        classify::option_inner(&response.ok_type).unwrap_or(&response.ok_type)
    } else {
        &response.ok_type
    } {
        let is_static_str = matches!(
            reference.elem.as_ref(),
            Type::Path(path) if path.path.is_ident("str")
        ) && reference
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| lifetime.ident == "static");
        if !is_static_str && operation.options.response.is_none() {
            return Err(syn::Error::new_spanned(
                &response.ok_type,
                "borrowed server responses require `response = OwnedType`",
            ));
        }
    }
    Ok(())
}

fn validate_version(version: &str, span: proc_macro2::Span) -> syn::Result<()> {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        Ok(())
    } else {
        Err(syn::Error::new(
            span,
            "service `version` must match x.x.x with ASCII digits",
        ))
    }
}

fn validate_token(value: &str, label: &str, span: proc_macro2::Span) -> syn::Result<()> {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'/' | b':'))
    {
        Ok(())
    } else {
        Err(syn::Error::new(span, format!("invalid {label} `{value}`")))
    }
}

fn validate_subject(value: &str, label: &str, span: proc_macro2::Span) -> syn::Result<()> {
    if value.is_empty() {
        return Err(syn::Error::new(span, format!("{label} cannot be empty")));
    }
    for (index, segment) in value.split('.').enumerate() {
        if segment.is_empty()
            || (segment == ">" && index + 1 != value.split('.').count())
            || (!matches!(segment, "*" | ">")
                && !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        {
            return Err(syn::Error::new(span, format!("invalid {label} `{value}`")));
        }
    }
    Ok(())
}
