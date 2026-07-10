#![allow(clippy::unused_async)]

use nm::{
    IntoNatsError, NatsErrorResponse, NatsService, Payload, SubjectParam, service, service_error,
    service_handlers,
};

#[derive(Debug, Clone, nm::serde::Serialize, nm::serde::Deserialize)]
#[serde(crate = "nm::serde")]
pub struct RenameRequest {
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenameProto {
    pub value: String,
}

impl nm::prost::Message for RenameProto {
    fn encode_raw(&self, buf: &mut impl nm::prost::bytes::BufMut) {
        nm::prost::encoding::string::encode(1, &self.value, buf);
    }

    fn merge_field(
        &mut self,
        tag: u32,
        wire_type: nm::prost::encoding::WireType,
        buf: &mut impl nm::prost::bytes::Buf,
        ctx: nm::prost::encoding::DecodeContext,
    ) -> Result<(), nm::prost::DecodeError> {
        match tag {
            1 => nm::prost::encoding::string::merge(wire_type, &mut self.value, buf, ctx),
            _ => nm::prost::encoding::skip_field(wire_type, tag, buf, ctx),
        }
    }

    fn encoded_len(&self) -> usize {
        nm::prost::encoding::string::encoded_len(1, &self.value)
    }

    fn clear(&mut self) {
        self.value.clear();
    }
}

#[service_error]
pub enum RenameError {
    #[code(400)]
    #[error("bad rename")]
    BadRename,
}

#[service(name = "renamed-service", version = "1.0.0")]
pub struct RenamedService;

#[service_handlers]
impl RenamedService {
    #[endpoint(subject = "json", group = "rename")]
    async fn json(
        payload: Payload<nm::Json<RenameRequest>>,
    ) -> Result<nm::Json<RenameRequest>, RenameError> {
        Ok(payload.into_wrapped())
    }

    #[endpoint(subject = "proto", group = "rename")]
    async fn proto(
        payload: Payload<nm::Proto<RenameProto>>,
    ) -> Result<nm::Proto<RenameProto>, NatsErrorResponse> {
        Ok(payload.into_wrapped())
    }

    #[endpoint(subject = "users.{user_id}", group = "rename")]
    async fn user(user_id: SubjectParam<String>) -> Result<String, NatsErrorResponse> {
        Ok(user_id.into_inner())
    }

    #[consumer(stream = "RENAMED", durable = "renamed-durable")]
    async fn events(_payload: Payload<String>) -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

fn uses_renamed_client(client: nm::NatsClient) {
    let generated = renamed_service_client::RenamedServiceClient::new(client, None);
    let request = RenameRequest {
        value: "value".to_string(),
    };
    let user_id = "user-1".to_string();
    let _ = async move {
        let _json: Result<RenameRequest, nm::ClientError<RenameError>> =
            generated.json(&request).await;
        let _user: Result<String, nm::ClientError<NatsErrorResponse>> =
            generated.user(&user_id).await;
    };
}

#[test]
fn renamed_dependency_surface_works() {
    let response = RenameError::BadRename.into_nats_error("req-1".to_string());
    let _: nm::NatsErrorResponse = response;
    assert_eq!(
        RenamedService::definition().metadata.subject_prefix,
        Some("renamed-service".to_string())
    );
    assert_eq!(
        RenamedService::endpoints().user.full_subject_template(),
        "renamed-service.v1.rename.users.{user_id}"
    );
    assert!(RenamedService::contract_json().unwrap().contains("events"));
    let _ = uses_renamed_client;
}

fn main() {}
