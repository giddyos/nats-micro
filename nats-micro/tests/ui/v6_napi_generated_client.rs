use nats_micro::{Body, NatsErrorResponse, service};

#[nats_micro::object]
#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct CreateInput {
    pub email: String,
}

#[nats_micro::object]
#[derive(Debug, Clone, nats_micro::serde::Serialize, nats_micro::serde::Deserialize)]
#[serde(crate = "nats_micro::serde")]
pub struct Created {
    pub id: String,
}

#[service(name = "napi-users", version = "2.0.0", napi)]
impl NapiUsers {
    #[request("users.{user_id}")]
    async fn create(
        user_id: &str,
        input: CreateInput,
    ) -> Result<Created, NatsErrorResponse> {
        Ok(Created {
            id: format!("{user_id}:{}", input.email),
        })
    }

    #[publish("events")]
    async fn events(body: Body<'_>) {
        let _ = body;
    }
}

fn assert_generated(client: &JsNapiUsersClient) {
    let _ = JsNapiUsersClient::connect;
    let _ = client.create("user-1".to_owned(), CreateInput {
        email: "ada@example.com".to_owned(),
    });
    let _ = client.events(nats_micro::napi::bindgen_prelude::Buffer::from(vec![1, 2, 3]));
}

fn main() {}
