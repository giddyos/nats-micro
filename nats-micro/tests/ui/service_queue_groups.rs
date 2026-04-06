use nats_micro::{NatsErrorResponse, NatsService, service, service_handlers};

#[service(name = "svc", version = "1.0.0")]
struct QueueGroupService;

#[service_handlers]
impl QueueGroupService {
    #[endpoint(subject = "jobs", group = "work", queue_group = "custom-workers")]
    async fn jobs() -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {
    let def = QueueGroupService::definition();
    let endpoint = QueueGroupService::jobs_endpoint();

    assert_eq!(endpoint.queue_group.as_deref(), Some("custom-workers"));

    let info = def
        .endpoint_info
        .iter()
        .find(|info| info.fn_name == "jobs")
        .expect("jobs endpoint info should exist");
    assert_eq!(info.queue_group.as_deref(), Some("custom-workers"));
}