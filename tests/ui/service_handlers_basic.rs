use nats_micro::{
    Json, NatsErrorResponse, NatsService, RequestId, SubjectParam,
    service, service_handlers,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Greeting {
    message: String,
}

#[service(name = "greeter", version = "1.0.0", description = "A test service")]
struct GreeterService;

#[service_handlers]
impl GreeterService {
    #[endpoint(subject = "status")]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[endpoint(subject = "greet.{name}", group = "hello")]
    async fn greet(
        name: SubjectParam<String>,
    ) -> Result<Json<Greeting>, NatsErrorResponse> {
        Ok(Json(Greeting {
            message: format!("hello {}", &*name),
        }))
    }

    #[endpoint(subject = "health", group = "system")]
    async fn health(request_id: RequestId) -> Result<String, NatsErrorResponse> {
        Ok(format!("ok:{}", &*request_id))
    }

    fn helper() -> &'static str {
        "not an endpoint"
    }
}

fn main() {
    let def = GreeterService::definition();
    assert_eq!(def.metadata.name, "greeter");
    assert_eq!(def.metadata.version, "1.0.0");
    assert_eq!(def.endpoints.len(), 3);
    assert_eq!(def.endpoint_info.len(), 3);
    assert_eq!(def.consumers.len(), 0);

    let status_endpoint = GreeterService::status_endpoint();
    assert_eq!(status_endpoint.group, "");
    assert_eq!(status_endpoint.full_subject(), "greeter.status");

    let greet_info = def
        .endpoint_info
        .iter()
        .find(|info| info.fn_name == "greet")
        .expect("greet endpoint info should exist");
    assert_eq!(greet_info.fn_name, "greet");
    assert_eq!(greet_info.subject_template, "greet.{name}");
    assert_eq!(greet_info.subject_pattern, "greet.*");
    assert!(greet_info.params[0].is_subject_param);

    let greet_endpoint = GreeterService::greet_endpoint();
    assert_eq!(greet_endpoint.full_subject(), "greeter.hello.greet.*");

    let _ = GreeterService::status_endpoint();
    let _ = GreeterService::greet_endpoint();
    let _ = GreeterService::health_endpoint();
    let _ = GreeterService::helper();
}
