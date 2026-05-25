use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

const DEFAULT_INTERVAL: u64 = 10;
const DEFAULT_TIMEOUT: u64 = 5;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct ServiceDefinition {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,

    #[serde(rename = "Service")]
    pub(crate) service: String,

    #[serde(rename = "Port")]
    pub(crate) port: u16,

    #[serde(rename = "Tags", skip_serializing_if = "Option::is_none")]
    pub(crate) tags: Option<Vec<String>>,
}

#[derive(Default, Debug, Deserialize, Clone)]
pub struct Node {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,

    #[serde(rename = "Node")]
    pub name: String,

    #[serde(rename = "Address")]
    pub(crate) address: String,

    #[serde(rename = "Datacenter", skip_serializing_if = "Option::is_none")]
    pub(crate) datacenter: Option<String>,

    #[serde(rename = "Meta", skip_serializing_if = "Option::is_none")]
    pub(crate) node_meta: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "TaggedAddresses", skip_serializing_if = "Option::is_none")]
    pub(crate) tagged_addresses: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "ModifyIndex")]
    pub(crate) modify_index: u64,
}

impl std::hash::Hash for Node {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Node {}

impl Node {
    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn node_meta(&self) -> Option<&HashMap<String, String>> {
        self.node_meta.as_ref()
    }
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq, Default)]
pub enum CheckStatus {
    #[serde(rename = "passing")]
    #[default]
    Passing,

    #[serde(rename = "warning")]
    Warning,

    #[serde(rename = "critical")]
    Critical,
}

impl std::fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckStatus::Passing => write!(f, "passing"),
            CheckStatus::Warning => write!(f, "warning"),
            CheckStatus::Critical => write!(f, "critical"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CheckDefinition {
    #[serde(rename = "Script", skip_serializing_if = "Option::is_none")]
    pub(crate) script: Option<String>,

    #[serde(rename = "Interval", skip_serializing_if = "Option::is_none")]
    pub(crate) interval: Option<String>,

    #[serde(rename = "Timeout", skip_serializing_if = "Option::is_none")]
    pub(crate) timeout: Option<String>,

    #[serde(rename = "HTTP", skip_serializing_if = "Option::is_none")]
    pub(crate) http: Option<String>,

    #[serde(rename = "TCP", skip_serializing_if = "Option::is_none")]
    pub(crate) tcp: Option<String>,
}

pub enum CheckType {
    Script,
    Http,
    Tcp,
    Unknown,
}

impl CheckDefinition {
    pub fn interval_in_seconds(&self) -> u64 {
        self.interval
            .as_ref()
            .and_then(|s| s.strip_suffix("s").and_then(|s| s.parse::<u64>().ok()))
            .unwrap_or(DEFAULT_INTERVAL)
    }
    pub fn timeout_in_seconds(&self) -> u64 {
        self.timeout
            .as_ref()
            .and_then(|s| s.strip_suffix("s").and_then(|s| s.parse::<u64>().ok()))
            .unwrap_or(DEFAULT_TIMEOUT)
    }

    pub fn check_type(&self) -> CheckType {
        if self.script.is_some() {
            CheckType::Script
        } else if self.http.is_some() {
            CheckType::Http
        } else if self.tcp.is_some() {
            CheckType::Tcp
        } else {
            CheckType::Unknown
        }
    }

    pub fn http(&self) -> Option<&str> {
        self.http.as_deref()
    }

    pub fn tcp(&self) -> Option<&str> {
        self.tcp.as_deref()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CatalogNodeServices {
    #[serde(rename = "Services", default)]
    pub(crate) services: Option<Vec<CatalogNodeService>>,
}

fn deserialize_optional_string_map<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };

    match value {
        serde_json::Value::Object(_) => serde_json::from_value(value)
            .map(Some)
            .map_err(serde::de::Error::custom),
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Array(values) if values.is_empty() => Ok(None),
        _ => Ok(None),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CatalogNodeService {
    #[serde(rename = "ID")]
    pub(crate) id: String,

    #[serde(
        rename = "Meta",
        default,
        deserialize_with = "deserialize_optional_string_map"
    )]
    pub(crate) meta: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct HealthCheck {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,

    #[serde(rename = "Node")]
    pub(crate) node: String,

    #[serde(rename = "CheckID", skip_serializing_if = "Option::is_none")]
    pub(crate) check_id: Option<String>,

    #[serde(rename = "Name")]
    pub name: String,

    #[serde(rename = "Status")]
    pub status: CheckStatus,

    #[serde(rename = "Notes", skip_serializing_if = "Option::is_none")]
    pub(crate) notes: Option<String>,

    #[serde(rename = "Definition", skip_serializing_if = "Option::is_none")]
    pub definition: Option<CheckDefinition>,

    #[serde(rename = "Output", skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<String>,

    #[serde(rename = "ServiceID", skip_serializing_if = "Option::is_none")]
    pub(crate) service_id: Option<String>,

    #[serde(rename = "ServiceName", skip_serializing_if = "Option::is_none")]
    pub(crate) service_name: Option<String>,

    #[serde(rename = "ServiceTags", skip_serializing_if = "Option::is_none")]
    pub(crate) service_tags: Option<Vec<String>>,

    #[serde(rename = "ServiceMeta", skip_serializing)]
    pub(crate) service_meta: Option<HashMap<String, String>>,

    #[serde(rename = "Namespace", skip_serializing_if = "Option::is_none")]
    pub(crate) namespace: Option<String>,

    #[serde(rename = "ModifyIndex")]
    pub(crate) modify_index: u64,
}

impl std::hash::Hash for HealthCheck {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

impl PartialEq for HealthCheck {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for HealthCheck {}

impl HealthCheck {
    pub fn key(&self) -> String {
        self.check_id
            .as_deref()
            .or(self.id.as_deref())
            .unwrap_or(&self.name)
            .to_string()
    }

    pub fn node(&self) -> &str {
        &self.node
    }

    pub fn service_id(&self) -> Option<&str> {
        self.service_id.as_deref()
    }

    pub fn service_name(&self) -> Option<&str> {
        self.service_name.as_deref()
    }

    pub fn service_meta(&self) -> Option<&HashMap<String, String>> {
        self.service_meta.as_ref()
    }

    pub fn set_service_meta(&mut self, service_meta: HashMap<String, String>) {
        self.service_meta = Some(service_meta);
    }

    pub fn set_status(&mut self, status: CheckStatus) {
        self.status = status;
    }
}

#[derive(Debug, Serialize)]
pub struct CatalogRegistration {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,

    #[serde(rename = "Node")]
    pub(crate) node: String,

    #[serde(rename = "Address", skip_serializing_if = "Option::is_none")]
    pub(crate) address: Option<String>,

    #[serde(rename = "Datacenter", skip_serializing_if = "Option::is_none")]
    pub(crate) datacenter: Option<String>,

    #[serde(rename = "TaggedAddresses", skip_serializing_if = "Option::is_none")]
    pub(crate) tagged_addresses: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "NodeMeta", skip_serializing_if = "Option::is_none")]
    pub(crate) node_meta: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "Service", skip_serializing_if = "Option::is_none")]
    pub(crate) service: Option<ServiceDefinition>,

    #[serde(rename = "SkipNodeUpdate", skip_serializing_if = "Option::is_none")]
    pub(crate) skip_node_update: Option<bool>,

    #[serde(rename = "Check", skip_serializing_if = "Option::is_none")]
    pub(crate) check: Option<HealthCheck>,

    #[serde(rename = "Checks", skip_serializing_if = "Option::is_none")]
    pub(crate) checks: Option<Vec<HealthCheck>>,
}

