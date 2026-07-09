use super::expand_service_error;
use syn::parse_quote;

#[test]
fn service_error_adds_debug_and_error_impls() {
    let input = parse_quote! {
        #[derive(Clone)]
        pub enum DemoError {
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
fn service_error_external_derive_skips_error_impls_and_from_impls() {
    let input = parse_quote! {
        #[derive(Debug, thiserror::Error)]
        pub enum DemoError {
            #[error("boom")]
            Boom,

            #[error("io failed")]
            Io(#[from] std::io::Error),
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("derive (Debug , thiserror :: Error)"));
    assert!(!expanded.contains("impl :: std :: fmt :: Display for DemoError"));
    assert!(!expanded.contains("impl :: std :: error :: Error for DemoError"));
    assert!(!expanded.contains("impl :: std :: convert :: From < std :: io :: Error >"));
    assert!(expanded.contains("impl :: nats_micro :: IntoNatsError for DemoError"));
}

#[test]
fn service_error_external_derive_preserves_thiserror_attrs() {
    let input = parse_quote! {
        #[derive(Debug, thiserror::Error)]
        pub enum DemoError {
            #[code(409)]
            #[internal]
            #[error("wrapped")]
            Wrapped {
                #[from]
                source: std::io::Error,
                #[backtrace]
                backtrace: std::backtrace::Backtrace,
            },
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("# [error (\"wrapped\")]"));
    assert!(expanded.contains("# [from]"));
    assert!(expanded.contains("# [backtrace]"));
    assert!(!expanded.contains("# [code"));
    assert!(!expanded.contains("# [internal]"));
}

#[test]
fn service_error_formats_common_thiserror_shorthand_without_thiserror() {
    let input = parse_quote! {
        pub enum DemoError {
            #[error("retry after {0} seconds")]
            RetryAfter(u64),
            #[error("invalid range {min:?}..{max}")]
            InvalidRange { min: i64, max: i64 },
            #[error("invalid value {}", .0)]
            InvalidValue(String),
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("write ! (f , \"retry after {} seconds\" , __field_0)"));
    assert!(expanded.contains("write ! (f , \"invalid range {:?}..{}\" , __min , __max)"));
    assert!(expanded.contains("write ! (f , \"invalid value {}\" , __field_0)"));
}

#[test]
fn service_error_supports_from_source_and_transparent() {
    let input = parse_quote! {
        pub enum DemoError {
            #[internal]
            #[error("io failed")]
            Io(#[from] std::io::Error),

            #[error("outer failed")]
            Outer { #[source] source: std::io::Error },

            #[error(transparent)]
            Transparent(std::io::Error),
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("impl :: std :: convert :: From < std :: io :: Error >"));
    assert!(expanded.contains("fn source"));
    assert!(expanded.contains("Display :: fmt"));
    assert!(expanded.contains("ServiceErrorMatch :: Untyped"));
}

#[test]
fn service_error_preserves_non_error_derives() {
    let input = parse_quote! {
        #[derive(Clone, PartialEq)]
        pub enum DemoError {
            #[error("boom")]
            Boom,
        }
    };

    let expanded = expand_service_error(input).to_string();

    assert!(expanded.contains("derive (Clone , PartialEq)"));
    assert!(expanded.contains("derive (Debug)"));
}

#[test]
fn service_error_napi_output_respects_macro_feature() {
    let input = parse_quote! {
        pub enum DemoError {
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
