use nats_micro::prelude::*;

pub struct ApplicationState {
    greeting: String,
}

#[message]
pub struct Greet<'a> {
    pub name: &'a str,
}

#[message]
pub struct Greeting {
    pub message: String,
}

#[service(
    name = "greeter",
    version = "1.0.0",
    state = ApplicationState,
    defaults(concurrency = 32, queue = "greeter-v1")
)]
impl GreeterService {
    #[request("greet.{language}")]
    async fn greet(
        state: &ApplicationState,
        language: &str,
        #[header("x-tenant-id")] tenant: Option<&str>,
        input: Greet<'_>,
    ) -> Greeting {
        Greeting {
            message: format!(
                "{} {}, language={language}, tenant={}",
                state.greeting,
                input.name,
                tenant.unwrap_or("public")
            ),
        }
    }
}

#[nats_micro::main]
async fn main() -> nats_micro::anyhow::Result<()> {
    App::new(ApplicationState {
        greeting: "Hello".to_owned(),
    })
    .profile(Profile::Production)
    .serve(GreeterService)
    .run()
    .await
}
