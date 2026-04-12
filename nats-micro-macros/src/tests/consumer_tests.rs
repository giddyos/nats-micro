use super::process_consumer_method;
use syn::{ImplItemFn, parse_quote};

fn consumer_attr(method: &ImplItemFn) -> &syn::Attribute {
    method
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("consumer"))
        .expect("consumer attribute")
}

#[test]
fn consumer_methods_default_stream_and_durable_names() {
    let struct_ident = parse_quote!(DemoService);
    let method: ImplItemFn = parse_quote! {
        #[consumer()]
        async fn jobs() -> Result<(), nats_micro::NatsErrorResponse> {
            Ok(())
        }
    };

    let generated =
        process_consumer_method(&struct_ident, &method, consumer_attr(&method)).unwrap();
    let def_tokens = generated.accessor_fn.to_string();
    let info_tokens = generated.info_expr.to_string();

    assert!(def_tokens.contains("stream : \"DEFAULT\" . to_string ()"));
    assert!(def_tokens.contains("durable : \"jobs\" . to_string ()"));
    assert!(def_tokens.contains("new_with_shutdown_signal_support (false"));
    assert!(info_tokens.contains("stream : \"DEFAULT\" . to_string ()"));
    assert!(info_tokens.contains("durable : \"jobs\" . to_string ()"));
}

#[test]
fn consumer_methods_emit_config_auth_and_shutdown_metadata() {
    let struct_ident = parse_quote!(DemoService);
    let method: ImplItemFn = parse_quote! {
        #[cfg(feature = "demo")]
        #[consumer(
            stream = "DEMO",
            durable = "cleanup",
            concurrency_limit = 4,
            config = nats_micro::__macros::ConsumerConfig::default()
        )]
        async fn cleanup(
            auth: nats_micro::Auth,
            mut shutdown: nats_micro::ShutdownSignal,
        ) -> Result<(), nats_micro::NatsErrorResponse> {
            let _ = auth;
            shutdown.wait_for_shutdown().await;
            Ok(())
        }
    };

    let generated =
        process_consumer_method(&struct_ident, &method, consumer_attr(&method)).unwrap();
    let def_tokens = generated.accessor_fn.to_string();
    let info_tokens = generated.info_expr.to_string();

    assert_eq!(generated.attrs.len(), 1);
    assert!(def_tokens.contains("stream : \"DEMO\" . to_string ()"));
    assert!(def_tokens.contains("durable : \"cleanup\" . to_string ()"));
    assert!(def_tokens.contains("auth_required : true"));
    assert!(def_tokens.contains("concurrency_limit : Some (4"));
    assert!(def_tokens.contains("let __config"));
    assert!(def_tokens.contains("ConsumerConfig"));
    assert!(def_tokens.contains("new_with_shutdown_signal_support (true"));
    assert!(info_tokens.contains("auth_required : true"));
    assert!(info_tokens.contains("concurrency_limit : Some (4"));
    assert!(
        generated
            .accessor_fn
            .to_string()
            .contains("pub fn cleanup_consumer")
    );
}
