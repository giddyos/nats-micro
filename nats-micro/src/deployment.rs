use std::collections::BTreeSet;

use serde::Serialize;

use crate::{ContractDocument, OperationKind};

/// Non-hot-path settings used to generate deployment recommendations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentOptions {
    pub drain_timeout_seconds: u64,
    pub readiness_initial_delay_seconds: u32,
    pub readiness_period_seconds: u32,
}

impl Default for DeploymentOptions {
    fn default() -> Self {
        Self {
            drain_timeout_seconds: 30,
            readiness_initial_delay_seconds: 2,
            readiness_period_seconds: 5,
        }
    }
}

/// Complete deployment metadata derived from a service contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeploymentMetadata {
    pub service_name: String,
    pub service_version: String,
    pub queue_groups: Vec<String>,
    pub endpoints: Vec<EndpointDeployment>,
    pub consumers: Vec<ConsumerDeployment>,
    pub drain_timeout_seconds: u64,
    pub readiness: ReadinessConfiguration,
    pub telemetry_environment: Vec<EnvironmentVariable>,
    pub credential_secrets: Vec<SecretPlaceholder>,
    pub termination_grace_period_seconds: u64,
}

/// Deployment settings for a core NATS endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndpointDeployment {
    pub name: String,
    pub subject: String,
    pub queue_group: Option<String>,
    pub concurrency: usize,
}

/// Deployment settings for a `JetStream` consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsumerDeployment {
    pub name: String,
    pub stream: String,
    pub durable: String,
    pub filter_subject: String,
    pub concurrency: usize,
}

/// Readiness probe recommendation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadinessConfiguration {
    pub path: String,
    pub port_name: String,
    pub initial_delay_seconds: u32,
    pub period_seconds: u32,
}

/// An environment variable surfaced in deployment output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}

/// A secret-backed environment variable surfaced in deployment output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretPlaceholder {
    pub environment_variable: String,
    pub secret_name: String,
    pub secret_key: String,
}

/// Supported deployment document formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentFormat {
    Kubernetes,
    HelmValues,
}

impl DeploymentMetadata {
    /// Derives deployment metadata from an explicitly materialized contract.
    #[must_use]
    pub fn from_contract(contract: &ContractDocument, options: DeploymentOptions) -> Self {
        let service = &contract.service;
        let queue_groups: BTreeSet<_> = service
            .operations
            .iter()
            .filter_map(|operation| operation.queue_group.clone())
            .collect();
        let endpoints = service
            .operations
            .iter()
            .filter(|operation| operation.kind != OperationKind::Consumer)
            .map(|operation| EndpointDeployment {
                name: operation.rust_name.clone(),
                subject: operation.subject.clone(),
                queue_group: operation.queue_group.clone(),
                concurrency: operation.concurrency,
            })
            .collect();
        let consumers = service
            .consumers
            .iter()
            .map(|consumer| ConsumerDeployment {
                name: consumer.rust_name.clone(),
                stream: consumer.stream.clone(),
                durable: consumer.durable.clone(),
                filter_subject: consumer.filter_subject.clone(),
                concurrency: consumer.concurrency,
            })
            .collect();

        Self {
            service_name: service.name.clone(),
            service_version: service.version.clone(),
            queue_groups: queue_groups.into_iter().collect(),
            endpoints,
            consumers,
            drain_timeout_seconds: options.drain_timeout_seconds,
            readiness: ReadinessConfiguration {
                path: "/ready".to_owned(),
                port_name: "health".to_owned(),
                initial_delay_seconds: options.readiness_initial_delay_seconds,
                period_seconds: options.readiness_period_seconds,
            },
            telemetry_environment: vec![
                EnvironmentVariable {
                    name: "NATS_MICRO_TELEMETRY".to_owned(),
                    value: "true".to_owned(),
                },
                EnvironmentVariable {
                    name: "OTEL_SERVICE_NAME".to_owned(),
                    value: service.name.clone(),
                },
                EnvironmentVariable {
                    name: "OTEL_EXPORTER_OTLP_ENDPOINT".to_owned(),
                    value: "http://otel-collector:4317".to_owned(),
                },
            ],
            credential_secrets: vec![
                SecretPlaceholder {
                    environment_variable: "NATS_URL".to_owned(),
                    secret_name: format!("{}-nats", service.name),
                    secret_key: "url".to_owned(),
                },
                SecretPlaceholder {
                    environment_variable: "NATS_CREDS".to_owned(),
                    secret_name: format!("{}-nats", service.name),
                    secret_key: "creds".to_owned(),
                },
            ],
            termination_grace_period_seconds: options.drain_timeout_seconds.saturating_add(10),
        }
    }

