use syn::{DeriveInput, LitInt, LitStr, Meta, Token, parse::Parser, punctuated::Punctuated};

pub(super) fn normalize_v2_error_attrs(input: &mut DeriveInput) -> syn::Result<()> {
    let syn::Data::Enum(data) = &mut input.data else {
        return Ok(());
    };
    for variant in &mut data.variants {
        let mut normalized = Vec::new();
        for attr in std::mem::take(&mut variant.attrs) {
            let Meta::List(list) = &attr.meta else {
                normalized.push(attr);
                continue;
            };
            if !attr.path().is_ident("error") || !list.tokens.to_string().contains("code") {
                normalized.push(attr);
                continue;
            }

            let options =
                Punctuated::<Meta, Token![,]>::parse_terminated.parse2(list.tokens.clone())?;
            let mut code = None;
            let mut message = None;
            for option in options {
                if option.path().is_ident("code") {
                    let Meta::NameValue(value) = option else {
                        return Err(syn::Error::new_spanned(option, "code requires an integer"));
                    };
                    let syn::Expr::Lit(expression) = value.value else {
                        return Err(syn::Error::new_spanned(value, "code requires an integer"));
                    };
                    let syn::Lit::Int(value) = expression.lit else {
                        return Err(syn::Error::new_spanned(
                            expression,
                            "code requires an integer",
                        ));
                    };
                    code = Some(value);
                } else if option.path().is_ident("message") {
                    let Meta::NameValue(value) = option else {
                        return Err(syn::Error::new_spanned(option, "message requires a string"));
                    };
                    let syn::Expr::Lit(expression) = value.value else {
                        return Err(syn::Error::new_spanned(value, "message requires a string"));
                    };
                    let syn::Lit::Str(value) = expression.lit else {
                        return Err(syn::Error::new_spanned(
                            expression,
                            "message requires a string",
                        ));
                    };
                    message = Some(value);
                } else {
                    return Err(syn::Error::new_spanned(
                        option,
                        "unsupported v2 error option",
                    ));
                }
            }
            let code: LitInt =
                code.ok_or_else(|| syn::Error::new_spanned(&attr, "missing error code"))?;
            let message: LitStr =
                message.ok_or_else(|| syn::Error::new_spanned(&attr, "missing error message"))?;
            normalized.push(syn::parse_quote!(#[code(#code)]));
            normalized.push(syn::parse_quote!(#[error(#message)]));
        }
        let is_internal = normalized
            .iter()
            .any(|attr| attr.path().is_ident("internal"));
        let has_error = normalized.iter().any(|attr| attr.path().is_ident("error"));
        if is_internal && !has_error {
            normalized.push(syn::parse_quote!(#[error("internal error")]));
        }
        variant.attrs = normalized;
    }
    Ok(())
}