#[derive(Debug, Serialize)]
pub struct CatalogDeregistration {
    #[serde(rename = "Node")]
    pub(crate) node: String,

    #[serde(rename = "Datacenter", skip_serializing_if = "Option::is_none")]
    pub(crate) datacenter: Option<String>,

    #[serde(rename = "CheckID", skip_serializing_if = "Option::is_none")]
    pub(crate) check_id: Option<String>,

    #[serde(rename = "ServiceID", skip_serializing_if = "Option::is_none")]
    pub(crate) service_id: Option<String>,
}

impl From<Node> for CatalogDeregistration {
    fn from(node: Node) -> Self {
        Self {
            node: node.name,
            datacenter: node.datacenter,
            check_id: None,
            service_id: None,
        }
    }
}

impl From<HealthCheck> for CatalogDeregistration {
    fn from(check: HealthCheck) -> Self {
        Self {
            node: check.node,
            datacenter: None,
            check_id: None,
            service_id: None,
        }
    }
}

impl From<Node> for CatalogRegistration {
    fn from(node: Node) -> Self {
        Self {
            node: node.name,
            address: Some(node.address),
            datacenter: node.datacenter,
            node_meta: node.node_meta,
            service: None,
            tagged_addresses: node.tagged_addresses,
            id: node.id,
            skip_node_update: None,
            check: None,
            checks: None,
        }
    }
}

impl From<HealthCheck> for CatalogRegistration {
    fn from(check: HealthCheck) -> Self {
        Self {
            id: check.id.clone(),
            node: check.node.clone(),
            address: None,
            datacenter: None,
            tagged_addresses: None,
            node_meta: None,
            service: None,
            skip_node_update: Some(true),
            check: Some(check.clone()),
            checks: None,
        }
    }
}
