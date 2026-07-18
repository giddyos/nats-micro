use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{Type, spanned::Spanned};

use super::{
    ArgumentKind, MethodModel, OperationKind, OperationModel, PayloadModel, ServiceModel,
    WireCodec, Wrapper, classify, metadata,
};
use crate::util::nats_micro_path;

pub(crate) fn generate(model: &ServiceModel) -> TokenStream {
    let items: Vec<_> = model
        .methods
        .iter()
        .filter_map(MethodModel::operation)
        .filter(|operation| operation.kind != OperationKind::Consumer)
        .map(|operation| generate_operation(model, operation))
        .collect();
    quote!(#(#items)*)
}

fn generate_operation(model: &ServiceModel, operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let operation_type = metadata::operation_type(model, operation);
    let state_type = &model.state_type;
    let spec = metadata::operation_spec(operation);
    let attrs = conditional_attrs(&operation.method);

    match operation.kind {
        OperationKind::Publish => quote! {
            #(#attrs)*
            #[doc(hidden)]
            pub struct #operation_type;

            impl #nats_micro::PublishOperation for #operation_type {
                const SPEC: #nats_micro::OperationSpec = #spec;
            }
        },
        OperationKind::Request => {
            let body = request_body(model, operation);
            quote! {
                #(#attrs)*
                #[doc(hidden)]
                pub struct #operation_type;

                impl #nats_micro::RequestEndpoint<#state_type> for #operation_type {
                    const SPEC: #nats_micro::OperationSpec = #spec;

                    async fn call<'__request>(
                        state: &'__request #state_type,
                        request: #nats_micro::Request<'__request>,
                    ) -> #nats_micro::DispatchResult {
                        #body
                    }
                }
            }
        }
        OperationKind::Subscribe => {
            let bindings = argument_bindings(model, operation);
            let call = handler_call(model, operation);
            quote! {
                #(#attrs)*
                #[doc(hidden)]
                pub struct #operation_type;

                impl #nats_micro::SubscriptionHandler<#state_type> for #operation_type {
                    const SPEC: #nats_micro::OperationSpec = #spec;

                    async fn call<'__request>(
                        state: &'__request #state_type,
                        request: #nats_micro::Request<'__request>,
                    ) -> Result<(), #nats_micro::ErrorReply> {
                        #bindings
                        let _ = #call;
                        Ok(())
                    }
                }
            }
        }
        OperationKind::Consumer => quote!(),
    }
}

pub(crate) fn argument_bindings(model: &ServiceModel, operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let state_type = &model.state_type;
    let bindings = operation.arguments.iter().map(|argument| {
        let ident = &argument.ident;
        let ty = &argument.ty;
        match &argument.kind {
            ArgumentKind::State => quote!(let #ident = state;),
            ArgumentKind::StateRef(inner) => quote_spanned! {ty.span()=>
                let #ident: #ty = #nats_micro::StateRef::new(
                    <#inner as #nats_micro::FromAppState<#state_type>>::from_state(state)
                );
            },
            ArgumentKind::Subject { name, segment } => quote_spanned! {ty.span()=>
                let __raw = #nats_micro::segment(request.subject(), #segment)
                    .ok_or_else(|| #nats_micro::ErrorReply::missing_subject_parameter(
                        #name,
                        request.request_id().existing(),
                    ))?;
                let #ident: #ty =
                    <#ty as #nats_micro::FromSubject<'__request>>::from_subject(__raw)
                        .map_err(|error| #nats_micro::ErrorReply::invalid_subject_parameter(
                            #name,
                            error,
                            request.request_id().existing(),
                        ))?;
            },
            ArgumentKind::Header { name, optional } => {
                if *optional {
                    quote_spanned! {ty.span()=>
                        let #ident: #ty = request.headers().get(#name);
                    }
                } else {
                    quote_spanned! {ty.span()=>
                        let #ident: #ty = request.headers().get(#name).ok_or_else(|| {
                            #nats_micro::ErrorReply::missing_header(
                                #name,
                                request.request_id().existing(),
                            )
                        })?;
                    }
                }
            }
            ArgumentKind::Headers => quote!(let #ident: #ty = request.headers();),
            ArgumentKind::RequestMeta => quote!(let #ident: #ty = request.meta();),
            ArgumentKind::Auth { claims, optional } => {
                if *optional {
                    quote_spanned! {ty.span()=>
                        let #ident: #ty = match <#claims as #nats_micro::FromRequestMeta>
                            ::from_request_meta(request.meta()).await
                        {
                            Ok(claims) => Some(#nats_micro::Auth::new(claims)),
                            Err(#nats_micro::AuthError::MissingCredentials) => None,
                            Err(error) => return Err(
                                #nats_micro::IntoServiceError::into_service_error(
                                    error,
                                    request.request_id(),
                                )
                            ),
                        };
                    }
                } else {
                    quote_spanned! {ty.span()=>
                        let #ident: #ty = #nats_micro::Auth::new(
                            <#claims as #nats_micro::FromRequestMeta>
                                ::from_request_meta(request.meta())
                                .await
                                .map_err(|error| {
                                    #nats_micro::IntoServiceError::into_service_error(
                                        error,
                                        request.request_id(),
                                    )
                                })?
                        );
                    }
                }
            }
            ArgumentKind::Payload(payload) => {
                let decoded = payload_decode(ty, payload);
                if payload.optional {
                    quote_spanned! {ty.span()=>
                        let #ident: #ty = if request.body().is_empty() {
                            None
                        } else {
                            Some(#decoded)
                        };
                    }
                } else {
                    quote_spanned! {ty.span()=>
                        let #ident: #ty = #decoded;
                    }
                }
            }
        }
    });
    quote!(#(#bindings)*)
}