    /// Renders the selected deployment format.
    #[must_use]
    pub fn render(&self, format: DeploymentFormat) -> String {
        match format {
            DeploymentFormat::Kubernetes => self.render_kubernetes(),
            DeploymentFormat::HelmValues => self.render_helm_values(),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn render_kubernetes(&self) -> String {
        let mut output = String::new();
        push_line(&mut output, "apiVersion: v1");
        push_line(&mut output, "kind: ConfigMap");
        push_line(
            &mut output,
            &format!("metadata:\n  name: {}-nats-micro", self.service_name),
        );
        push_line(&mut output, "data:");
        push_line(
            &mut output,
            &format!("  service.name: {}", yaml_string(&self.service_name)),
        );
        push_line(
            &mut output,
            &format!("  service.version: {}", yaml_string(&self.service_version)),
        );
        push_line(
            &mut output,
            &format!(
                "  queue.groups: {}",
                yaml_string(&self.queue_groups.join(","))
            ),
        );
        for endpoint in &self.endpoints {
            push_line(
                &mut output,
                &format!(
                    "  endpoint.{}.subject: {}",
                    endpoint.name,
                    yaml_string(&endpoint.subject)
                ),
            );
            push_line(
                &mut output,
                &format!(
                    "  endpoint.{}.concurrency: {}",
                    endpoint.name, endpoint.concurrency
                ),
            );
        }
        for consumer in &self.consumers {
            push_line(
                &mut output,
                &format!(
                    "  consumer.{}.stream: {}",
                    consumer.name,
                    yaml_string(&consumer.stream)
                ),
            );
            push_line(
                &mut output,
                &format!(
                    "  consumer.{}.durable: {}",
                    consumer.name,
                    yaml_string(&consumer.durable)
                ),
            );
            push_line(
                &mut output,
                &format!(
                    "  consumer.{}.concurrency: {}",
                    consumer.name, consumer.concurrency
                ),
            );
        }
        push_line(&mut output, "---");
        push_line(&mut output, "apiVersion: apps/v1");
        push_line(&mut output, "kind: Deployment");
        push_line(
            &mut output,
            &format!("metadata:\n  name: {}", self.service_name),
        );
        push_line(&mut output, "spec:");
        push_line(&mut output, "  template:");
        push_line(&mut output, "    spec:");
        push_line(
            &mut output,
            &format!(
                "      terminationGracePeriodSeconds: {}",
                self.termination_grace_period_seconds
            ),
        );
        push_line(&mut output, "      containers:");
        push_line(
            &mut output,
            &format!("        - name: {}", self.service_name),
        );
        push_line(
            &mut output,
            &format!(
                "          image: {}:{}",
                self.service_name, self.service_version
            ),
        );
        push_line(&mut output, "          env:");
        for variable in &self.telemetry_environment {
            push_line(
                &mut output,
                &format!(
                    "            - name: {}\n              value: {}",
                    variable.name,
                    yaml_string(&variable.value)
                ),
            );
        }
        for secret in &self.credential_secrets {
            push_line(
                &mut output,
                &format!(
                    "            - name: {}\n              valueFrom:\n                secretKeyRef:\n                  name: {}\n                  key: {}",
                    secret.environment_variable, secret.secret_name, secret.secret_key
                ),
            );
        }
        push_line(&mut output, "          readinessProbe:");
        push_line(
            &mut output,
            &format!(
                "            httpGet:\n              path: {}\n              port: {}",
                self.readiness.path, self.readiness.port_name
            ),
        );
        push_line(
            &mut output,
            &format!(
                "            initialDelaySeconds: {}\n            periodSeconds: {}",
                self.readiness.initial_delay_seconds, self.readiness.period_seconds
            ),
        );
        push_line(&mut output, "          lifecycle:");
        push_line(
            &mut output,
            &format!(
                "            preStop:\n              exec:\n                command: [\"nats-micro-drain\", \"--timeout\", \"{}s\"]",
                self.drain_timeout_seconds
            ),
        );
        output
    }

    fn render_helm_values(&self) -> String {
        let mut output = String::new();
        push_line(
            &mut output,
            &format!("service:\n  name: {}", yaml_string(&self.service_name)),
        );
        push_line(
            &mut output,
            &format!("  version: {}", yaml_string(&self.service_version)),
        );
        push_line(
            &mut output,
            &format!(
                "terminationGracePeriodSeconds: {}",
                self.termination_grace_period_seconds
            ),
        );
        push_line(
            &mut output,
            &format!("drainTimeoutSeconds: {}", self.drain_timeout_seconds),
        );
        push_line(&mut output, "queueGroups:");
        for queue in &self.queue_groups {
            push_line(&mut output, &format!("  - {}", yaml_string(queue)));
        }
        push_line(&mut output, "endpoints:");
        for endpoint in &self.endpoints {
            push_line(
                &mut output,
                &format!(
                    "  - name: {}\n    subject: {}\n    queueGroup: {}\n    concurrency: {}",
                    yaml_string(&endpoint.name),
                    yaml_string(&endpoint.subject),
                    endpoint
                        .queue_group
                        .as_deref()
                        .map_or_else(|| "null".to_owned(), yaml_string),
                    endpoint.concurrency
                ),
            );
        }
        push_line(&mut output, "consumers:");
        for consumer in &self.consumers {
            push_line(
                &mut output,
                &format!(
                    "  - name: {}\n    stream: {}\n    durable: {}\n    filterSubject: {}\n    concurrency: {}",
                    yaml_string(&consumer.name),
                    yaml_string(&consumer.stream),
                    yaml_string(&consumer.durable),
                    yaml_string(&consumer.filter_subject),
                    consumer.concurrency
                ),
            );
        }
        push_line(
            &mut output,
            &format!(
                "readiness:\n  path: {}\n  port: {}\n  initialDelaySeconds: {}\n  periodSeconds: {}",
                yaml_string(&self.readiness.path),
                yaml_string(&self.readiness.port_name),
                self.readiness.initial_delay_seconds,
                self.readiness.period_seconds
            ),
        );
        push_line(&mut output, "telemetryEnv:");
        for variable in &self.telemetry_environment {
            push_line(
                &mut output,
                &format!("  {}: {}", variable.name, yaml_string(&variable.value)),
            );
        }
        push_line(&mut output, "credentialSecrets:");
        for secret in &self.credential_secrets {
            push_line(
                &mut output,
                &format!(
                    "  {}:\n    secretName: {}\n    secretKey: {}",
                    secret.environment_variable,
                    yaml_string(&secret.secret_name),
                    yaml_string(&secret.secret_key)
                ),
            );
        }
        output
    }
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

fn push_line(output: &mut String, value: &str) {
    output.push_str(value);
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::{DeploymentFormat, DeploymentMetadata, DeploymentOptions};
    use crate::ContractDocument;

    #[test]
    fn output_contains_operational_metadata() {
        let contract: ContractDocument =
            serde_json::from_str(include_str!("../tests/fixtures/v6-contract.json"))
                .expect("fixture contract");
        let deployment = DeploymentMetadata::from_contract(&contract, DeploymentOptions::default());
        let kubernetes = deployment.render(DeploymentFormat::Kubernetes);
        let helm = deployment.render(DeploymentFormat::HelmValues);

        for required in [
            "service.name",
            "queue.groups",
            "endpoint.get.concurrency",
            "consumer.project.stream",
            "terminationGracePeriodSeconds",
            "readinessProbe",
            "OTEL_SERVICE_NAME",
            "NATS_CREDS",
        ] {
            assert!(kubernetes.contains(required), "missing `{required}`");
        }
        for required in [
            "queueGroups",
            "endpoints",
            "consumers",
            "drainTimeoutSeconds",
            "readiness",
            "telemetryEnv",
            "credentialSecrets",
        ] {
            assert!(helm.contains(required), "missing `{required}`");
        }
    }
}
