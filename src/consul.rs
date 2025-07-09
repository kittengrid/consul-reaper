use futures::stream::{self, Stream};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use std::pin::Pin;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct Consul {
    host: String,
    port: u16,
    client: Client,
}
const DEFAULT_INTERVAL: u64 = 10;
const DEFAULT_TIMEOUT: u64 = 5;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct ServiceDefinition {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    id: Option<String>,

    #[serde(rename = "Service")]
    service: String,

    #[serde(rename = "Port")]
    port: u16,

    #[serde(rename = "Tags", skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
}

#[derive(Default, Debug, Deserialize, Clone)]
pub struct Node {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    id: Option<String>,

    #[serde(rename = "Node")]
    pub name: String,

    #[serde(rename = "Address")]
    address: String,

    #[serde(rename = "Datacenter", skip_serializing_if = "Option::is_none")]
    datacenter: Option<String>,

    #[serde(rename = "Meta", skip_serializing_if = "Option::is_none")]
    node_meta: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "TaggedAddresses", skip_serializing_if = "Option::is_none")]
    tagged_addresses: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "ModifyIndex")]
    modify_index: u64,
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

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
pub enum CheckStatus {
    #[serde(rename = "passing")]
    Passing,

    #[serde(rename = "warning")]
    Warning,

    #[serde(rename = "critical")]
    Critical,
}

impl Default for CheckStatus {
    fn default() -> Self {
        CheckStatus::Passing
    }
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
    script: Option<String>,

    #[serde(rename = "Interval", skip_serializing_if = "Option::is_none")]
    interval: Option<String>,

    #[serde(rename = "Timeout", skip_serializing_if = "Option::is_none")]
    timeout: Option<String>,

    #[serde(rename = "HTTP", skip_serializing_if = "Option::is_none")]
    http: Option<String>,

    #[serde(rename = "TCP", skip_serializing_if = "Option::is_none")]
    tcp: Option<String>,
}

