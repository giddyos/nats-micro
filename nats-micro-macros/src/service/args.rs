use proc_macro2::TokenStream;
use syn::{Ident, LitBool, LitStr, Type, parse::Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthIntent {
    None,
    Optional,
    Required,
}

impl AuthIntent {
    fn parse_ident(ident: &Ident) -> syn::Result<Self> {
        match ident.to_string().as_str() {
            "none" => Ok(Self::None),
            "optional" => Ok(Self::Optional),
            "required" => Ok(Self::Required),
            _ => Err(syn::Error::new(
                ident.span(),
                "auth must be `none`, `optional`, or `required`",
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Defaults {
    pub concurrency: usize,
    pub queue: Option<String>,
    pub timeout_ms: Option<u64>,
    pub auth: AuthIntent,
    pub codec: super::WireCodec,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            concurrency: 256,
            queue: None,
            timeout_ms: None,
            auth: AuthIntent::None,
            codec: super::WireCodec::Json,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ServiceArgs {
    pub name: String,
    pub version: String,
    pub description: String,
    pub prefix: String,
    pub state_type: Type,
    pub defaults: Defaults,
    pub napi: bool,
}

impl ServiceArgs {
    pub(crate) fn parse(tokens: TokenStream) -> syn::Result<Self> {
        let mut name = None;
        let mut version = None;
        let mut description = String::new();
        let mut prefix = None;
        let mut state_type: Type = syn::parse_quote!(());
        let mut defaults = Defaults::default();
        let mut napi = false;

        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("name") {
                name = Some(meta.value()?.parse::<LitStr>()?.value());
                return Ok(());
            }
            if meta.path.is_ident("version") {
                version = Some(meta.value()?.parse::<LitStr>()?.value());
                return Ok(());
            }
            if meta.path.is_ident("description") {
                description = meta.value()?.parse::<LitStr>()?.value();
                return Ok(());
            }
            if meta.path.is_ident("prefix") {
                prefix = Some(meta.value()?.parse::<LitStr>()?.value());
                return Ok(());
            }
            if meta.path.is_ident("state") {
                state_type = meta.value()?.parse()?;
                return Ok(());
            }
            if meta.path.is_ident("napi") {
                napi = if meta.input.peek(syn::Token![=]) {
                    meta.value()?.parse::<LitBool>()?.value
                } else {
                    true
                };
                return Ok(());
            }
            if meta.path.is_ident("defaults") {
                return meta.parse_nested_meta(|nested| {
                    if nested.path.is_ident("concurrency") {
                        defaults.concurrency =
                            nested.value()?.parse::<syn::LitInt>()?.base10_parse()?;
                        return Ok(());
                    }
                    if nested.path.is_ident("queue") {
                        defaults.queue = Some(nested.value()?.parse::<LitStr>()?.value());
                        return Ok(());
                    }
                    if nested.path.is_ident("timeout") {
                        let value = nested.value()?.parse::<LitStr>()?;
                        defaults.timeout_ms =
                            Some(parse_duration_ms(&value.value(), value.span())?);
                        return Ok(());
                    }
                    if nested.path.is_ident("auth") {
                        let ident = nested.value()?.parse::<Ident>()?;
                        defaults.auth = AuthIntent::parse_ident(&ident)?;
                        return Ok(());
                    }
                    if nested.path.is_ident("codec") {
                        let ident = nested.value()?.parse::<Ident>()?;
                        defaults.codec = super::WireCodec::parse_ident(&ident)?;
                        return Ok(());
                    }
                    Err(nested.error("unsupported service default"))
                });
            }
            Err(meta.error("unsupported #[service] option"))
        });
        parser.parse2(tokens)?;

        let name = name.ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "missing required `name`")
        })?;
        let version = version.ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "missing required `version`")
        })?;
        let prefix = prefix.unwrap_or_else(|| name.clone());

        Ok(Self {
            name,
            version,
            description,
            prefix,
            state_type,
            defaults,
            napi,
        })
    }
}

pub(crate) fn parse_duration_ms(value: &str, span: proc_macro2::Span) -> syn::Result<u64> {
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: u64 = number
        .parse()
        .map_err(|_| syn::Error::new(span, "duration must start with an integer"))?;
    let multiplier = match unit {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        _ => {
            return Err(syn::Error::new(
                span,
                "duration unit must be `ms`, `s`, or `m`",
            ));
        }
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| syn::Error::new(span, "duration is too large"))
}
