use nats_micro::{
    Auth, AuthError, FromAuthRequest, NatsErrorResponse, NatsService, RequestContext, service,
    service_handlers,
};

#[service(name = "multi-auth", version = "1.0.0")]
struct MultiAuthService;

struct JwtUser;

impl FromAuthRequest for JwtUser {
    async fn from_auth_request(_ctx: &RequestContext) -> Result<Self, AuthError> {
        Err(AuthError::MissingCredentials)
    }
}

struct ApiUser;

impl FromAuthRequest for ApiUser {
    async fn from_auth_request(_ctx: &RequestContext) -> Result<Self, AuthError> {
        Err(AuthError::MissingCredentials)
    }
}

#[service_handlers]
impl MultiAuthService {
    #[endpoint(subject = "check", group = "auth")]
    async fn check_auth(
        _jwt: Auth<JwtUser>,
        _jwt_again: Auth<JwtUser>,
        _maybe_jwt: Option<Auth<JwtUser>>,
        _api: Option<Auth<ApiUser>>,
        _api_again: Option<Auth<ApiUser>>,
    ) -> Result<(), NatsErrorResponse> {
        Ok(())
    }

    #[consumer(stream = "EVENTS", durable = "multi-auth")]
    async fn handle_event(
        _jwt: Auth<JwtUser>,
        _maybe_jwt: Option<Auth<JwtUser>>,
        _api: Option<Auth<ApiUser>>,
        _api_again: Auth<ApiUser>,
    ) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn main() {
    let _ = MultiAuthService::definition();
}