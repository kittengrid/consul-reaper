use clap::Parser;

use consul_reaper::checkers;
use consul_reaper::checkers::HealthChecker;
use consul_reaper::consul;
use consul_reaper::consul::{HealthCheckEvent, NodeEvent};
use serde_json::json;

use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info};

const DEFAULT_DELETE_THRESHOLD: usize = 3;

async fn perform_health_check<T: HealthChecker + std::fmt::Debug>(
    checker: T,
    health_check_name: &str,
    check_type: &str,
) -> Result<(), checkers::HealthCheckError> {
    info!("Running health: {:?}", checker);
    match checker.check().await {
        Ok(_) => {
            debug!("{} check succeeded for: {}", check_type, health_check_name);
            Ok(())
        }
        Err(e) => {
            error!("{} check failed: {}", check_type, e);
            Err(e)
        }
    }
}

struct HealthCheckRunner {
    task: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
enum CheckStatusEvent {
    Registered { check: consul::HealthCheck },
    Upsert { check: consul::HealthCheck },
    Removed { check: consul::HealthCheck },
}

use thiserror::Error;
#[derive(Debug, Clone, Error)]
pub enum Error {
    HealthCheckDefinitionMissing,
    HealthCheckTaskFailed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::HealthCheckDefinitionMissing => write!(f, "Health check definition is missing"),
            Error::HealthCheckTaskFailed(msg) => write!(f, "Health check task failed: {}", msg),
        }
    }
}

impl HealthCheckRunner {
    fn try_new(
        health_check: consul::HealthCheck,
        consul: consul::Consul,
        status_tx: mpsc::UnboundedSender<CheckStatusEvent>,
        delete_threshold: usize,
    ) -> Result<Self, Error> {
        let task = tokio::spawn({
            let consul = consul.clone();
            let mut health_check = health_check.clone();

            if health_check.definition.is_none() {
                error!("Health check definition is None, cannot start task.");
                return Err(Error::HealthCheckDefinitionMissing);
            }

            if matches!(
                health_check.definition.clone().unwrap().check_type(),
                consul::CheckType::Unknown
            ) {
                error!("Script health checks are not supported yet.");
                return Err(Error::HealthCheckTaskFailed(
                    "Script health checks are not supported yet.".to_string(),
                ));
            }

            async move {
                let mut consecutive_times_critical = 0;
                let mut last_reported_status: Option<consul::CheckStatus> = None;

                loop {
                    let definition = health_check.definition.clone();
                    if definition.is_none() {
                        error!("Health check definition is None, skipping.");
                        break;
                    }

                    let definition = definition.unwrap();
                    let interval = definition.interval_in_seconds();

                    tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
                    match definition.check_type() {
                        consul::CheckType::Http => {
                            let checker = checkers::HTTPChecker::from(&definition);
                            match perform_health_check(checker, &health_check.name, "HTTP").await {
                                Ok(_) => {
                                    health_check.set_status(consul::CheckStatus::Passing);
                                    consecutive_times_critical = 0;
                                }
                                Err(_) => {
                                    consecutive_times_critical += 1;
                                }
                            }
                        }
                        consul::CheckType::Tcp => {
                            let checker = checkers::TCPChecker::from(&definition);
                            match perform_health_check(checker, &health_check.name, "TCP").await {
                                Ok(_) => {
                                    health_check.set_status(consul::CheckStatus::Passing);
                                    consecutive_times_critical = 0;
                                }
                                Err(_) => {
                                    consecutive_times_critical += 1;
                                }
                            }
                        }
                        _ => {
                            error!("Not implemented: Health check type supported yet.");
                            break;
                        }
                    }
                    if consecutive_times_critical >= delete_threshold {
                        health_check.set_status(consul::CheckStatus::Critical);

                        error!(
                            "Health check {} failed {} times consecutively, marking as critical.",
                            health_check.name, consecutive_times_critical
                        );
                    }

                    if consecutive_times_critical > delete_threshold {
                        error!(
                            "Health check {} failed more than {} times consecutively, keeping it critical.",
                            health_check.name, delete_threshold
                        );
                    }

                    match consul.register_node(health_check.clone().into()).await {
                        Ok(_) => {
                            debug!(
                                "Health check {} registered successfully.",
                                health_check.name
                            );

                            if last_reported_status.as_ref() != Some(&health_check.status) {
                                if let Err(e) = status_tx.send(CheckStatusEvent::Upsert {
                                    check: health_check.clone(),
                                }) {
                                    error!(
                                        "Failed to send status update for health check {}: {}",
                                        health_check.name, e
                                    );
                                } else {
                                    last_reported_status = Some(health_check.status.clone());
                                }
                            }
                        }
                        Err(e) => {
                            error!(
                                "Failed to register health check {}: {}",
                                health_check.name, e
                            );
                        }
                    }
                }
            }
        });

        Ok(Self { task })
    }

