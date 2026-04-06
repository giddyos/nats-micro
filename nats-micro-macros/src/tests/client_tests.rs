use super::{
    ClientPayloadShape, ClientResponseShape, RawValueKind, build_client_module_spec,
    build_endpoint_client_spec, generate_client_module,
};
use crate::endpoint::EndpointArgs;
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
    method: &ImplItemFn,
    subject: &str,
    group: Option<&str>,
) -> super::ClientEndpointSpec {
    let attrs = method
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .cloned()
        .collect();
    build_endpoint_client_spec(method, &endpoint_args(subject, group), attrs).unwrap()
}

#[test]
fn generated_client_uses_service_metadata_prefix() {
    let struct_ident = parse_quote!(DemoService);
    let module_spec = build_client_module_spec(&struct_ident, "DemoService", &[]);
    assert_eq!(module_spec.module_name.to_string(), "demo_service_client");
    assert_eq!(
        module_spec.client_struct_name.to_string(),
        "DemoServiceClient"
    );

    let expanded = generate_client_module(&struct_ident, "DemoService", &[]).to_string();

    assert!(expanded.contains("let service_meta = DemoService :: __nats_micro_service_meta ()"));
    assert!(expanded.contains("prefix : service_meta . subject_prefix"));
    assert!(expanded.contains("service_version : String"));
    assert!(expanded.contains("build_subject"));
    assert!(expanded.contains("self . prefix . as_deref ()"));
    assert!(expanded.contains("X_CLIENT_VERSION_HEADER"));
    assert!(expanded.contains("with_required_header"));
    if cfg!(feature = "macros_encryption_feature") {
        assert!(expanded.contains("recipient : Option <"));
        assert!(expanded.contains("recipient_public_key : Option < [u8 ; 32] >"));
        assert!(expanded.contains(
            "Some (recipient) => options . with_default_recipient (recipient . clone ())"
        ));
        assert!(!expanded.contains("# [cfg (feature = \"encryption\") ]"));
    }
    assert!(!expanded.contains("# [cfg (feature = \"macros_encryption_feature\") ]"));
}

#[test]
fn generated_client_omits_recipient_constructor_args_without_encryption() {
    let struct_ident = parse_quote!(DemoService);
    let mut module_spec = build_client_module_spec(&struct_ident, "DemoService", &[]);
    module_spec.encryption_enabled = false;

    let expanded = module_spec.render_rust_tokens().to_string();

    assert!(
        expanded.contains("pub fn new (client : :: nats_micro :: async_nats :: Client) -> Self")
    );
    assert!(expanded.contains("pub fn with_prefix"));
    assert!(expanded.contains("prefix : impl Into < String >"));
    assert!(!expanded.contains("recipient_public_key"));
    assert!(!expanded.contains("recipient : Option <"));
    assert!(!expanded.contains("with_recipient"));
}

#[test]
fn client_module_spec_exposes_endpoint_shapes_for_future_emitters() {
    let struct_ident = parse_quote!(DemoService);
    let method: ImplItemFn = parse_quote! {
        async fn maybe_secret(
            user_id: nats_micro::SubjectParam<String>,
            payload: nats_micro::Payload<Option<nats_micro::Encrypted<String>>>,
        ) -> Result<Option<nats_micro::Json<JsonResponse>>, nats_micro::NatsErrorResponse> {
            let _ = user_id;
            let _ = payload;
            Ok(None)
        }
    };
    let endpoint = endpoint_spec_for(&method, "users.{user_id}.secret", Some("demo"));
    let module_spec = build_client_module_spec(
        &struct_ident,
        "DemoService",
        std::slice::from_ref(&endpoint),
    );
    let endpoint = &module_spec.endpoints[0];

    assert_eq!(endpoint.subject.template, "users.{user_id}.secret");
    assert_eq!(endpoint.subject.params.len(), 1);
    assert_eq!(endpoint.subject.params[0].name, "user_id");
    assert!(matches!(
        endpoint.payload.as_ref().map(|payload| &payload.shape),
        Some(ClientPayloadShape::Raw(RawValueKind::String))
    ));
    assert!(
        endpoint
            .payload
            .as_ref()
            .is_some_and(|payload| payload.optional)
    );
    assert!(
        endpoint
            .payload
            .as_ref()
            .is_some_and(|payload| payload.encrypted)
    );
    assert!(matches!(
        &endpoint.response.shape,
        ClientResponseShape::Json(_)
    ));
    assert!(endpoint.response.optional);
}