pub(crate) fn handler_call(model: &ServiceModel, operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let service = &model.service_ident;
    let method = &operation.method.sig.ident;
    let args = operation.arguments.iter().map(|argument| &argument.ident);
    let call = quote!(#service::#method(#(#args),*).await);
    if operation.response.error_type.is_some() {
        quote! {
            match #call {
                Ok(value) => value,
                Err(error) => return Err(
                    #nats_micro::IntoServiceError::into_service_error(
                        error,
                        request.request_id(),
                    )
                ),
            }
        }
    } else {
        call
    }
}

fn request_body(model: &ServiceModel, operation: &OperationModel) -> TokenStream {
    let bindings = argument_bindings(model, operation);
    let call = handler_call(model, operation);
    let response = encode_response(operation);
    quote! {
        #bindings
        let __response = #call;
        #response
    }
}

fn encode_response(operation: &OperationModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let response = &operation.response;
    let encode = encode_value(&response.ok_type, response.codec, response.wrapper);
    if response.optional {
        quote! {
            let Some(__response) = __response else {
                return Ok(#nats_micro::Response::optional_none());
            };
            #encode
        }
    } else {
        encode
    }
}

fn encode_value(ok_type: &Type, codec: WireCodec, wrapper: Wrapper) -> TokenStream {
    let nats_micro = nats_micro_path();
    match wrapper {
        Wrapper::Response => quote!(Ok(__response)),
        Wrapper::Json => quote! {
            let payload = #nats_micro::encode_json(&__response.0)?;
            Ok(#nats_micro::Response::Payload(payload))
        },
        Wrapper::Proto => quote! {
            let payload = #nats_micro::encode_proto(&__response.0)?;
            Ok(#nats_micro::Response::Payload(payload))
        },
        Wrapper::Direct if classify::is_unit(ok_type) => {
            quote!(Ok(#nats_micro::Response::Empty))
        }
        Wrapper::Direct => match codec {
            WireCodec::Json => quote_spanned! {ok_type.span()=>
                let payload = #nats_micro::encode_json(&__response)?;
                Ok(#nats_micro::Response::Payload(payload))
            },
            WireCodec::Protobuf => quote_spanned! {ok_type.span()=>
                let payload = #nats_micro::encode_proto(&__response)?;
                Ok(#nats_micro::Response::Payload(payload))
            },
            WireCodec::Raw | WireCodec::Utf8 => {
                quote!(Ok(#nats_micro::Response::bytes(__response)))
            }
            WireCodec::Empty => quote!(Ok(#nats_micro::Response::Empty)),
        },
        Wrapper::Body | Wrapper::Text => {
            quote!(Ok(#nats_micro::Response::bytes(__response.0)))
        }
    }
}

fn payload_decode(ty: &Type, payload: &PayloadModel) -> TokenStream {
    let nats_micro = nats_micro_path();
    let value_type = payload.decoded_type.as_ref();
    match payload.wrapper {
        Wrapper::Body => quote!(#nats_micro::Body(request.body())),
        Wrapper::Text => quote!(#nats_micro::Text(#nats_micro::decode_text(&request)?)),
        Wrapper::Json => {
            let value_type = value_type.expect("JSON type");
            quote!(#nats_micro::Json(#nats_micro::decode_json::<#value_type>(&request)?))
        }
        Wrapper::Proto => {
            let value_type = value_type.expect("protobuf type");
            quote!(#nats_micro::Proto(#nats_micro::decode_proto::<#value_type>(&request)?))
        }
        Wrapper::Direct => {
            let value_type = value_type.unwrap_or(ty);
            match payload.codec {
                WireCodec::Json => quote!(#nats_micro::decode_json::<#value_type>(&request)?),
                WireCodec::Protobuf => {
                    quote!(#nats_micro::decode_proto::<#value_type>(&request)?)
                }
                WireCodec::Utf8 => quote!(#nats_micro::decode_text(&request)?),
                WireCodec::Empty => quote!(()),
                WireCodec::Raw => match classify::last_ident(value_type).as_deref() {
                    Some("Bytes") => quote!(#nats_micro::Bytes::copy_from_slice(request.body())),
                    Some("Vec") => quote!(request.body().to_vec()),
                    Some("String") => quote!(#nats_micro::decode_text(&request)?.to_owned()),
                    _ => quote!(request.body()),
                },
            }
        }
        Wrapper::Response => quote!(request),
    }
}

pub(crate) fn conditional_attrs(method: &syn::ImplItemFn) -> Vec<syn::Attribute> {
    method
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .cloned()
        .collect()
}
