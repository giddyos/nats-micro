use crate::{
    client::build_endpoint_client_spec,
    endpoint::EndpointArgs,
    napi::{gen_napi_asserts, generate_client_napi_module},
};
use syn::{ImplItemFn, parse_quote};

fn endpoint_args(subject: &str, group: Option<&str>) -> EndpointArgs {
    EndpointArgs {
        subject: subject.to_string(),
        group: group.map(str::to_string),
        queue_group: None,
        concurrency_limit: None,
    }
}

fn endpoint_spec_for(
    method: ImplItemFn,
    subject: &str,
    group: Option<&str>,
) -> crate::client::ClientEndpointSpec {
    let attrs = method
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .cloned()
        .collect();
    build_endpoint_client_spec(&method, &endpoint_args(subject, group), attrs).unwrap()
}

#[test]
fn generated_napi_asserts_cover_payload_and_response_dtos() {
    let endpoint = endpoint_spec_for(
        parse_quote! {
            async fn sum(
                payload: nats_micro::Payload<nats_micro::Json<SumRequest>>,
            ) -> Result<nats_micro::Json<SumResponse>, nats_micro::NatsErrorResponse> {
                let _ = payload;
                Ok(nats_micro::Json(SumResponse))
            }
        },
        "sum",
        Some("math"),
    );

    let expanded = gen_napi_asserts(&[endpoint]).to_string();

    assert!(expanded.contains("assert_napi_object :: < SumRequest >"));
    assert!(expanded.contains("assert_napi_object :: < SumResponse >"));
}

#[test]
fn generated_napi_module_wraps_generated_rust_client() {
    let service_ident = parse_quote!(DemoService);
    let endpoints = vec![
        endpoint_spec_for(
            parse_quote! {
                async fn get_profile(
                    user_id: nats_micro::SubjectParam<String>,
                ) -> Result<nats_micro::Json<UserProfile>, nats_micro::NatsErrorResponse> {
                    let _ = user_id;
                    Ok(nats_micro::Json(UserProfile))
                }
            },
            "users.{user_id}.profile",
            Some("accounts"),
        ),
        endpoint_spec_for(
            parse_quote! {
                async fn echo(
                    payload: nats_micro::Payload<Vec<u8>>,
                ) -> Result<Vec<u8>, nats_micro::NatsErrorResponse> {
                    Ok(payload.0)
                }
            },
            "echo",
            Some("raw"),
        ),
    ];

    let expanded =
        generate_client_napi_module(&service_ident, "DemoService", &endpoints).to_string();

    assert!(expanded.contains("JsDemoServiceClient"));
    assert!(expanded.contains("DemoServiceClientConnectOptions"));
    assert!(expanded.contains("demo_service_client :: DemoServiceClient"));
    assert!(expanded.contains("pub async fn connect"));
    assert!(expanded.contains("JsDemoClientFrameworkError"));
    assert!(expanded.contains("JsDemoClientTransportError"));
    assert!(expanded.contains("__demo_service_generic_napi_error"));
    assert!(expanded.contains("js_name = \"getProfile\""));
    assert!(expanded.contains("DemoServiceGetProfileArgs"));
    assert!(expanded.contains("Buffer"));
}

#[test]
fn generated_napi_module_emits_service_error_interface() {
    let service_ident = parse_quote!(DemoService);
    let endpoints = vec![
        endpoint_spec_for(
            parse_quote! {
                async fn first() -> Result<(), DemoDomainError> {
                    Ok(())
                }
            },
            "first",
            Some("demo"),
        ),
        endpoint_spec_for(
            parse_quote! {
                async fn second() -> Result<(), AnotherDemoError> {
                    Ok(())
                }
            },
            "second",
            Some("demo"),
        ),
        endpoint_spec_for(
            parse_quote! {
                async fn third() -> Result<(), nats_micro::NatsErrorResponse> {
                    Ok(())
                }
            },
            "third",
            Some("demo"),
        ),
    ];

    let expanded =
        generate_client_napi_module(&service_ident, "DemoService", &endpoints).to_string();

    assert!(expanded.contains("assert_napi_service_error :: < DemoDomainError >"));
    assert!(expanded.contains("assert_napi_service_error :: < AnotherDemoError >"));
    assert!(expanded.contains("js_name = \"DemoClientError\""));
    assert!(expanded.contains("js_name = \"DemoClientFrameworkError\""));
    assert!(expanded.contains("js_name = \"DemoClientTransportError\""));
    assert!(expanded.contains("ts_type = \"true\""));
    assert!(expanded.contains("ts_type = \"JsDemoDomainError | JsAnotherDemoError\""));
    assert!(expanded.contains("ts_type = \"JsDemoClientFrameworkError | string\""));
    assert!(expanded.contains("ts_type = \"JsDemoClientTransportError | string\""));
    assert!(expanded.contains("pub is_service_error : bool"));
    assert!(expanded.contains("pub is_framework_error : bool"));
    assert!(expanded.contains("pub is_transport_error : bool"));
    assert!(expanded.contains("pub request_id : String"));
    assert!(expanded.contains("__demo_service_service_napi_error"));
    assert!(expanded.contains("__demo_service_generic_napi_error"));
}

#[test]
fn generated_napi_module_supports_optional_call_headers() {
    let service_ident = parse_quote!(DemoService);
    let endpoints = vec![
        endpoint_spec_for(
            parse_quote! {
                async fn health() -> Result<String, nats_micro::NatsErrorResponse> {
                    Ok("ok".to_string())
                }
            },
            "health",
            None,
        ),
        endpoint_spec_for(
            parse_quote! {
                async fn get_profile(
                    user_id: nats_micro::SubjectParam<String>,
                ) -> Result<nats_micro::Json<UserProfile>, nats_micro::NatsErrorResponse> {
                    let _ = user_id;
                    Ok(nats_micro::Json(UserProfile))
                }
            },
            "users.{user_id}.profile",
            Some("accounts"),
        ),
        endpoint_spec_for(
            parse_quote! {
                async fn echo(
                    payload: nats_micro::Payload<Vec<u8>>,
                ) -> Result<Vec<u8>, nats_micro::NatsErrorResponse> {
                    Ok(payload.0)
                }
            },
            "echo",
            Some("raw"),
        ),
    ];

    let expanded =
        generate_client_napi_module(&service_ident, "DemoService", &endpoints).to_string();

    assert!(expanded.contains("pub struct DemoServiceClientHeader"));
    assert!(expanded.contains("js_name = \"Header\""));
    if cfg!(feature = "macros_encryption_feature") {
        assert!(expanded.contains("encrypted"));
    } else {
        assert!(!expanded.contains("pub encrypted : bool"));
        assert!(!expanded.contains("encrypted : value . encrypted"));
    }
    assert!(expanded.contains("headers"));
    assert!(expanded.contains("pub async fn health_with_headers"));
    assert!(expanded.contains("__napi_health"));
    assert!(expanded.contains("__napi_get_profile"));
    assert!(expanded.contains("DemoServiceGetProfileArgs"));
    assert!(expanded.contains("js_name = \"setDefaultHeaders\""));
}