#[test]
fn optional_payloads_preserve_marker_metadata_for_clients() {
    let json_method: ImplItemFn = parse_quote! {
        async fn maybe_json(
            payload: nats_micro::Payload<Option<nats_micro::Json<JsonRequest>>>,
        ) -> Result<(), nats_micro::NatsErrorResponse> {
            let _ = payload;
            Ok(())
        }
    };
    let json_payload = endpoint_spec_for(&json_method, "maybe-json", None);
    assert!(matches!(
        json_payload.payload.as_ref().map(|payload| &payload.shape),
        Some(ClientPayloadShape::Json(_))
    ));
    assert!(
        json_payload
            .payload
            .as_ref()
            .is_some_and(|payload| payload.optional)
    );
    assert!(
        json_payload
            .payload
            .as_ref()
            .is_some_and(|payload| !payload.encrypted)
    );

    let proto_method: ImplItemFn = parse_quote! {
        async fn maybe_proto(
            payload: nats_micro::Payload<Option<nats_micro::Proto<ProtoRequest>>>,
        ) -> Result<(), nats_micro::NatsErrorResponse> {
            let _ = payload;
            Ok(())
        }
    };
    let proto_payload = endpoint_spec_for(&proto_method, "maybe-proto", None);
    assert!(matches!(
        proto_payload.payload.as_ref().map(|payload| &payload.shape),
        Some(ClientPayloadShape::Proto(_))
    ));
    assert!(
        proto_payload
            .payload
            .as_ref()
            .is_some_and(|payload| payload.optional)
    );
    assert!(
        proto_payload
            .payload
            .as_ref()
            .is_some_and(|payload| !payload.encrypted)
    );

    let encrypted_method: ImplItemFn = parse_quote! {
        async fn maybe_secret(
            payload: nats_micro::Payload<Option<nats_micro::Encrypted<String>>>,
        ) -> Result<(), nats_micro::NatsErrorResponse> {
            let _ = payload;
            Ok(())
        }
    };
    let encrypted_payload = endpoint_spec_for(&encrypted_method, "maybe-secret", None);
    assert!(matches!(
        encrypted_payload
            .payload
            .as_ref()
            .map(|payload| &payload.shape),
        Some(ClientPayloadShape::Raw(RawValueKind::String))
    ));
    assert!(
        encrypted_payload
            .payload
            .as_ref()
            .is_some_and(|payload| payload.optional)
    );
    assert!(
        encrypted_payload
            .payload
            .as_ref()
            .is_some_and(|payload| payload.encrypted)
    );
}

#[test]
fn optional_payloads_reject_nested_options_after_markers() {
    let method: ImplItemFn = parse_quote! {
        async fn bad_payload(
            payload: nats_micro::Payload<Option<nats_micro::Json<Option<String>>>>,
        ) -> Result<(), nats_micro::NatsErrorResponse> {
            let _ = payload;
            Ok(())
        }
    };

    let error =
        build_endpoint_client_spec(&method, &endpoint_args("bad-payload", None), Vec::new())
            .unwrap_err()
            .to_string();

    assert!(error.contains("nested `Option` payloads"));
}

