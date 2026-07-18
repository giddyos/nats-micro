use heck::ToUpperCamelCase;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Type;

use super::{
    ArgumentKind, MethodModel, OperationKind, OperationModel, PayloadModel, ServiceModel,
    WireCodec, Wrapper, classify,
};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let client_ident = format_ident!("{}Client", model.service_ident);
    let service_ident = &model.service_ident;
    let operations: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| {
            matches!(
                operation.kind,
                OperationKind::Request | OperationKind::Publish
            )
        })
        .map(operation_methods)
        .collect();
    let operation_models: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| {
            matches!(
                operation.kind,
                OperationKind::Request | OperationKind::Publish
            )
        })
        .map(|operation| {
            let marker = format_ident!(
                "{}{}Operation",
                service_ident,
                operation.method.sig.ident.to_string().to_upper_camel_case()
            );
            let index = operation.operation_index.expect("operation index");
            quote! {
                #[derive(Debug, Clone, Copy, Default)]
                pub struct #marker;

                impl #marker {
                    pub const SPEC: &'static #nats_micro::OperationSpec =
                        &#service_ident::SPEC.operations[#index];
                }
            }
        })
        .collect();

    quote! {
        #[derive(Debug, Clone)]
        pub struct #client_ident<T = #nats_micro::NatsTransport> {
            transport: T,
            default_headers: Option<#nats_micro::NatsHeaderMap>,
        }

        impl<T> #client_ident<T> {
            #[must_use]
            pub const fn new(transport: T) -> Self {
                Self {
                    transport,
                    default_headers: None,
                }
            }

            #[must_use]
            pub const fn transport(&self) -> &T {
                &self.transport
            }

            #[must_use]
            pub fn into_transport(self) -> T {
                self.transport
            }

            #[must_use]
            pub fn with_default_headers(
                mut self,
                headers: #nats_micro::NatsHeaderMap,
            ) -> Self {
                self.default_headers = Some(headers);
                self
            }

            #[must_use]
            pub const fn default_headers(&self) -> Option<&#nats_micro::NatsHeaderMap> {
                self.default_headers.as_ref()
            }
        }

        impl<T> #client_ident<T>
        where
            T: #nats_micro::ClientTransport,
        {
            #(#operations)*
        }

        #(#operation_models)*
    }
}

fn operation_methods(operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let method = &operation.method.sig.ident;
    let call_method = format_ident!("{method}_call");
    let parameters = client_parameters(operation);
    let declarations: Vec<_> = parameters
        .iter()
        .map(|parameter| {
            let ident = &parameter.ident;
            let ty = &parameter.ty;
            quote!(#ident: #ty)
        })
        .collect();
    let arguments: Vec<_> = parameters
        .iter()
        .map(|parameter| parameter.ident.clone())
        .collect();
    let subject = subject_expression(operation);
    let payload = payload_expression(operation);
    let timeout = operation.options.timeout_ms.map_or_else(
        || quote!(None),
        |milliseconds| quote!(Some(::std::time::Duration::from_millis(#milliseconds))),
    );
    let error = operation.response.error_type.as_ref().map_or_else(
        || syn::parse_quote!(#nats_micro::NatsErrorResponse),
        |error| {
            if classify::last_ident(error).as_deref() == Some("ErrorReply") {
                syn::parse_quote!(#nats_micro::NatsErrorResponse)
            } else {
                error.clone()
            }
        },
    );

    match operation.kind {
        OperationKind::Request => {
            let response = client_response_type(operation);
            let decoder = decoder_type(operation);
            quote! {
                pub async fn #method(
                    &self,
                    #(#declarations),*
                ) -> ::std::result::Result<#response, #nats_micro::ClientError<#error>> {
                    self.#call_method(#(#arguments),*).send().await
                }

                #[must_use]
                pub fn #call_method<'__client>(
                    &'__client self,
                    #(#declarations),*
                ) -> #nats_micro::RequestCall<
                    '__client,
                    T,
                    #response,
                    #error,
                    #decoder,
                > {
                    let __subject = #subject;
                    let __payload = #payload;
                    #nats_micro::RequestCall::<T, #response, #error, #decoder>::new(
                        &self.transport,
                        __subject,
                        __payload,
                        self.default_headers.as_ref(),
                        #timeout,
                    )
                }
            }
        }
        OperationKind::Publish => quote! {
            pub async fn #method(
                &self,
                #(#declarations),*
            ) -> ::std::result::Result<(), #nats_micro::ClientError<#error>> {
                self.#call_method(#(#arguments),*).send().await
            }

            #[must_use]
            pub fn #call_method<'__client>(
                &'__client self,
                #(#declarations),*
            ) -> #nats_micro::PublishCall<'__client, T, #error> {
                let __subject = #subject;
                let __payload = #payload;
                #nats_micro::PublishCall::<T, #error>::new(
                    &self.transport,
                    __subject,
                    __payload,
                    self.default_headers.as_ref(),
                )
            }
        },
        OperationKind::Subscribe | OperationKind::Consumer => quote!(),
    }
}

