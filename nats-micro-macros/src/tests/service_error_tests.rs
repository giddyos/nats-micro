use super::expand_service_error;
use syn::parse_quote;

#[test]
fn service_error_adds_debug_and_reexported_error_derive() {
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
    assert!(expanded.contains("derive (:: nats_micro :: thiserror :: Error)"));
}

#[test]
fn service_error_rewrites_existing_error_derive_to_nats_micro() {
    let input = parse_quote! {
        #[derive(Debug, thiserror::Error)]
        enum DemoError {
            #[error("boom")]
            Boom,
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("derive (Debug , :: nats_micro :: thiserror :: Error)"));
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