use darling::{FromMeta, ast::NestedMeta};
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{ImplItemFn, Meta};

pub fn nats_micro_path() -> syn::Path {
    match crate_name("nats-micro") {
        Ok(FoundCrate::Itself) => syn::parse_quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            syn::parse_str(&format!("::{name}")).unwrap_or_else(|_| syn::parse_quote!(::nats_micro))
        }
        Err(_) => syn::parse_quote!(::nats_micro),
    }
}

pub fn error_stream<T: ToTokens>(span: Span, msg: &str, original: T) -> TokenStream {
    let err = syn::Error::new(span, msg);
    let compile_error = err.to_compile_error();
    quote! {
        #compile_error
        #original
    }
}

pub(crate) fn parse_attr<T: FromMeta>(attr: &syn::Attribute) -> Result<T, TokenStream> {
    let Meta::List(meta_list) = &attr.meta else {
        return Err(
            syn::Error::new_spanned(attr, "expected attribute arguments in parentheses")
                .to_compile_error(),
        );
    };

    let nested = NestedMeta::parse_meta_list(meta_list.tokens.clone())
        .map_err(|error| darling::Error::from(error).write_errors())?;
    T::from_list(&nested).map_err(darling::Error::write_errors)
}

pub(crate) fn conditional_attrs(method: &ImplItemFn) -> Vec<syn::Attribute> {
    method
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .cloned()
        .collect()
}
