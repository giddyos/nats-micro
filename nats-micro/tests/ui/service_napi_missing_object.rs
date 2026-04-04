use nats_micro::{Json, NatsErrorResponse, Payload, service, service_handlers};
#[cfg(feature = "napi")]
use nats_micro::napi;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct MissingMarkerPayload {
    value: String,
}

#[nats_micro::object]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedResponse {
    pub ok: bool,
}

#[service(name = "broken", napi = true)]
struct BrokenService;

#[service_handlers]
impl BrokenService {
    #[endpoint(subject = "create")]
    async fn create(
        payload: Payload<Json<MissingMarkerPayload>>,
    ) -> Result<Json<CreatedResponse>, NatsErrorResponse> {
        let _ = payload;
        Ok(Json(CreatedResponse { ok: true }))
    }
}

fn main() {}