struct ClientParameter {
    ident: syn::Ident,
    ty: Type,
}

fn client_parameters(operation: &OperationModel) -> Vec<ClientParameter> {
    operation
        .arguments
        .iter()
        .filter_map(|argument| match &argument.kind {
            ArgumentKind::Subject { .. } => Some(ClientParameter {
                ident: argument.ident.clone(),
                ty: argument.ty.clone(),
            }),
            ArgumentKind::Payload(payload) => Some(ClientParameter {
                ident: argument.ident.clone(),
                ty: client_payload_type(&argument.ty, payload),
            }),
            ArgumentKind::State
            | ArgumentKind::StateRef(_)
            | ArgumentKind::Header { .. }
            | ArgumentKind::Headers
            | ArgumentKind::RequestMeta
            | ArgumentKind::Auth { .. } => None,
        })
        .collect()
}

fn client_payload_type(declared: &Type, payload: &PayloadModel) -> Type {
    let value = payload.decoded_type.as_ref().unwrap_or(declared);
    let borrowed: Type = match payload.wrapper {
        Wrapper::Body => syn::parse_quote!(&[u8]),
        Wrapper::Text => syn::parse_quote!(&str),
        Wrapper::Direct
            if classify::last_ident(value).as_deref() == Some("Vec")
                && payload.codec == WireCodec::Raw =>
        {
            syn::parse_quote!(&[u8])
        }
        Wrapper::Json | Wrapper::Proto | Wrapper::Direct => syn::parse_quote!(&#value),
        Wrapper::Response => declared.clone(),
    };
    if payload.optional {
        syn::parse_quote!(Option<#borrowed>)
    } else {
        borrowed
    }
}

fn subject_expression(operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    if operation.subject.placeholders.is_empty() {
        let subject = &operation.subject.pattern;
        return quote!(#nats_micro::ClientSubject::Static(#subject));
    }

    let mut length_terms = Vec::new();
    let mut writes = Vec::new();
    for (index, segment) in operation.subject.template.split('.').enumerate() {
        if index > 0 {
            length_terms.push(quote!(1usize));
            writes.push(quote!(__subject.push('.');));
        }
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            let ident = format_ident!("{name}");
            length_terms.push(quote!(#nats_micro::subject_param_len(&#ident)));
            writes.push(quote!(#nats_micro::push_subject_param(&mut __subject, &#ident);));
        } else {
            let length = segment.len();
            length_terms.push(quote!(#length));
            writes.push(quote!(__subject.push_str(#segment);));
        }
    }
    quote!({
        let __capacity = 0usize #(+ #length_terms)*;
        let mut __subject = String::with_capacity(__capacity);
        #(#writes)*
        debug_assert_eq!(__subject.len(), __capacity);
        #nats_micro::ClientSubject::Owned(__subject)
    })
}

fn payload_expression(operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let Some(argument) = operation
        .arguments
        .iter()
        .find(|argument| matches!(argument.kind, ArgumentKind::Payload(_)))
    else {
        return quote!(Ok(#nats_micro::Bytes::new()));
    };
    let ArgumentKind::Payload(payload) = &argument.kind else {
        unreachable!();
    };
    let ident = &argument.ident;
    let encode = match payload.codec {
        WireCodec::Json => quote!(#nats_micro::encode_json(#ident)),
        WireCodec::Protobuf => quote!(#nats_micro::encode_proto(#ident)),
        WireCodec::Utf8 => {
            quote!(Ok(#nats_micro::Bytes::copy_from_slice(#ident.as_bytes())))
        }
        WireCodec::Raw => match payload.wrapper {
            Wrapper::Direct
                if classify::last_ident(payload.decoded_type.as_ref().unwrap_or(&argument.ty))
                    .as_deref()
                    == Some("Bytes") =>
            {
                quote!(Ok(#nats_micro::Bytes::clone(#ident)))
            }
            Wrapper::Direct
                if classify::last_ident(payload.decoded_type.as_ref().unwrap_or(&argument.ty))
                    .as_deref()
                    == Some("String") =>
            {
                quote!(Ok(#nats_micro::Bytes::copy_from_slice(#ident.as_bytes())))
            }
            _ => quote!(Ok(#nats_micro::Bytes::copy_from_slice(#ident))),
        },
        WireCodec::Empty => quote!(Ok(#nats_micro::Bytes::new())),
    };
    if payload.optional {
        quote!(match #ident {
            Some(#ident) => #encode,
            None => Ok(#nats_micro::Bytes::new()),
        })
    } else {
        encode
    }
}

fn client_response_type(operation: &OperationModel) -> Type {
    let response = &operation.response;
    let nats_micro = nats_micro_path();
    let value = if response.wrapper == Wrapper::Response {
        syn::parse_quote!(#nats_micro::ClientResponse)
    } else if matches!(response.wire_type, Type::Path(ref path) if path.path.is_ident("str")) {
        syn::parse_quote!(String)
    } else {
        response.wire_type.clone()
    };
    if response.optional {
        syn::parse_quote!(Option<#value>)
    } else {
        value
    }
}

fn decoder_type(operation: &OperationModel) -> Type {
    let nats_micro = nats_micro_path();
    let response = &operation.response;
    if response.wrapper == Wrapper::Response {
        return syn::parse_quote!(#nats_micro::ClientResponseDecoder);
    }
    let decoder = match response.codec {
        WireCodec::Json if response.optional => "OptionalJsonDecoder",
        WireCodec::Json => "JsonDecoder",
        WireCodec::Protobuf if response.optional => "OptionalProtoDecoder",
        WireCodec::Protobuf => "ProtoDecoder",
        WireCodec::Utf8 if response.optional => "OptionalTextDecoder",
        WireCodec::Utf8 => "TextDecoder",
        WireCodec::Raw
            if classify::last_ident(&response.wire_type).as_deref() == Some("String")
                && response.optional =>
        {
            "OptionalTextDecoder"
        }
        WireCodec::Raw
            if classify::last_ident(&response.wire_type).as_deref() == Some("String") =>
        {
            "TextDecoder"
        }
        WireCodec::Raw
            if classify::last_ident(&response.wire_type).as_deref() == Some("Vec")
                && response.optional =>
        {
            "OptionalVecDecoder"
        }
        WireCodec::Raw if classify::last_ident(&response.wire_type).as_deref() == Some("Vec") => {
            "VecDecoder"
        }
        WireCodec::Raw if response.optional => "OptionalBytesDecoder",
        WireCodec::Raw => "BytesDecoder",
        WireCodec::Empty => "EmptyDecoder",
    };
    let decoder = format_ident!("{decoder}");
    syn::parse_quote!(#nats_micro::#decoder)
}
