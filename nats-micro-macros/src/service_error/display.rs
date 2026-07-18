use heck::AsShoutySnakeCase;

pub(super) fn default_kind(variant: &syn::Ident) -> String {
    format!("{}", AsShoutySnakeCase(&variant.to_string()))
}
