use nats_micro::{NatsErrorResponse, NatsService, service, service_handlers};
#[cfg(feature = "encryption")]
use nats_micro::Encrypted;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CollectionItem {
    value: String,
}

#[service(name = "collection_response_svc", version = "1.0.0")]
struct CollectionResponseService;

#[service_handlers]
impl CollectionResponseService {
    #[endpoint(subject = "list")]
    async fn list() -> Result<Vec<CollectionItem>, NatsErrorResponse> {
        Ok(Vec::new())
    }

    #[cfg(feature = "encryption")]
    #[endpoint(subject = "secret-list")]
    async fn secret_list() -> Result<Encrypted<Vec<CollectionItem>>, NatsErrorResponse> {
        Ok(Encrypted(Vec::new()))
    }
}

#[cfg(feature = "client")]
fn _assert_client_module() {
    use collection_response_service_client::CollectionResponseServiceClient;

    fn _assert_list(client: &CollectionResponseServiceClient) {
        let _ = async {
            let _result: Result<Vec<CollectionItem>, nats_micro::ClientError<NatsErrorResponse>> =
                client.list().await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<Vec<CollectionItem>, nats_micro::ClientError<NatsErrorResponse>> =
                client.list_with(options).await;
        };
    }

    #[cfg(feature = "encryption")]
    fn _assert_secret_list(client: &CollectionResponseServiceClient) {
        let _ = async {
            let _result: Result<Vec<CollectionItem>, nats_micro::ClientError<NatsErrorResponse>> =
                client.secret_list().await;
            let options = nats_micro::ClientCallOptions::new();
            let _result: Result<Vec<CollectionItem>, nats_micro::ClientError<NatsErrorResponse>> =
                client.secret_list_with(options).await;
        };
    }
}

fn main() {
    let def = CollectionResponseService::definition();
    assert_eq!(def.metadata.name, "collection_response_svc");
    assert!(!def.endpoints.is_empty());
}
