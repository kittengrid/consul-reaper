use consul_reaper::consul::{CheckStatus, Node};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TestServiceRegistration {
    pub id: Option<String>,
    pub name: String,
    pub port: u16,
    pub tags: Option<Vec<String>>,
    pub meta: Option<HashMap<String, String>>,
    pub checks: Vec<TestHealthCheck>,
}

#[derive(Debug, Clone)]
pub struct TestHealthCheck {
    pub name: String,
    pub check_id: Option<String>,
    pub status: CheckStatus,
    pub service_id: Option<String>,
    pub notes: Option<String>,
    pub output: Option<String>,
    pub definition: Option<TestCheckDefinition>,
}

#[derive(Debug, Clone, Default)]
pub struct TestCheckDefinition {
    pub interval: Option<String>,
    pub timeout: Option<String>,
    pub http: Option<String>,
    pub tcp: Option<String>,
}

#[derive(Debug, Serialize)]
struct CatalogRegistrationPayload {
    #[serde(rename = "Node")]
    node: String,

    #[serde(rename = "Address")]
    address: String,

    #[serde(rename = "Service")]
    service: ServicePayload,

    #[serde(rename = "Checks", skip_serializing_if = "Vec::is_empty")]
    checks: Vec<HealthCheckPayload>,
}

#[derive(Debug, Serialize)]
struct ServicePayload {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    id: Option<String>,

    #[serde(rename = "Service")]
    service: String,

    #[serde(rename = "Port")]
    port: u16,

    #[serde(rename = "Tags", skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,

    #[serde(rename = "Meta", skip_serializing_if = "Option::is_none")]
    meta: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct HealthCheckPayload {
    #[serde(rename = "Name")]
    name: String,

    #[serde(rename = "CheckID", skip_serializing_if = "Option::is_none")]
    check_id: Option<String>,

    #[serde(rename = "Status")]
    status: CheckStatus,

    #[serde(rename = "ServiceID", skip_serializing_if = "Option::is_none")]
    service_id: Option<String>,

    #[serde(rename = "Notes", skip_serializing_if = "Option::is_none")]
    notes: Option<String>,

    #[serde(rename = "Output", skip_serializing_if = "Option::is_none")]
    output: Option<String>,

    #[serde(rename = "Definition", skip_serializing_if = "Option::is_none")]
    definition: Option<CheckDefinitionPayload>,
}

#[derive(Debug, Serialize)]
struct CheckDefinitionPayload {
    #[serde(rename = "Interval", skip_serializing_if = "Option::is_none")]
    interval: Option<String>,

    #[serde(rename = "Timeout", skip_serializing_if = "Option::is_none")]
    timeout: Option<String>,

    #[serde(rename = "HTTP", skip_serializing_if = "Option::is_none")]
    http: Option<String>,

    #[serde(rename = "TCP", skip_serializing_if = "Option::is_none")]
    tcp: Option<String>,
}

pub async fn register_node_service_for_test(
    consul_http_addr: &str,
    node: &Node,
    service: TestServiceRegistration,
) -> Result<(), Box<dyn std::error::Error>> {
    let service_id = service.id.clone();
    let payload = CatalogRegistrationPayload {
        node: node.name.clone(),
        address: node.address().to_string(),
        service: ServicePayload {
            id: service.id,
            service: service.name,
            port: service.port,
            tags: service.tags,
            meta: service.meta,
        },
        checks: service
            .checks
            .into_iter()
            .map(|check| HealthCheckPayload {
                name: check.name,
                check_id: check.check_id,
                status: check.status,
                service_id: check.service_id.or_else(|| service_id.clone()),
                notes: check.notes,
                output: check.output,
                definition: check.definition.map(|definition| CheckDefinitionPayload {
                    interval: definition.interval,
                    timeout: definition.timeout,
                    http: definition.http,
                    tcp: definition.tcp,
                }),
            })
            .collect(),
    };

    let response = reqwest::Client::new()
        .put(format!("{}/v1/catalog/register", consul_http_addr))
        .json(&payload)
        .send()
        .await?;

    if !response.status().is_success() {
        let body = response.text().await?;
        return Err(format!("Failed to register node service: {}", body).into());
    }

    Ok(())
}
