use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nats_micro::{ContractDocument, DeploymentFormat, DeploymentMetadata, DeploymentOptions};

const USAGE: &str = "\
Usage:
  nats-micro contract --format json [--input CONTRACT]
  nats-micro validate [--input CONTRACT]
  nats-micro subjects [--input CONTRACT]
  nats-micro deployment --format kubernetes|helm-values [--input CONTRACT]

Contract input defaults to $NATS_MICRO_CONTRACT or ./nats-micro.contract.json.";

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("nats-micro: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut arguments: Vec<String>) -> Result<String, String> {
    let input = take_option(&mut arguments, "--input")?
        .map(PathBuf::from)
        .or_else(|| env::var_os("NATS_MICRO_CONTRACT").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("nats-micro.contract.json"));
    let format = take_option(&mut arguments, "--format")?;
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err("missing command".to_owned());
    };
    if arguments.len() != 1 {
        return Err(format!("unexpected argument `{}`", arguments[1]));
    }

    let contract = read_contract(&input)?;
    match command {
        "contract" => {
            require_format(format.as_deref(), "json")?;
            serde_json::to_string_pretty(&contract)
                .map_err(|error| format!("could not serialize contract: {error}"))
        }
        "validate" => {
            reject_format(format)?;
            contract
                .validate()
                .map_err(|error| format!("invalid contract: {error}"))?;
            Ok(format!(
                "valid: {} {}",
                contract.service.name, contract.service.version
            ))
        }
        "subjects" => {
            reject_format(format)?;
            contract
                .validate()
                .map_err(|error| format!("invalid contract: {error}"))?;
            Ok(contract.subjects().collect::<Vec<_>>().join("\n"))
        }
        "deployment" => {
            contract
                .validate()
                .map_err(|error| format!("invalid contract: {error}"))?;
            let format = match format.as_deref() {
                Some("kubernetes") => DeploymentFormat::Kubernetes,
                Some("helm-values") => DeploymentFormat::HelmValues,
                Some(other) => return Err(format!("unsupported deployment format `{other}`")),
                None => return Err("deployment requires `--format`".to_owned()),
            };
            Ok(
                DeploymentMetadata::from_contract(&contract, DeploymentOptions::default())
                    .render(format),
            )
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn read_contract(path: &Path) -> Result<ContractDocument, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read `{}`: {error}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("could not parse `{}`: {error}", path.display()))
}

fn take_option(arguments: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    let Some(index) = arguments.iter().position(|argument| argument == name) else {
        return Ok(None);
    };
    if index + 1 >= arguments.len() {
        return Err(format!("`{name}` requires a value"));
    }
    let value = arguments.remove(index + 1);
    arguments.remove(index);
    Ok(Some(value))
}

fn require_format(actual: Option<&str>, expected: &str) -> Result<(), String> {
    match actual {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(format!("unsupported format `{actual}`")),
        None => Err("contract requires `--format json`".to_owned()),
    }
}

fn reject_format(format: Option<String>) -> Result<(), String> {
    if let Some(format) = format {
        Err(format!("command does not accept format `{format}`"))
    } else {
        Ok(())
    }
}
