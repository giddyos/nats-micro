use quote::quote;

use super::expand;

#[test]
fn generated_request_dispatch_is_static_and_borrowed() {
    let expanded = expand(
        quote!(
            name = "users",
            version = "1.2.3",
            state = AppState,
            defaults(concurrency = 16, queue = "users")
        ),
        quote! {
            impl UserService {
                #[request("users.{user_id}")]
                async fn get(
                    state: &AppState,
                    user_id: &str,
                    #[header("x-tenant")] tenant: Option<&str>,
                ) -> Result<User, UserError> {
                    todo!()
                }
            }
        },
    )
    .to_string();

    assert!(expanded.contains("RequestEndpoint < AppState >"));
    assert!(expanded.contains("FromSubject"));
    assert!(expanded.contains("segment (request . subject () , 3usize)"));
    assert!(expanded.contains("ServiceSpec"));
    assert!(expanded.contains("OperationSpec"));
    assert!(expanded.contains("UserServiceClient"));
    assert!(expanded.contains("Service < AppState >"));
    assert!(expanded.contains("spawn_request"));
    assert!(expanded.contains("ClientTransport"));
    assert!(expanded.contains("RequestCall"));
    assert!(expanded.contains("ClientSubject :: Owned"));
    assert!(expanded.contains("subject_param_len"));
    #[cfg(feature = "macros_test_util_feature")]
    {
        assert!(expanded.contains("testing :: dispatch"));
        assert!(expanded.contains("subject_matches"));
        assert!(expanded.contains("testing :: LocalDispatch :: NotMatched"));
    }
    for prohibited in [
        "HandlerFn",
        "HandlerFuture",
        "StateMap",
        "ServiceDefinition",
        "inventory",
        "RequestContext",
        "Box :: pin",
        "tokio :: spawn",
    ] {
        assert!(
            !expanded.contains(prohibited),
            "generated code contains `{prohibited}`"
        );
    }
}

#[test]
fn duplicate_subjects_are_rejected_before_generation() {
    let expanded = expand(
        quote!(name = "demo", version = "1.0.0"),
        quote! {
            impl Demo {
                #[request("same")]
                async fn one() {}

                #[request("same")]
                async fn two() {}
            }
        },
    )
    .to_string();

    assert!(expanded.contains("duplicate operation subject"));
}

#[test]
fn ordinary_payloads_and_varied_returns_are_classified() {
    let expanded = expand(
        quote!(name = "demo", version = "1.0.0"),
        quote! {
            impl Demo {
                #[request]
                async fn plain(input: Borrowed<'_>) -> Output {
                    todo!()
                }

                #[request]
                async fn fallible() -> Result<Option<Output>, Error> {
                    todo!()
                }
            }
        },
    )
    .to_string();

    assert!(expanded.contains("decode_json"));
    assert!(expanded.contains("optional_none"));
    assert!(expanded.contains("IntoServiceError"));
    assert!(expanded.contains("ClientSubject :: Static"));
}
