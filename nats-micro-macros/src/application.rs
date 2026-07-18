use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Expr, Ident, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

use crate::util::nats_micro_path;

struct Application {
    state: Expr,
    profile: Ident,
    services: Punctuated<Type, Token![,]>,
}

impl Parse for Application {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let state_name: Ident = input.parse()?;
        if state_name != "state" {
            return Err(syn::Error::new(state_name.span(), "expected `state`"));
        }
        input.parse::<Token![:]>()?;
        let state = input.parse()?;
        input.parse::<Token![,]>()?;

        let profile_name: Ident = input.parse()?;
        if profile_name != "profile" {
            return Err(syn::Error::new(profile_name.span(), "expected `profile`"));
        }
        input.parse::<Token![:]>()?;
        let profile = input.parse()?;
        input.parse::<Token![,]>()?;

        let services_name: Ident = input.parse()?;
        if services_name != "services" {
            return Err(syn::Error::new(services_name.span(), "expected `services`"));
        }
        input.parse::<Token![:]>()?;
        let content;
        syn::bracketed!(content in input);
        let services = Punctuated::<Type, Token![,]>::parse_terminated(&content)?;
        if services.is_empty() {
            return Err(syn::Error::new(
                services_name.span(),
                "application requires at least one service",
            ));
        }
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected application option"));
        }

        Ok(Self {
            state,
            profile,
            services,
        })
    }
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    match syn::parse2::<Application>(input) {
        Ok(application) => generate(application),
        Err(error) => error.to_compile_error(),
    }
}

fn generate(application: Application) -> TokenStream {
    let nats_micro = nats_micro_path();
    let state = application.state;
    let profile = application.profile;
    let services: Vec<_> = application.services.into_iter().collect();
    let mut pairs = Vec::new();
    for (left_index, left) in services.iter().enumerate() {
        for right in services.iter().skip(left_index + 1) {
            pairs.push(quote! {
                const _: () = {
                    #nats_micro::assert_services_compatible(
                        &#left::SPEC,
                        &#right::SPEC,
                    );
                };
            });
        }
    }

    quote! {
        #(#pairs)*

        #[#nats_micro::main]
        async fn main() -> #nats_micro::Result<()> {
            #nats_micro::App::new(#state)
                .profile(#nats_micro::Profile::#profile)
                #(.serve(<#services as Default>::default()))*
                .run()
                .await
        }
    }
}