pub enum CheckType {
    Script,
    HTTP,
    TCP,
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
            CheckType::HTTP
        } else if self.tcp.is_some() {
            CheckType::TCP
        } else {
            CheckType::Unknown
        }
    }

    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }

    pub fn http(&self) -> Option<&str> {
        self.http.as_deref()
    }

    pub fn tcp(&self) -> Option<&str> {
        self.tcp.as_deref()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct HealthCheck {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    id: Option<String>,

    #[serde(rename = "Node")]
    node: String,

    #[serde(rename = "CheckID", skip_serializing_if = "Option::is_none")]
    check_id: Option<String>,

    #[serde(rename = "Name")]
    pub name: String,

    #[serde(rename = "Status")]
    pub status: CheckStatus,

    #[serde(rename = "Notes", skip_serializing_if = "Option::is_none")]
    notes: Option<String>,

    #[serde(rename = "Definition", skip_serializing_if = "Option::is_none")]
    pub definition: Option<CheckDefinition>,

    #[serde(rename = "Output", skip_serializing_if = "Option::is_none")]
    output: Option<String>,

    #[serde(rename = "ServiceID", skip_serializing_if = "Option::is_none")]
    service_id: Option<String>,

    #[serde(rename = "ServiceName", skip_serializing_if = "Option::is_none")]
    service_name: Option<String>,

    #[serde(rename = "ServiceTags", skip_serializing_if = "Option::is_none")]
    service_tags: Option<Vec<String>>,

    #[serde(rename = "Namespace", skip_serializing_if = "Option::is_none")]
    namespace: Option<String>,

    #[serde(rename = "ModifyIndex")]
    modify_index: u64,
}

impl std::hash::Hash for HealthCheck {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl PartialEq for HealthCheck {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for HealthCheck {}

impl HealthCheck {
    pub fn set_status(&mut self, status: CheckStatus) {
        self.status = status;
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

#[derive(Debug, Serialize)]
pub struct CatalogRegistration {
    #[serde(rename = "ID", skip_serializing_if = "Option::is_none")]
    id: Option<String>,

    #[serde(rename = "Node")]
    node: String,

    #[serde(rename = "Address", skip_serializing_if = "Option::is_none")]
    address: Option<String>,

    #[serde(rename = "Datacenter", skip_serializing_if = "Option::is_none")]
    datacenter: Option<String>,

    #[serde(rename = "TaggedAddresses", skip_serializing_if = "Option::is_none")]
    tagged_addresses: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "NodeMeta", skip_serializing_if = "Option::is_none")]
    node_meta: Option<std::collections::HashMap<String, String>>,

    #[serde(rename = "Service", skip_serializing_if = "Option::is_none")]
    service: Option<ServiceDefinition>,

    #[serde(rename = "SkipNodeUpdate", skip_serializing_if = "Option::is_none")]
    skip_node_update: Option<bool>,

    #[serde(rename = "Check", skip_serializing_if = "Option::is_none")]
    check: Option<HealthCheck>,

    #[serde(rename = "Checks", skip_serializing_if = "Option::is_none")]
    checks: Option<Vec<HealthCheck>>,
}

#[derive(Debug, Serialize)]
pub struct CatalogDeregistration {
    #[serde(rename = "Node")]
    node: String,

    #[serde(rename = "Datacenter", skip_serializing_if = "Option::is_none")]
    datacenter: Option<String>,

    #[serde(rename = "CheckID", skip_serializing_if = "Option::is_none")]
    check_id: Option<String>,

    #[serde(rename = "ServiceID", skip_serializing_if = "Option::is_none")]
    service_id: Option<String>,
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

#[derive(Debug, Clone)]
struct NodeStreamState {
    consul: Consul,
    last_index: String,
    known_nodes: HashSet<Node>,
    pending_added_nodes: VecDeque<Node>,
    pending_deleted_nodes: VecDeque<Node>,
    pending_updated_nodes: VecDeque<Node>,
    node_versions: HashMap<String, u64>,
    filter: String,
}

impl NodeStreamState {
    fn new(consul: Consul, filter: String) -> Self {
        Self {
            consul,
            last_index: "0".to_string(),
            known_nodes: HashSet::new(),
            pending_added_nodes: VecDeque::new(),
            pending_deleted_nodes: VecDeque::new(),
            pending_updated_nodes: VecDeque::new(),
            node_versions: HashMap::new(),
            filter,
        }
    }

    fn handle_pending_nodes(&mut self) -> Option<NodeEvent> {
        if let Some(node) = self.pending_added_nodes.pop_front() {
            self.known_nodes.insert(node.clone());
            self.node_versions
                .insert(node.name.clone(), node.modify_index);
            return Some(NodeEvent::Added(node));
        }
        if let Some(node) = self.pending_deleted_nodes.pop_front() {
            self.known_nodes.remove(&node);
            self.node_versions.remove(&node.name);
            return Some(NodeEvent::Removed(node));
        }
        if let Some(node) = self.pending_updated_nodes.pop_front() {
            self.known_nodes.remove(&node);
            *self
                .node_versions
                .entry(node.name.clone())
                .or_insert(node.modify_index) = node.modify_index;
            self.known_nodes.insert(node.clone());
            return Some(NodeEvent::Updated(node));
        }
        None
    }

    fn process_node_changes(&mut self, nodes: Vec<Node>) -> Option<NodeEvent> {
        let current_nodes: HashSet<Node> = nodes.iter().cloned().collect();

        let mut added_nodes: VecDeque<Node> = current_nodes
            .difference(&self.known_nodes)
            .cloned()
            .collect();

        let mut deleted_nodes: VecDeque<Node> = self
            .known_nodes
            .difference(&current_nodes)
            .cloned()
            .collect();

        let mut updated_nodes = VecDeque::<Node>::new();
        for node in current_nodes {
            if let Some(version) = self.node_versions.get(&node.name) {
                if version < &node.modify_index {
                    updated_nodes.push_back(node.clone());
                }
            }
        }

        if let Some(new_node) = added_nodes.pop_front() {
            self.known_nodes.insert(new_node.clone());
            self.node_versions
                .insert(new_node.name.clone(), new_node.modify_index);

            self.pending_added_nodes = added_nodes;
            self.pending_deleted_nodes = deleted_nodes;
            self.pending_updated_nodes = updated_nodes;

            return Some(NodeEvent::Added(new_node));
        }

        if let Some(removed_node) = deleted_nodes.pop_front() {
            self.known_nodes.remove(&removed_node);
            self.node_versions.remove(&removed_node.name);

            self.pending_added_nodes = added_nodes;
            self.pending_deleted_nodes = deleted_nodes;
            self.pending_updated_nodes = updated_nodes;
            return Some(NodeEvent::Removed(removed_node));
        }

        if let Some(updated_node) = updated_nodes.pop_front() {
            self.known_nodes.remove(&updated_node);
            self.known_nodes.insert(updated_node.clone());
            self.node_versions
                .insert(updated_node.name.clone(), updated_node.modify_index);

            self.pending_added_nodes = added_nodes;
            self.pending_deleted_nodes = deleted_nodes;
            self.pending_updated_nodes = updated_nodes;
            return Some(NodeEvent::Updated(updated_node));
        }

        None
    }

    async fn fetch_nodes_from_consul(&mut self) -> Result<Vec<Node>, NodeEvent> {
        let response = self
            .consul
            .get("/v1/catalog/nodes?{}")
            .query(&[
                ("wait", "5s"),
                ("index", &self.last_index),
                ("filter", &self.filter),
            ])
            .send()
            .await
            .map_err(|e| {
                eprintln!("Request error: {}", e);
                e
            })
            .map_err(|e| NodeEvent::Error(e.to_string()))?;

        if let Some(index_header) = response.headers().get("X-Consul-Index") {
            if let Ok(index_str) = index_header.to_str() {
                self.last_index = index_str.to_string();
            }
        }

        let response_bytes = response.bytes().await.unwrap_or_default();
        let nodes: Vec<Node> =
            serde_json::from_slice(&response_bytes).map_err(|e| NodeEvent::Error(e.to_string()))?;

        Ok(nodes)
    }
}

#[derive(Debug, Clone)]
struct HealthCheckStreamState {
    consul: Consul,
    node_name: String,
    last_index: String,
    known_checks: HashSet<HealthCheck>,
    check_versions: HashMap<String, u64>,
    pending_added_checks: VecDeque<HealthCheck>,
    pending_deleted_checks: VecDeque<HealthCheck>,
    pending_updated_checks: VecDeque<HealthCheck>,
}

impl HealthCheckStreamState {
    fn new(consul: Consul, node_name: String) -> Self {
        Self {
            consul,
            node_name,
            last_index: "0".to_string(),
            known_checks: HashSet::new(),
            check_versions: HashMap::new(),
            pending_added_checks: VecDeque::new(),
            pending_deleted_checks: VecDeque::new(),
            pending_updated_checks: VecDeque::new(),
        }
    }

    fn handle_pending_checks(&mut self) -> Option<HealthCheckEvent> {
        if let Some(check) = self.pending_added_checks.pop_front() {
            self.known_checks.insert(check.clone());
            self.check_versions
                .insert(check.name.clone(), check.modify_index);

            return Some(HealthCheckEvent::Added(check));
        }
        if let Some(check) = self.pending_deleted_checks.pop_front() {
            self.known_checks.remove(&check);
            self.check_versions.remove(&check.name);

            return Some(HealthCheckEvent::Removed(check));
        }
        if let Some(check) = self.pending_updated_checks.pop_front() {
            self.known_checks.remove(&check);
            self.known_checks.insert(check.clone());
            self.check_versions
                .insert(check.name.clone(), check.modify_index);
            return Some(HealthCheckEvent::Updated(check));
        }
        None
    }

    fn process_check_changes(&mut self, checks: Vec<HealthCheck>) -> Option<HealthCheckEvent> {
        let current_checks: HashSet<HealthCheck> = checks.iter().cloned().collect();

        let mut added_checks: VecDeque<HealthCheck> = current_checks
            .difference(&self.known_checks)
            .cloned()
            .collect();

        let mut deleted_checks: VecDeque<HealthCheck> = self
            .known_checks
            .difference(&current_checks)
            .cloned()
            .collect();

        let mut updated_checks = VecDeque::<HealthCheck>::new();
        for check in current_checks {
            if let Some(version) = self.check_versions.get(&check.name) {
                if version < &check.modify_index {
                    updated_checks.push_back(check.clone());
                }
            }
        }

        if let Some(new_check) = added_checks.pop_front() {
            self.known_checks.insert(new_check.clone());
            self.pending_added_checks = added_checks;
            self.pending_deleted_checks = deleted_checks;
            self.pending_updated_checks = updated_checks;
            self.check_versions
                .insert(new_check.name.clone(), new_check.modify_index);
            return Some(HealthCheckEvent::Added(new_check));
        }

        if let Some(removed_check) = deleted_checks.pop_front() {
            self.known_checks.remove(&removed_check);
            self.check_versions.remove(&removed_check.name);
            self.pending_added_checks = added_checks;
            self.pending_deleted_checks = deleted_checks;
            self.pending_updated_checks = updated_checks;
            return Some(HealthCheckEvent::Removed(removed_check));
        }

        if let Some(updated_check) = updated_checks.pop_front() {
            self.known_checks.remove(&updated_check);
            self.known_checks.insert(updated_check.clone());
            self.pending_added_checks = added_checks;
            self.pending_deleted_checks = deleted_checks;
            self.pending_updated_checks = updated_checks;
            self.check_versions
                .insert(updated_check.name.clone(), updated_check.modify_index);

            return Some(HealthCheckEvent::Updated(updated_check));
        }

        None
    }

    async fn fetch_checks_from_consul(&mut self) -> Result<Vec<HealthCheck>, HealthCheckEvent> {
        let url = format!("/v1/health/node/{}", self.node_name);
        let response = self
            .consul
            .get(&url)
            .query(&[("wait", "5s"), ("index", &self.last_index)])
            .send()
            .await
            .map_err(|e| {
                eprintln!("Request error: {}", e);
                e
            })
            .map_err(|e| HealthCheckEvent::Error(e.to_string()))?;

        if let Some(index_header) = response.headers().get("X-Consul-Index") {
            if let Ok(index_str) = index_header.to_str() {
                self.last_index = index_str.to_string();
            }
        }

        let response_bytes = response.bytes().await.unwrap_or_default();
        let checks: Vec<HealthCheck> = serde_json::from_slice(&response_bytes)
            .map_err(|e| HealthCheckEvent::Error(e.to_string()))?;

        Ok(checks)
    }
}

#[derive(Debug)]
pub enum NodeEvent {
    Added(Node),
    Removed(Node),
    Updated(Node),
    Error(String),
}

#[derive(Debug)]
pub enum HealthCheckEvent {
    Added(HealthCheck),
    Removed(HealthCheck),
    Updated(HealthCheck),
    Error(String),
}

impl Consul {
    /// Returns a new instance of the Consul client.
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            client: Client::new(),
        }
    }

    pub fn from_url(url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let parsed_url = url::Url::parse(url)?;
        let host = parsed_url
            .host_str()
            .ok_or("Invalid URL: missing host")?
            .to_string();
        let port = parsed_url.port().unwrap_or(80);
        Ok(Self::new(&host, port))
    }

    /// A tokio::stream that will return new nodes as they are registered/deregisterd/updated in Consul.
    pub fn watch_nodes(&self, filter: &str) -> Pin<Box<dyn Stream<Item = NodeEvent> + Send>> {
        let state = NodeStreamState::new(self.clone(), filter.to_string());

        Box::pin(stream::unfold(
            Arc::new(Mutex::new(state)),
            |state| async move {
                let mut state_guard = state.lock().await;

                loop {
                    if let Some(event) = state_guard.handle_pending_nodes() {
                        return Some((event, state.clone()));
                    }

                    match state_guard.fetch_nodes_from_consul().await {
                        Ok(nodes) => {
                            if let Some(event) = state_guard.process_node_changes(nodes) {
                                return Some((event, state.clone()));
                            }
                            sleep(Duration::from_secs(1)).await;
                        }
                        Err(e) => {
                            return Some((e, state.clone()));
                        }
                    }
                }
            },
        ))
    }

    /// A tokio::stream that will return health check events for a specific node.
    pub fn watch_health_checks(
        &self,
        node_name: &str,
    ) -> Pin<Box<dyn Stream<Item = HealthCheckEvent> + Send>> {
        let state = HealthCheckStreamState::new(self.clone(), node_name.to_string());

        Box::pin(stream::unfold(
            Arc::new(Mutex::new(state)),
            |state| async move {
                let mut state_guard = state.lock().await;

                loop {
                    if let Some(event) = state_guard.handle_pending_checks() {
                        return Some((event, state.clone()));
                    }

                    match state_guard.fetch_checks_from_consul().await {
                        Ok(checks) => {
                            if let Some(event) = state_guard.process_check_changes(checks) {
                                return Some((event, state.clone()));
                            }
                            sleep(Duration::from_secs(1)).await;
                        }
                        Err(e) => {
                            return Some((e, state.clone()));
                        }
                    }
                }
            },
        ))
    }

    /// Deregisters a node in the Consul catalog.
    /// It uses a CatalogDeregistration struct to send the registration data.
    /// Note that you can create a CatalogDeregistration from a Node using the From trait.
    pub async fn deregister_node(
        &self,
        registration: CatalogDeregistration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let response = self
            .put("/v1/catalog/deregister")
            .json(&registration)
            .send()
            .await?;

        if !response.status().is_success() {
            let body = response.text().await?;
            return Err(format!("Failed to register node: {}", body).into());
        }

        Ok(())
    }

    /// Registers a new node in the Consul catalog.
    /// It uses a CatalogRegistration struct to send the registration data.
    /// Note that you can create a CatalogRegistration from a Node using the From trait.
    pub async fn register_node(
        &self,
        registration: CatalogRegistration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let response = self
            .put("/v1/catalog/register")
            .json(&registration)
            .send()
            .await?;

        if !response.status().is_success() {
            let body = response.text().await?;
            return Err(format!("Failed to register node: {}", body).into());
        }

        Ok(())
    }

    #[cfg(test)]
    pub fn new_with_client(host: String, port: u16, client: Client) -> Self {
        Self { host, port, client }
    }

    fn put(&self, url: &str) -> reqwest::RequestBuilder {
        let full_url = if self.port == 0 {
            format!("http://{}{}", self.host, url)
        } else {
            format!("http://{}:{}{}", self.host, self.port, url)
        };
        self.client.put(full_url)
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let full_url = if self.port == 0 {
            format!("http://{}{}", self.host, url)
        } else {
            format!("http://{}:{}{}", self.host, self.port, url)
        };
        self.client.get(full_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    fn test_node() -> Node {
        Node {
            name: "test-node".to_string(),
            address: "192.168.1.100".to_string(),
            datacenter: Some("dc1".to_string()),
            node_meta: Some({
                let mut meta = HashMap::new();
                meta.insert("env".to_string(), "test".to_string());
                meta
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_register_node_success() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("PUT", "/v1/catalog/register")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("true")
            .create();

        let client = reqwest::Client::new();
        let consul = Consul::new_with_client(server.host_with_port(), 0, client);
        let node = test_node();

        let result = consul.register_node((node).into()).await;
        if let Err(e) = &result {
            eprintln!("Error: {}", e);
        }
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_node_with_minimal_data() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("PUT", "/v1/catalog/register")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("true")
            .create();

        let client = reqwest::Client::new();
        let consul = Consul::new_with_client(server.host_with_port(), 0, client);
        let node = Node {
            name: "minimal-node".to_string(),
            address: "10.0.0.1".to_string(),
            datacenter: None,
            node_meta: None,
            ..Default::default()
        };

        let result = consul.register_node(node.into()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_node_server_error() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("PUT", "/v1/catalog/register")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body("Internal Server Error")
            .create();

        let client = reqwest::Client::new();
        let consul = Consul::new_with_client(server.host_with_port(), 0, client);
        let node = test_node();

        let result = consul.register_node(node.into()).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to register node")
        );
    }

    #[tokio::test]
    async fn test_register_node_request_body() {
        let mut server = Server::new_async().await;
        let _m = server.mock("PUT", "/v1/catalog/register")
            .match_body(mockito::Matcher::JsonString(r#"{"Node":"test-node","Address":"192.168.1.100","Datacenter":"dc1","NodeMeta":{"env":"test"},"Service":{"Service":"web","Port":8080,"Tags":["http","api"]}}"#.to_string()))
            .with_status(200)
            .with_body("true")
            .create();

        let client = reqwest::Client::new();
        let consul = Consul::new_with_client(server.host_with_port(), 0, client);
        let node = test_node();
        let mut registration: CatalogRegistration = node.into();
        registration.service = Some(ServiceDefinition {
            id: None,
            service: "web".to_string(),
            port: 8080,
            tags: Some(vec!["http".to_string(), "api".to_string()]),
        });

        let result = consul.register_node(registration).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_catalog_registration_from_node() {
        let node = test_node();
        let mut registration: CatalogRegistration = node.into();
        registration.service = Some(ServiceDefinition {
            id: None,
            service: "web".to_string(),
            port: 8080,
            tags: Some(vec!["http".to_string(), "api".to_string()]),
        });

        assert_eq!(registration.node, "test-node");
        assert_eq!(registration.address, Some("192.168.1.100".to_string()));
        assert_eq!(registration.datacenter, Some("dc1".to_string()));
        assert!(registration.node_meta.is_some());
        assert!(registration.service.is_some());

        let service = registration.service.unwrap();
        assert_eq!(service.service, "web");
        assert_eq!(service.port, 8080);
        assert_eq!(
            service.tags,
            Some(vec!["http".to_string(), "api".to_string()])
        );
    }

    // Integration tests against local Consul
    fn random_node() -> Node {
        use std::time::{SystemTime, UNIX_EPOCH};
        let name = format!("test-node-{}", rand::random::<u64>());

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Node {
            name,
            address: format!("192.168.1.{}", (timestamp % 254) + 1),
            datacenter: Some("dc1".to_string()),
            node_meta: Some({
                let mut meta = std::collections::HashMap::new();
                meta.insert("env".to_string(), "integration-test".to_string());
                meta.insert("timestamp".to_string(), timestamp.to_string());
                meta
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_register_node_integration_success() {
        let consul = Consul::new("localhost", 8500);
        let node = random_node();

        let result = consul.register_node(node.clone().into()).await;
        assert!(result.is_ok(), "Failed to register node: {:?}", result);
    }

    #[tokio::test]
    async fn test_register_node_integration_minimal() {
        let consul = Consul::new("localhost", 8500);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let node = Node {
            name: format!("minimal-node-{}", timestamp),
            address: format!("10.0.0.{}", (timestamp % 254) + 1),
            datacenter: None,
            node_meta: None,
            ..Default::default()
        };

        let result = consul.register_node(node.clone().into()).await;
        assert!(
            result.is_ok(),
            "Failed to register minimal node: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_register_multiple_nodes_integration() {
        let consul = Consul::new("localhost", 8500);

        for i in 0..3 {
            let mut node = random_node();
            node.name = format!("{}-{}", node.name, i);
            node.address = format!("172.16.0.{}", i + 10);

            let result = consul.register_node(node.clone().into()).await;
            assert!(
                result.is_ok(),
                "Failed to register node {}: {:?}",
                i,
                result
            );
        }
    }
    use futures::StreamExt;

    #[tokio::test]
    async fn test_watch_added_nodes() {
        let consul = Consul::new("localhost", 8500);
        let mut stream = consul.watch_nodes("");

        // Create a task to wait for the new node
        let new_added_node = random_node();
        let new_deleted_node = random_node();
        let expected_node_added_name = new_added_node.name.clone();

        let task = tokio::spawn(async move {
            loop {
                // Wait for the next node in the stream
                tokio::select! {
                    Some(node_event) = stream.next() => {
                        match node_event {
                            NodeEvent::Added(node) => {
                                if node.name == expected_node_added_name {
                                    assert_eq!(node.name, expected_node_added_name);
                                    break;
                                }
                            },
                            NodeEvent::Removed(_)|NodeEvent::Updated(_) => { },
                            NodeEvent::Error(err) => {
                                panic!("Stream error: {}", err);
                            },
                        }
                    }
                    else => break,
                }
            }
        });

        sleep(Duration::from_millis(100)).await;

        consul
            .register_node(new_added_node.clone().into())
            .await
            .unwrap();
        consul
            .deregister_node(new_deleted_node.clone().into())
            .await
            .unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn test_watch_removed_nodes() {
        let consul = Consul::new("localhost", 8500);
        let mut stream = consul.watch_nodes("");

        // Create a task to wait for the new node
        let new_deleted_node = random_node();
        let expected_node_removed_name = new_deleted_node.name.clone();

        consul
            .register_node(new_deleted_node.clone().into())
            .await
            .unwrap();

        let task = tokio::spawn(async move {
            loop {
                // Wait for the next node in the stream
                tokio::select! {
                    Some(node_event) = stream.next() => {
                        match node_event {
                            NodeEvent::Removed(node) => {
                                if node.name == expected_node_removed_name {
                                    assert_eq!(node.name, expected_node_removed_name);
                                    break;
                                }
                            },
                            NodeEvent::Added(_)|NodeEvent::Updated(_) => { },
                            NodeEvent::Error(err) => {
                                panic!("Stream error: {}", err);
                            },
                        }
                    }
                    else => break,
                }
            }
        });

        sleep(Duration::from_millis(100)).await;

        consul
            .deregister_node(new_deleted_node.clone().into())
            .await
            .unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn test_watch_updated_nodes() {
        let consul = Consul::new("localhost", 8500);
        let mut stream = consul.watch_nodes("");

        // Create a task to wait for the new node
        let outer_node = random_node();
        let outer_node_name = outer_node.name.clone();

        consul
            .register_node(outer_node.clone().into())
            .await
            .unwrap();

        let task = tokio::spawn(async move {
            loop {
                // Wait for the next node in the stream
                tokio::select! {
                    Some(node_event) = stream.next() => {
                        match node_event {
                            NodeEvent::Updated(node) => {
                                if node.name == outer_node_name {
                                    assert_eq!(node.name, outer_node_name);
                                    assert!(node.node_meta.is_some());
                                    assert!(node.node_meta.as_ref().unwrap().contains_key("modified"));
                                    break;
                                }
                            },
                            NodeEvent::Added(_)|NodeEvent::Removed(_) => { },
                            NodeEvent::Error(err) => {
                                panic!("Stream error: {}", err);
                            },
                        }
                    }
                    else => break,
                }
            }
        });

        sleep(Duration::from_millis(1000)).await;
        let mut modified_node = outer_node.clone();
        modified_node.node_meta = Some({
            let mut meta = HashMap::new();
            meta.insert("modified".to_string(), "true".to_string());
            meta
        });

        consul
            .register_node(modified_node.clone().into())
            .await
            .unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn test_watch_health_checks() {
        let consul = Consul::new("localhost", 8500);
        let node = random_node();
        let node_name = node.name.clone();

        // Register a node first
        consul.register_node(node.clone().into()).await.unwrap();

        let mut stream = consul.watch_health_checks(&node_name);

        // Create a task to wait for health check events
        let task = tokio::spawn(async move {
            let mut events_received = 0;
            loop {
                tokio::select! {
                    Some(event) = stream.next() => {
                        match event {
                            HealthCheckEvent::Added(check) => {
                                println!("Health check added: {:?}", check.name);
                                events_received += 1;
                            },
                            HealthCheckEvent::Updated(check) => {
                                println!("Health check updated: {:?}", check.name);
                                events_received += 1;
                            },
                            HealthCheckEvent::Removed(check) => {
                                println!("Health check removed: {:?}", check.name);
                                events_received += 1;
                            },
                            HealthCheckEvent::Error(err) => {
                                println!("Health check error: {}", err);
                            },
                        }

                        if events_received >= 1 {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(10)) => {
                        break;
                    }
                }
            }
        });

        let catalog_registration = CatalogRegistration {
            id: None,
            address: Some(node.address.clone()),
            datacenter: None,
            tagged_addresses: None,
            node_meta: None,
            node: node.name.clone(),
            service: Some(ServiceDefinition {
                id: Some("web1".to_string()),
                service: "web".to_string(),
                port: 8080,
                tags: Some(vec!["http".to_string(), "api".to_string()]),
            }),
            skip_node_update: None,
            checks: Some(vec![
                HealthCheck {
                    name: "web-check".to_string(),
                    check_id: Some("web".to_string()),
                    status: CheckStatus::Passing,
                    service_id: Some("web1".to_string()),
                    notes: Some("Web service check".to_string()),
                    output: Some("Web service is healthy".to_string()),
                    definition: Some(CheckDefinition {
                        script: None,
                        interval: Some("10s".to_string()),
                        timeout: Some("5s".to_string()),
                        http: Some(format!("http://{}:8080/health", node.address)),
                        tcp: None,
                    }),
                    ..HealthCheck::default()
                },
                HealthCheck {
                    name: "host-check".to_string(),
                    check_id: None,
                    status: CheckStatus::Passing,
                    service_id: None,
                    notes: Some("Web service check".to_string()),
                    output: Some("Web service is healthy".to_string()),
                    definition: Some(CheckDefinition {
                        script: None,
                        interval: Some("10s".to_string()),
                        timeout: Some("5s".to_string()),
                        http: None,
                        tcp: Some("localhost:22".to_string()),
                    }),
                    ..HealthCheck::default()
                },
            ]),
            check: None,
        };

        consul.register_node(catalog_registration).await.unwrap();

        // Wait for initial health checks to be detected
        task.await.unwrap();
    }
}