    fn stop(&mut self) {
        self.task.abort();
    }
}

struct NodeHealthChecker {
    tasks: Arc<RwLock<HashMap<String, HealthCheckRunner>>>,
    consul: consul::Consul,
    node: consul::Node,
    events_webhook_url: String,
    delete_threshold: usize,
    check_watcher: Option<tokio::task::JoinHandle<()>>,
    node_watcher: Option<tokio::task::JoinHandle<()>>,
}

impl NodeHealthChecker {
    fn new(
        node: consul::Node,
        consul: consul::Consul,
        events_webhook_url: String,
        delete_threshold: usize,
    ) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            consul,
            node,
            events_webhook_url,
            delete_threshold,
            check_watcher: None,
            node_watcher: None,
        }
    }

    async fn stop(&mut self) {
        for (_, mut health_checker) in self.tasks.write().await.drain() {
            health_checker.stop();
        }

        if let Some(check_watcher) = self.check_watcher.take() {
            check_watcher.abort();
        }

        if let Some(node_watcher) = self.node_watcher.take() {
            node_watcher.abort();
        }
    }

    fn start(&mut self) {
        if self.check_watcher.is_none() {
            let (status_tx, mut status_rx) = mpsc::unbounded_channel::<CheckStatusEvent>();
            let node_name = self.node.name.clone();
            let consul = self.consul.clone();
            let check_watcher = tokio::spawn({
                let tasks = self.tasks.clone();
                let status_tx = status_tx.clone();
                let delete_threshold = self.delete_threshold;

                async move {
                    let mut stream = consul.watch_health_checks(&node_name);
                    while let Some(event) = stream.next().await {
                        match event {
                            HealthCheckEvent::Added(check) => {
                                let check_key = check.key();
                                let _ = status_tx.send(CheckStatusEvent::Registered {
                                    check: check.clone(),
                                });

                                if let Ok(health_checker) = HealthCheckRunner::try_new(
                                    check.clone(),
                                    consul.clone(),
                                    status_tx.clone(),
                                    delete_threshold,
                                ) {
                                    tasks.write().await.insert(check_key, health_checker);

                                    info!("Healthcheck added: {:?}", check);
                                }
                            }
                            HealthCheckEvent::Removed(check) => {
                                let check_key = check.key();
                                if let Some(mut health_checker) =
                                    tasks.write().await.remove(&check_key)
                                {
                                    health_checker.stop();
                                }

                                let _ = status_tx.send(CheckStatusEvent::Removed { check });
                            }
                            HealthCheckEvent::Updated(check) => {
                                let check_key = check.key();
                                let _ = status_tx.send(CheckStatusEvent::Upsert {
                                    check: check.clone(),
                                });

                                if let Some(mut health_checker) =
                                    tasks.write().await.remove(&check_key)
                                {
                                    health_checker.stop();
                                }

                                if let Ok(health_checker) = HealthCheckRunner::try_new(
                                    check.clone(),
                                    consul.clone(),
                                    status_tx.clone(),
                                    delete_threshold,
                                ) {
                                    tasks.write().await.insert(check_key, health_checker);

                                    info!("Healthcheck updated: {:?}", check);
                                }
                            }
                            HealthCheckEvent::Error(err) => {
                                error!("Error watching check: {}", err);
                            }
                        }
                    }
                }
            });

            let node_watcher = tokio::spawn({
                let node = self.node.clone();
                let consul = self.consul.clone();
                let service_webhook_url = webhook_url(&self.events_webhook_url, "service");

                async move {
                    let client = reqwest::Client::new();
                    let mut check_statuses: HashMap<String, consul::CheckStatus> = HashMap::new();
                    let mut last_network_status: Option<&'static str> = None;

                    while let Some(event) = status_rx.recv().await {
                        match event {
                            CheckStatusEvent::Registered { check } => {
                                let check_name = check.key();
                                info!("health check registered: {}", check_name);
                                if should_notify_service(&check) {
                                    notify_service_registered(
                                        &client,
                                        &service_webhook_url,
                                        &check,
                                    )
                                    .await;
                                }
                                continue;
                            }
                            CheckStatusEvent::Upsert { check } => {
                                let check_name = check.key();
                                let status = check.status.clone();
                                info!("health status for: {} is {}", check_name, status);
                                check_statuses.insert(check_name, status);
                                if should_notify_service(&check) {
                                    notify_service_status_changed(
                                        &client,
                                        &service_webhook_url,
                                        &check,
                                    )
                                    .await;
                                }
                            }
                            CheckStatusEvent::Removed { check } => {
                                let check_name = check.key();
                                info!("health check removed: {}", check_name);
                                check_statuses.remove(&check_name);
                                if should_notify_service(&check) {
                                    notify_service_deleted(&client, &service_webhook_url, &check)
                                        .await;
                                }
                            }
                        }

                        if check_statuses.is_empty() {
                            continue;
                        }

                        let network_status = if check_statuses
                            .values()
                            .all(|status| *status == consul::CheckStatus::Critical)
                        {
                            "unhealthy"
                        } else {
                            "healthy"
                        };

                        if last_network_status == Some(network_status) {
                            info!("Node {} status unchanged: {}", node.name, network_status);
                            continue;
                        }

                        if network_status == "unhealthy" {
                            match consul.deregister_node(node.clone().into()).await {
                                Ok(_) => {
                                    info!(
                                        "Node {} deregistered because all health checks are critical.",
                                        node.name
                                    );
                                }
                                Err(e) => {
                                    error!(
                                        "Failed to deregister unhealthy node {}: {}",
                                        node.name, e
                                    );
                                    continue;
                                }
                            }
                        }

                        info!("Node {} status changed: {}", node.name, network_status);
                        last_network_status = Some(network_status);
                    }
                }
            });

            self.check_watcher = Some(check_watcher);
            self.node_watcher = Some(node_watcher);
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, env, default_value = "http://localhost:8500")]
    consul_http_addr: String,

    #[arg(long, env)]
    wg_network: String,

    #[arg(long, env)]
    events_webhook_url: String,

    #[arg(long, env, default_value_t = DEFAULT_DELETE_THRESHOLD)]
    delete_threshold: usize,
}

