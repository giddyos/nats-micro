use nats_micro::{
    Json, NatsErrorResponse, NatsService, RequestId,
    service, service_handlers,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Pong {
    message: String,
}

#[service(name = "svc_a", version = "0.1.0")]
struct ServiceA;

#[service_handlers]
impl ServiceA {
    #[endpoint(subject = "ping", group = "a")]
    async fn ping(request_id: RequestId) -> Result<Json<Pong>, NatsErrorResponse> {
        Ok(Json(Pong {
            message: format!("pong:{}", &*request_id),
        }))
    }
}

#[service(name = "svc_b", version = "0.2.0", description = "Second service")]
struct ServiceB;

#[service_handlers]
impl ServiceB {
    #[endpoint(subject = "echo", group = "b")]
    async fn echo() -> Result<String, NatsErrorResponse> {
        Ok("echo".to_string())
    }

    #[consumer(
        stream = "EVENTS",
        durable = "svc-b-con",
        config = nats_micro::ConsumerConfig {
            ack_wait: std::time::Duration::from_secs(30),
            max_deliver: 5,
            ..Default::default()
        }
    )]
    async fn handle_event(payload: nats_micro::Payload<Json<serde_json::Value>>) -> Result<(), NatsErrorResponse> {
        let _ = payload;
        Ok(())
    }
}

fn main() {
    let def_a = ServiceA::definition();
    let def_b = ServiceB::definition();

    assert_eq!(def_a.metadata.name, "svc_a");
    assert_eq!(def_b.metadata.name, "svc_b");
    assert_eq!(def_a.endpoints.len(), 1);
    assert_eq!(def_b.endpoints.len(), 1);
    assert_eq!(def_b.consumers.len(), 1);

    assert_eq!(def_a.endpoints[0].full_subject(), "svc_a.a.ping");
    assert_eq!(def_b.endpoints[0].full_subject(), "svc_b.b.echo");

    let con = &def_b.consumer_info[0];
    assert_eq!(con.fn_name, "handle_event");
    assert_eq!(con.stream, "EVENTS");
    assert!(!con.auth_required);
}
