use super::{ServiceArgs, expand_service, process_consumer_method, process_endpoint_method};
use syn::{ImplItemFn, parse_quote};

#[test]
fn service_metadata_includes_prefix_when_present() {
    let tokens = expand_service(
        ServiceArgs {
            name: Some("demo".to_string()),
            version: Some("1.0.0".to_string()),
            description: Some("test".to_string()),
            prefix: Some("api".to_string()),
            #[cfg(feature = "macros_napi_feature")]
            napi: false,
        },
        parse_quote! {
            struct DemoService;
        },
    );

    let expanded = tokens.to_string();
    assert!(expanded.contains("ServiceMetadata :: new"));
    assert!(expanded.contains("Some (\"api\" . to_string ())"));
}

#[cfg(feature = "macros_napi_feature")]
#[test]
fn service_expansion_emits_napi_gate_module() {
    let tokens = expand_service(
        ServiceArgs {
            name: Some("demo".to_string()),
            version: Some("1.0.0".to_string()),
            description: Some("test".to_string()),
            prefix: None,
            napi: true,
        },
        parse_quote! {
            struct DemoService;
        },
    );

    let expanded = tokens.to_string();
    assert!(expanded.contains("__nats_micro_service_config_demo_service"));
    assert!(expanded.contains("emit_napi_items"));
}

#[test]
fn generated_endpoint_handlers_only_enable_shutdown_support_when_requested() {
    let struct_ident = parse_quote!(DemoService);
    let idle_method: ImplItemFn = parse_quote! {
        #[endpoint(subject = "status", group = "demo")]
        async fn status() -> Result<&'static str, nats_micro::NatsErrorResponse> {
            Ok("ok")
        }
    };
    let idle_attr = idle_method
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("endpoint"))
        .unwrap();
    let (idle_result, _) = process_endpoint_method(&struct_ident, &idle_method, idle_attr).unwrap();
    let idle_tokens = idle_result.def_fn.to_string();

    assert!(idle_tokens.contains("new_with_shutdown_signal_support (false"));

    let shutdown_method: ImplItemFn = parse_quote! {
        #[endpoint(subject = "cleanup", group = "demo")]
        async fn cleanup(
            mut shutdown: nats_micro::ShutdownSignal,
        ) -> Result<&'static str, nats_micro::NatsErrorResponse> {
            shutdown.wait_for_shutdown().await;
            Ok("ok")
        }
    };
    let shutdown_attr = shutdown_method
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("endpoint"))
        .unwrap();
    let (shutdown_result, _) =
        process_endpoint_method(&struct_ident, &shutdown_method, shutdown_attr).unwrap();
    let shutdown_tokens = shutdown_result.def_fn.to_string();

    assert!(shutdown_tokens.contains("new_with_shutdown_signal_support (true"));
}

#[test]
fn generated_consumer_handlers_only_enable_shutdown_support_when_requested() {
    let struct_ident = parse_quote!(DemoService);
    let idle_method: ImplItemFn = parse_quote! {
        #[consumer(stream = "DEMO", durable = "jobs")]
        async fn jobs() -> Result<(), nats_micro::NatsErrorResponse> {
            Ok(())
        }
    };
    let idle_attr = idle_method
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("consumer"))
        .unwrap();
    let idle_result = process_consumer_method(&struct_ident, &idle_method, idle_attr).unwrap();
    let idle_tokens = idle_result.def_fn.to_string();

    assert!(idle_tokens.contains("new_with_shutdown_signal_support (false"));

    let shutdown_method: ImplItemFn = parse_quote! {
        #[consumer(stream = "DEMO", durable = "cleanup-jobs")]
        async fn cleanup_jobs(
            mut shutdown: nats_micro::ShutdownSignal,
        ) -> Result<(), nats_micro::NatsErrorResponse> {
            shutdown.wait_for_shutdown().await;
            Ok(())
        }
    };
    let shutdown_attr = shutdown_method
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("consumer"))
        .unwrap();
    let shutdown_result =
        process_consumer_method(&struct_ident, &shutdown_method, shutdown_attr).unwrap();
    let shutdown_tokens = shutdown_result.def_fn.to_string();

    assert!(shutdown_tokens.contains("new_with_shutdown_signal_support (true"));
}
