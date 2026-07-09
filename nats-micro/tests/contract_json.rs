use nats_micro::{Json, NatsErrorResponse, Payload, service, service_handlers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct SumRequest {
    numbers: Vec<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SumResponse {
    total: i64,
}

#[service(
    name = "contract_svc",
    version = "1.2.3",
    description = "Contract fixture"
)]
struct ContractService;

#[service_handlers]
impl ContractService {
    #[endpoint(subject = "sum", group = "math")]
    async fn sum(
        payload: Payload<Json<SumRequest>>,
    ) -> Result<Json<SumResponse>, NatsErrorResponse> {
        Ok(Json(SumResponse {
            total: payload.numbers.iter().sum(),
        }))
    }

    #[endpoint(subject = "health")]
    async fn health() -> Result<(), NatsErrorResponse> {
        Ok(())
    }
}

#[test]
fn contract_json_uses_stable_snake_case_enum_values() {
    let json = ContractService::contract_json().expect("contract JSON should serialize");
    let expected = r#"{
  "metadata": {
    "name": "contract_svc",
    "version": "1.2.3",
    "description": "Contract fixture",
    "subject_prefix": "contract_svc"
  },
  "endpoints": [
    {
      "fn_name": "sum",
      "subject_template": "sum",
      "subject_pattern": "sum",
      "group": "math",
      "queue_group": null,
      "auth_policy": "none",
      "concurrency_limit": null,
      "params": [
        {
          "name": "payload",
          "type_name": "Payload < Json < SumRequest > >",
          "is_subject_param": false
        }
      ],
      "payload_meta": {
        "encoding": "json",
        "encrypted": false,
        "inner_type": "SumRequest"
      },
      "response_meta": {
        "encoding": "json",
        "encrypted": false,
        "inner_type": "SumResponse"
      }
    },
    {
      "fn_name": "health",
      "subject_template": "health",
      "subject_pattern": "health",
      "group": "",
      "queue_group": null,
      "auth_policy": "none",
      "concurrency_limit": null,
      "params": [],
      "payload_meta": null,
      "response_meta": {
        "encoding": "unit",
        "encrypted": false,
        "inner_type": ""
      }
    }
  ],
  "consumers": []
}"#;
    assert_eq!(json, expected);
}
