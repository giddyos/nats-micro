use super::{
    EndpointArgs, PayloadEncoding, ResponseEncoding, classify_return_type, extract_client_meta,
    parse_subject_template, process_endpoint_method, validate_template_bindings,
};
use syn::{ImplItemFn, Signature, parse_quote};

fn endpoint_attr(method: &ImplItemFn) -> &syn::Attribute {
    method
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("endpoint"))
        .expect("endpoint attribute")
}

fn signature(value: Signature) -> Signature {
    value
}

#[test]
fn parse_subject_template_rejects_invalid_segments() {
    let ident = syn::Ident::new("demo", proc_macro2::Span::call_site());
    let error = parse_subject_template("users..profile", &ident)
        .unwrap_err()
        .to_string();

    assert!(error.contains("contains an empty segment"));
}

#[test]
fn validate_template_bindings_accepts_leading_underscore_subject_params() {
    let sig = signature(parse_quote! {
        async fn show(_user_id: nats_micro::SubjectParam<String>) -> Result<(), nats_micro::NatsErrorResponse>
    });
    let (_, template_params) = parse_subject_template("users.{user_id}", &sig.ident).unwrap();

    validate_template_bindings(&sig, &template_params).unwrap();
}

#[test]
fn validate_template_bindings_rejects_missing_subject_params() {
    let sig = signature(parse_quote! {
        async fn show() -> Result<(), nats_micro::NatsErrorResponse>
    });
    let (_, template_params) = parse_subject_template("users.{user_id}", &sig.ident).unwrap();
    let error = validate_template_bindings(&sig, &template_params)
        .unwrap_err()
        .to_string();

    assert!(error.contains("requires handler argument `user_id: SubjectParam<...>`"));
}

#[test]
fn client_meta_tracks_optional_payload_and_response_markers() {
    let sig = signature(parse_quote! {
        async fn maybe_secret(
            user_id: nats_micro::SubjectParam<String>,
            payload: nats_micro::Payload<Option<nats_micro::Encrypted<String>>>,
        ) -> Result<Option<nats_micro::Encrypted<String>>, nats_micro::NatsErrorResponse>
    });

    let meta = extract_client_meta(&sig).unwrap();
    let payload = meta.payload.expect("payload metadata");

    assert_eq!(meta.subject_params.len(), 1);
    assert_eq!(meta.subject_params[0].name, "user_id");
    assert!(payload.optional);
    assert!(payload.encrypted);
    assert_eq!(payload.encoding, PayloadEncoding::Raw);
    assert!(meta.response.optional);
    assert!(meta.response.encrypted);
    assert!(matches!(meta.response.encoding, ResponseEncoding::Raw));
}

#[test]
fn classify_return_type_tracks_plain_collection_responses() {
    let plain = signature(parse_quote! {
        async fn list() -> Result<Vec<String>, nats_micro::NatsErrorResponse>
    });
    let plain_meta = classify_return_type(&plain).unwrap();

    assert!(matches!(plain_meta.encoding, ResponseEncoding::Serde));
    assert!(!plain_meta.encrypted);
    assert!(!plain_meta.optional);

    let encrypted = signature(parse_quote! {
        async fn list() -> Result<nats_micro::Encrypted<Vec<String>>, nats_micro::NatsErrorResponse>
    });
    let encrypted_meta = classify_return_type(&encrypted).unwrap();

    assert!(matches!(encrypted_meta.encoding, ResponseEncoding::Serde));
    assert!(encrypted_meta.encrypted);
    assert!(!encrypted_meta.optional);
}

#[test]
fn classify_return_type_rejects_nested_optional_markers() {
    let sig = signature(parse_quote! {
        async fn bad() -> Result<Option<nats_micro::Json<Option<String>>>, nats_micro::NatsErrorResponse>
    });
    let error = classify_return_type(&sig).unwrap_err().to_string();

    assert!(error.contains("nested `Option` types"));
}

#[test]
fn classify_return_type_rejects_nested_encrypted_markers() {
    let sig = signature(parse_quote! {
        async fn bad() -> Result<nats_micro::Json<nats_micro::Encrypted<String>>, nats_micro::NatsErrorResponse>
    });
    let error = classify_return_type(&sig).unwrap_err().to_string();

    assert!(error.contains("outermost response marker"));
}

#[test]
fn process_endpoint_method_emits_metadata_for_service_wiring() {
    let struct_ident = parse_quote!(DemoService);
    let method: ImplItemFn = parse_quote! {
        #[cfg(feature = "demo")]
        #[endpoint(
            subject = "users.{user_id}.profile",
            group = "accounts",
            queue_group = "workers",
            concurrency_limit = 8
        )]
        async fn get_profile(
            user_id: nats_micro::SubjectParam<String>,
            auth: nats_micro::Auth,
            payload: nats_micro::Payload<nats_micro::Json<GetProfileRequest>>,
        ) -> Result<nats_micro::Json<UserProfile>, nats_micro::NatsErrorResponse> {
            let _ = user_id;
            let _ = auth;
            let _ = payload;
            Ok(nats_micro::Json(UserProfile))
        }
    };

    let (generated, client_endpoint) =
        process_endpoint_method(&struct_ident, &method, endpoint_attr(&method)).unwrap();
    let def_tokens = generated.def_fn.to_string();
    let info_tokens = generated.info_expr.to_string();

    assert_eq!(generated.attrs.len(), 1);
    assert_eq!(client_endpoint.group, "accounts");
    assert_eq!(client_endpoint.subject.template, "users.{user_id}.profile");
    assert_eq!(client_endpoint.subject.pattern, "users.*.profile");
    assert_eq!(client_endpoint.subject.params[0].name, "user_id");
    assert!(def_tokens.contains("queue_group : Some (\"workers\" . to_string ())"));
    assert!(def_tokens.contains("auth_required : true"));
    assert!(def_tokens.contains("concurrency_limit : Some (8"));
    assert!(info_tokens.contains("PayloadEncoding :: Json"));
    assert!(info_tokens.contains("ResponseEncoding :: Json"));
    assert!(info_tokens.contains("is_subject_param : true"));
}

#[test]
fn endpoint_args_capture_optional_queue_group_and_limit() {
    let args = EndpointArgs {
        subject: "status".to_string(),
        group: Some("demo".to_string()),
        queue_group: Some("workers".to_string()),
        concurrency_limit: Some(3),
    };

    assert_eq!(args.group.as_deref(), Some("demo"));
    assert_eq!(args.queue_group.as_deref(), Some("workers"));
    assert_eq!(args.concurrency_limit, Some(3));
}
