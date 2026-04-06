use super::{ServiceArgs, expand_service, expand_service_handlers, validate_service_args};
use syn::{ItemImpl, parse_quote};

#[test]
fn service_metadata_includes_prefix_when_present() {
    let item_struct = parse_quote! {
        struct DemoService;
    };
    let tokens = expand_service(
        ServiceArgs {
            name: "demo".to_string(),
            version: "1.0.0".to_string(),
            description: Some("test".to_string()),
            prefix: Some("api".to_string()),
            #[cfg(feature = "macros_napi_feature")]
            napi: false,
        },
        &item_struct,
    );

    let expanded = tokens.to_string();
    assert!(expanded.contains("ServiceMetadata :: new"));
    assert!(expanded.contains("Some (\"api\" . to_string ())"));
}

#[cfg(feature = "macros_napi_feature")]
#[test]
fn service_expansion_emits_napi_gate_module() {
    let item_struct = parse_quote! {
        struct DemoService;
    };
    let tokens = expand_service(
        ServiceArgs {
            name: "demo".to_string(),
            version: "1.0.0".to_string(),
            description: Some("test".to_string()),
            prefix: None,
            napi: true,
        },
        &item_struct,
    );

    let expanded = tokens.to_string();
    assert!(expanded.contains("__nats_micro_service_config_demo_service"));
    assert!(expanded.contains("emit_napi_items"));
}

#[test]
fn service_handlers_collect_endpoint_and_consumer_wiring() {
    let item_impl: ItemImpl = parse_quote! {
        impl DemoService {
            #[cfg(feature = "demo")]
            #[endpoint(subject = "status", group = "demo")]
            async fn status() -> Result<&'static str, nats_micro::NatsErrorResponse> {
                Ok("ok")
            }

            #[consumer(stream = "DEMO", durable = "jobs")]
            async fn jobs() -> Result<(), nats_micro::NatsErrorResponse> {
                Ok(())
            }

            fn helper(&self) {}
        }
    };

    let expanded = expand_service_handlers(&item_impl).to_string();

    assert!(expanded.contains("impl DemoService"));
    assert!(expanded.contains("NatsService for DemoService"));
    assert!(expanded.contains("inventory :: submit"));
    assert!(expanded.contains("pub fn __ep_status"));
    assert!(expanded.contains("pub fn __con_jobs"));
    assert!(expanded.contains("pub fn status_endpoint"));
    assert!(expanded.contains("pub fn jobs_consumer"));
    assert!(expanded.contains("async fn status ()"));
    assert!(expanded.contains("async fn jobs ()"));
    assert!(!expanded.contains("# [ endpoint"));
    assert!(!expanded.contains("# [ consumer"));
    assert!(expanded.contains("cfg (feature = \"demo\")"));
    assert!(expanded.contains("pub fn __ep_status"));
}

#[test]
fn service_args_reject_non_numeric_semver() {
    let item_struct = parse_quote! {
        struct DemoService;
    };

    assert!(
        validate_service_args(
            &ServiceArgs {
                name: "demo".to_string(),
                version: "1.2.3".to_string(),
                description: None,
                prefix: None,
                #[cfg(feature = "macros_napi_feature")]
                napi: false,
            },
            &item_struct,
        )
        .is_ok()
    );

    let err = validate_service_args(
        &ServiceArgs {
            name: "demo".to_string(),
            version: "1.2".to_string(),
            description: None,
            prefix: None,
            #[cfg(feature = "macros_napi_feature")]
            napi: false,
        },
        &item_struct,
    )
    .unwrap_err();

    assert!(err.to_string().contains("x.x.x"));
}

#[test]
fn service_handlers_emit_client_module_when_enabled() {
    let item_impl: ItemImpl = parse_quote! {
        impl DemoService {
            #[endpoint(subject = "status", group = "demo")]
            async fn status() -> Result<&'static str, nats_micro::NatsErrorResponse> {
                Ok("ok")
            }
        }
    };

    let expanded = expand_service_handlers(&item_impl).to_string();

    if cfg!(feature = "macros_client_feature") {
        assert!(expanded.contains("pub mod demo_service_client"));
        assert!(expanded.contains("DemoServiceClient"));
    } else {
        assert!(!expanded.contains("pub mod demo_service_client"));
    }
}
