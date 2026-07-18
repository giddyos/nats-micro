use nats_micro::{
    ConsumerAction, ContractDocument, DeploymentFormat, DeploymentMetadata, DeploymentOptions,
    ServiceContract, message, service,
};

#[message]
pub struct ContractOutput {
    pub value: String,
}

#[service(
    name = "v6-contract",
    version = "2.4.6",
    description = "contract fixture",
    defaults(queue = "contract-workers", concurrency = 12)
)]
impl ContractService {
    #[request("lookup.{id}")]
    async fn lookup(id: &str) -> ContractOutput {
        ContractOutput {
            value: id.to_owned(),
        }
    }

    #[consumer(
        stream = "CONTRACTS",
        durable = "contract-projector",
        filter = "contracts.events",
        concurrency = 3
    )]
    async fn project() -> ConsumerAction {
        ConsumerAction::Ack
    }
}

#[test]
fn generated_contract_access_is_static_until_serialization() {
    const CONTRACT: ServiceContract<'static> = ContractService::contract();
    assert!(std::ptr::eq(CONTRACT.service, &ContractService::SPEC));
    assert!(std::ptr::eq(
        ContractService::operations().as_ptr(),
        ContractService::SPEC.operations.as_ptr()
    ));
    assert_eq!(CONTRACT.operations()[0].service_name, "v6-contract");
    assert_eq!(CONTRACT.operations()[0].service_version, "2.4.6");

    let json = ContractService::contract_json().expect("serialize static contract");
    let document: ContractDocument =
        serde_json::from_str(&json).expect("deserialize owned contract");
    document.validate().expect("valid generated contract");
    assert_eq!(
        document.service.operations[0].queue_group.as_deref(),
        Some("contract-workers")
    );
    assert_eq!(document.service.consumers[0].stream, "CONTRACTS");
}

#[test]
fn deployment_output_contains_runtime_recommendations() {
    let document = ContractDocument::from(ContractService::contract());
    let deployment = DeploymentMetadata::from_contract(&document, DeploymentOptions::default());

    assert_eq!(deployment.service_name, "v6-contract");
    assert_eq!(deployment.service_version, "2.4.6");
    assert_eq!(deployment.queue_groups, ["contract-workers"]);
    assert_eq!(deployment.endpoints[0].concurrency, 12);
    assert_eq!(deployment.consumers[0].concurrency, 3);
    assert_eq!(deployment.consumers[0].durable, "contract-projector");
    assert_eq!(deployment.termination_grace_period_seconds, 40);

    let kubernetes = deployment.render(DeploymentFormat::Kubernetes);
    let helm = deployment.render(DeploymentFormat::HelmValues);
    assert!(kubernetes.contains("readinessProbe"));
    assert!(kubernetes.contains("NATS_CREDS"));
    assert!(helm.contains("telemetryEnv"));
    assert!(helm.contains("drainTimeoutSeconds"));
}