fn webhook_url(base_url: &str, path: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    let base_url = base_url
        .strip_suffix("/node")
        .or_else(|| base_url.strip_suffix("/service"))
        .unwrap_or(base_url);

    format!("{}/{}", base_url, path.trim_start_matches('/'))
}

fn should_notify_service(check: &consul::HealthCheck) -> bool {
    check
        .service_id()
        .is_some_and(|service_id| !service_id.is_empty())
        && check
            .service_meta()
            .is_some_and(|meta| meta.get("id").is_some_and(|id| !id.is_empty()))
}

fn network_status_from_check_status(status: &consul::CheckStatus) -> &'static str {
    match status {
        consul::CheckStatus::Passing => "healthy",
        consul::CheckStatus::Warning | consul::CheckStatus::Critical => "unhealthy",
    }
}

fn service_payload(check: &consul::HealthCheck, network_status: &str) -> serde_json::Value {
    json!({
        "node": check.node(),
        "check": {
            "id": check.key(),
            "name": check.name.clone(),
            "status": check.status.clone(),
        },
        "network_status": network_status,
        "service": {
            "id": check.service_id(),
            "name": check.service_name(),
            "meta": check.service_meta(),
        }
    })
}

async fn notify_service_registered(
    client: &reqwest::Client,
    service_webhook_url: &str,
    check: &consul::HealthCheck,
) {
    if let Err(e) = client
        .patch(service_webhook_url)
        .json(&service_payload(check, "registered"))
        .send()
        .await
    {
        error!(
            "Failed to notify service registration webhook for {}: {}",
            check.key(),
            e
        );
    } else {
        info!("Service registration webhook notified for {}", check.key());
    }
}