#[test]
fn optional_responses_preserve_marker_metadata_for_clients() {
    let json_method: ImplItemFn = parse_quote! {
        async fn maybe_json() -> Result<Option<nats_micro::Json<JsonResponse>>, nats_micro::NatsErrorResponse> {
            Ok(None)
        }
    };
    let json_response = endpoint_spec_for(&json_method, "maybe-json", None);
    assert!(matches!(
        &json_response.response.shape,
        ClientResponseShape::Json(_)
    ));
    assert!(json_response.response.optional);
    assert!(!json_response.response.encrypted);

    let proto_method: ImplItemFn = parse_quote! {
        async fn maybe_proto() -> Result<Option<nats_micro::Proto<ProtoResponse>>, nats_micro::NatsErrorResponse> {
            Ok(None)
        }
    };
    let proto_response = endpoint_spec_for(&proto_method, "maybe-proto", None);
    assert!(matches!(
        &proto_response.response.shape,
        ClientResponseShape::Proto(_)
    ));
    assert!(proto_response.response.optional);
    assert!(!proto_response.response.encrypted);

    let encrypted_method: ImplItemFn = parse_quote! {
        async fn maybe_secret() -> Result<Option<nats_micro::Encrypted<String>>, nats_micro::NatsErrorResponse> {
            Ok(None)
        }
    };
    let encrypted_response = endpoint_spec_for(&encrypted_method, "maybe-secret", None);
    assert!(matches!(
        &encrypted_response.response.shape,
        ClientResponseShape::Raw(RawValueKind::String)
    ));
    assert!(encrypted_response.response.optional);
    assert!(encrypted_response.response.encrypted);
}

#[test]
fn optional_responses_reject_nested_options_after_markers() {
    let method: ImplItemFn = parse_quote! {
        async fn bad_response() -> Result<Option<nats_micro::Json<Option<String>>>, nats_micro::NatsErrorResponse> {
            Ok(None)
        }
    };

    let error =
        build_endpoint_client_spec(&method, &endpoint_args("bad-response", None), Vec::new())
            .unwrap_err()
            .to_string();

    assert!(error.contains("optional responses do not support nested `Option` types"));
}

#[test]
fn generated_client_branches_optional_encrypted_payloads() {
    let struct_ident = parse_quote!(DemoService);
    let method: ImplItemFn = parse_quote! {
        async fn maybe_secret(
            payload: nats_micro::Payload<Option<nats_micro::Encrypted<String>>>,
        ) -> Result<String, nats_micro::NatsErrorResponse> {
            let _ = payload;
            Ok(String::new())
        }
    };
    let client_endpoint = endpoint_spec_for(&method, "maybe-secret", Some("demo"));
    let expanded =
        generate_client_module(&struct_ident, "DemoService", &[client_endpoint]).to_string();

    assert!(expanded.contains("payload : Option < & str >"));
    assert!(expanded.contains("if let Some (payload) = payload"));
    assert!(expanded.contains("into_encrypted_request"));
    assert!(expanded.contains("into_request"));
    assert!(expanded.contains("Bytes :: new ()"));
}

#[test]
fn generated_client_decodes_optional_responses() {
    let struct_ident = parse_quote!(DemoService);
    let client_data = vec![
        endpoint_spec_for(
            &parse_quote! {
                async fn maybe_json() -> Result<Option<nats_micro::Json<JsonResponse>>, nats_micro::NatsErrorResponse> {
                    Ok(None)
                }
            },
            "maybe-json",
            Some("demo"),
        ),
        endpoint_spec_for(
            &parse_quote! {
                async fn maybe_proto() -> Result<Option<nats_micro::Proto<ProtoResponse>>, nats_micro::NatsErrorResponse> {
                    Ok(None)
                }
            },
            "maybe-proto",
            Some("demo"),
        ),
        endpoint_spec_for(
            &parse_quote! {
                async fn maybe_secret() -> Result<Option<nats_micro::Encrypted<String>>, nats_micro::NatsErrorResponse> {
                    Ok(None)
                }
            },
            "maybe-secret",
            Some("demo"),
        ),
    ];
    let expanded = generate_client_module(&struct_ident, "DemoService", &client_data).to_string();

    assert!(expanded.contains("Result < Option < JsonResponse >"));
    assert!(expanded.contains("deserialize_optional_response"));
    assert!(expanded.contains("deserialize_optional_proto_response"));
    assert!(expanded.contains("raw_response_to_optional_string"));
    assert!(expanded.contains("into_request_with_context"));
    assert!(expanded.contains("decrypt_client_response"));
}
