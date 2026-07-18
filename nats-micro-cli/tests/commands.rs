use std::path::PathBuf;
use std::process::Command;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nats-micro/tests/fixtures/v6-contract.json")
}

fn command(arguments: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_nats-micro"))
        .args(arguments)
        .arg("--input")
        .arg(fixture())
        .output()
        .expect("run nats-micro CLI");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("UTF-8 CLI output")
}

#[test]
fn contract_and_validation_commands_work() {
    let contract = command(&["contract", "--format", "json"]);
    assert!(contract.contains("\"name\": \"users\""));
    assert_eq!(command(&["validate"]).trim(), "valid: users 2.3.4");
}

#[test]
fn subjects_and_deployment_commands_work() {
    let subjects = command(&["subjects"]);
    assert!(subjects.contains("api.v2.users.*"));
    assert!(subjects.contains("users.events.>"));

    let kubernetes = command(&["deployment", "--format", "kubernetes"]);
    assert!(kubernetes.contains("terminationGracePeriodSeconds"));
    assert!(kubernetes.contains("users-projector"));

    let helm = command(&["deployment", "--format", "helm-values"]);
    assert!(helm.contains("credentialSecrets"));
    assert!(helm.contains("queueGroups"));
}