async fn notify_service_status_changed(
    client: &reqwest::Client,
    service_webhook_url: &str,
    check: &consul::HealthCheck,
) {
    if let Err(e) = client
        .patch(service_webhook_url)
        .json(&service_payload(
            check,
            network_status_from_check_status(&check.status),
        ))
        .send()
        .await
    {
        error!(
            "Failed to notify service status webhook for {}: {}",
            check.key(),
            e
        );
    } else {
        info!("Service status webhook notified for {}", check.key());
    }
}

async fn notify_service_deleted(
    client: &reqwest::Client,
    service_webhook_url: &str,
    check: &consul::HealthCheck,
) {
    if let Err(e) = client
        .delete(service_webhook_url)
        .json(&service_payload(
            check,
            network_status_from_check_status(&check.status),
        ))
        .send()
        .await
    {
        error!(
            "Failed to notify service deletion webhook for {}: {}",
            check.key(),
            e
        );
    } else {
        info!("Service deletion webhook notified for {}", check.key());
    }
}

async fn notify_node_deleted(client: &reqwest::Client, node_webhook_url: &str, node_name: &str) {
    if let Err(e) = client
        .delete(node_webhook_url)
        .json(&json!({
            "node": node_name,
        }))
        .send()
        .await
    {
        error!(
            "Failed to notify node deletion webhook for {}: {}",
            node_name, e
        );
    } else {
        info!("Node deletion webhook notified for {}", node_name);
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    info!("Consul address: {}", args.consul_http_addr);
    let consul = consul::Consul::from_url(&args.consul_http_addr)?;

    let node_health_checkers: Arc<RwLock<HashMap<String, NodeHealthChecker>>> =
        Arc::new(RwLock::new(HashMap::new()));

    info!("Starting Consul Node Reaper...");

    let node_task = tokio::spawn({
        let consul = consul.clone();
        let webhook_client = reqwest::Client::new();

        async move {
            let mut stream = consul.watch_nodes(
                &("Meta.external_probe == \"true\" and Meta.wg_network == \"".to_string()
                    + &args.wg_network
                    + "\""),
            );

            while let Some(event) = stream.next().await {
                match event {
                    NodeEvent::Added(node) => {
                        info!("Node added {:?}", node);

                        let mut node_health_checker = NodeHealthChecker::new(
                            node.clone(),
                            consul.clone(),
                            args.events_webhook_url.clone(),
                            args.delete_threshold,
                        );
                        node_health_checker.start();

                        node_health_checkers
                            .write()
                            .await
                            .insert(node.name.clone(), node_health_checker);
                    }
                    NodeEvent::Removed(node) => {
                        info!("Node removed {:?}", node);

                        if let Some(mut node_health_checker) =
                            node_health_checkers.write().await.remove(&node.name)
                        {
                            node_health_checker.stop().await;
                        }

                        notify_node_deleted(
                            &webhook_client,
                            &webhook_url(&args.events_webhook_url, "node"),
                            &node.name,
                        )
                        .await;
                    }
                    NodeEvent::Updated(node) => {
                        info!("Node updated: {:?}", node);
                    }
                    NodeEvent::Error(err) => {
                        error!("Error watching nodes: {}", err);
                    }
                }
            }
        }
    });

    node_task.await?;

    Ok(())
}
