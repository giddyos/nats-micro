use nats_micro::{NatsErrorResponse, NatsService, service, service_handlers};

#[service(name = "svc", version = "1.0.0")]
struct ConcurrencyService;

#[service_handlers]
impl ConcurrencyService {
    #[endpoint(subject = "status", concurrency_limit = 17)]
    async fn status() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[endpoint(subject = "health")]
    async fn health() -> Result<&'static str, NatsErrorResponse> {
        Ok("ok")
    }

    #[consumer(stream = "EVENTS", durable = "jobs", concurrency_limit = 23)]
    async fn jobs() -> Result<(), NatsErrorResponse> {
        Ok(())
    }

    #[consumer(stream = "EVENTS", durable = "jobs-default")]
    async fn jobs_default() -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {
    let def = ConcurrencyService::definition();

    let status_endpoint = ConcurrencyService::status_endpoint();
    assert_eq!(status_endpoint.concurrency_limit, Some(17));

    let health_endpoint = ConcurrencyService::health_endpoint();
    assert_eq!(health_endpoint.concurrency_limit, None);

    let jobs_consumer = ConcurrencyService::jobs_consumer();
    assert_eq!(jobs_consumer.concurrency_limit, Some(23));

    let jobs_default_consumer = ConcurrencyService::jobs_default_consumer();
    assert_eq!(jobs_default_consumer.concurrency_limit, None);

    let status_info = def
        .endpoint_info
        .iter()
        .find(|info| info.fn_name == "status")
        .expect("status endpoint info should exist");
    assert_eq!(status_info.concurrency_limit, Some(17));

    let jobs_info = def
        .consumer_info
        .iter()
        .find(|info| info.fn_name == "jobs")
        .expect("jobs consumer info should exist");
    assert_eq!(jobs_info.concurrency_limit, Some(23));
}