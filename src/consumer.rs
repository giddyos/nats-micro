use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Pat, Type};

#[derive(Debug, FromMeta)]
pub struct ConsumerArgs {
    stream: Option<String>,
    durable: Option<String>,
    filter_subject: Option<String>,
    #[darling(default)]
    ack_on_success: bool,
}

pub fn expand_consumer(args: ConsumerArgs, func: ItemFn) -> TokenStream {
    let stream = args.stream.as_deref().unwrap_or("DEFAULT");
    let durable = args.durable.unwrap_or_else(|| func.sig.ident.to_string());
    let filter_subject = args.filter_subject.as_deref().unwrap_or(">");
    let ack_on_success = args.ack_on_success;

    let fn_name = &func.sig.ident;
    let def_fn_name = format_ident!("{}_consumer", fn_name);
    let handler = match build_handler(&func) {
        Ok(handler) => handler,
        Err(err) => return err,
    };

    quote! {
        #func

        #[doc(hidden)]
        pub fn #def_fn_name() -> ::nats_micro::ConsumerDefinition {
            ::nats_micro::ConsumerDefinition {
                stream: #stream.to_string(),
                durable: #durable.to_string(),
                filter_subject: #filter_subject.to_string(),
                ack_on_success: #ack_on_success,
                handler: #handler,
            }
        }
    }
}

fn build_handler(func: &ItemFn) -> Result<TokenStream, TokenStream> {
    let fn_name = &func.sig.ident;
    let mut extractors = Vec::new();
    let mut args = Vec::new();

    for input in &func.sig.inputs {
        let FnArg::Typed(pat_type) = input else {
            return Err(syn::Error::new_spanned(input, "consumer handlers cannot take a receiver")
                .to_compile_error());
        };

        let Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &pat_type.pat,
                "consumer handler arguments must be simple identifiers",
            )
            .to_compile_error());
        };

        let ident = &pat_ident.ident;
        let ty = &pat_type.ty;
        let ctx_expr = if is_subject_param(ty) {
            quote! { &ctx.with_param_name(::std::string::ToString::to_string(stringify!(#ident))) }
        } else {
            quote! { &ctx }
        };

        extractors.push(quote! {
            let #ident: #ty = match <#ty as ::nats_micro::__private::FromRequest>::from_request(#ctx_expr) {
                Ok(value) => value,
                Err(err) => return Err(err),
            };
        });
        args.push(quote! { #ident });
    }

    Ok(quote! {
        ::nats_micro::HandlerFn::new(move |ctx: ::nats_micro::__private::RequestContext| {
            ::std::boxed::Box::pin(async move {
                let __request_id = ctx.request.request_id.clone();
                #(#extractors)*
                let response = #fn_name(#(#args),*)
                    .await
                    .map_err(|err| ::nats_micro::__private::IntoNatsError::into_nats_error(err, __request_id.clone()))?;
                ::nats_micro::__private::IntoNatsResponse::into_response(response, __request_id)
            })
        })
    })
}

fn is_subject_param(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident == "SubjectParam")
        .unwrap_or(false)
}
