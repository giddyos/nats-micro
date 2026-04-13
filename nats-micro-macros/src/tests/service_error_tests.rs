use super::expand_service_error;
use syn::parse_quote;

#[test]
fn service_error_adds_debug_and_error_impls() {
    let input = parse_quote! {
        #[derive(Clone)]
        enum DemoError {
            #[error("boom")]
            Boom,
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("derive (Clone)"));
    assert!(expanded.contains("derive (Debug)"));
    assert!(expanded.contains("impl :: std :: fmt :: Display for DemoError"));
    assert!(expanded.contains("impl :: std :: error :: Error for DemoError"));
}

#[test]
fn service_error_strips_existing_error_derive() {
    let input = parse_quote! {
        #[derive(Debug, thiserror::Error)]
        enum DemoError {
            #[error("boom")]
            Boom,
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("derive (Debug)"));
    assert!(!expanded.contains("thiserror"));
}

#[test]
fn service_error_formats_tuple_and_named_fields_without_thiserror() {
    let input = parse_quote! {
        enum DemoError {
            #[error("retry after {0} seconds")]
            RetryAfter(u64),
            #[error("invalid range {min}..{max}")]
            InvalidRange { min: i64, max: i64 },
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("write ! (f , \"retry after {} seconds\" , __field_0)"));
    assert!(expanded.contains("write ! (f , \"invalid range {}..{}\" , __min , __max)"));
}

#[test]
fn service_error_napi_output_respects_macro_feature() {
    let input = parse_quote! {
        enum DemoError {
            #[error("boom")]
            Boom,
        }
    };

    let expanded = expand_service_error(input).to_string();

    if cfg!(feature = "macros_napi_feature") {
        assert!(expanded.contains("napi_derive :: napi"));
        assert!(expanded.contains("NapiServiceError"));
    } else {
        assert!(!expanded.contains("napi_derive :: napi"));
        assert!(!expanded.contains("NapiServiceError"));
    }
}
