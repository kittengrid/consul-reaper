mod checkers;
mod consul;
use clap::Parser;

use checkers::HealthChecker;
use consul::{HealthCheckEvent, NodeEvent};
use serde_json::json;
use tokio::time::Duration;

use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

const CRITICAL_THRESHOLD: usize = 3; // Number of consecutive failures before marking as critical

async fn perform_health_check<T: HealthChecker>(
    checker: T,
    health_check_name: &str,
    check_type: &str,
) -> Result<(), checkers::HealthCheckError> {
    info!(
        "Running {} health check for: {}",
        check_type, health_check_name
    );
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
    health_check: consul::HealthCheck,
    task: tokio::task::JoinHandle<()>,
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
    fn try_new(health_check: consul::HealthCheck, consul: consul::Consul) -> Result<Self, Error> {
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
                        consul::CheckType::HTTP => {
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
                        consul::CheckType::TCP => {
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
                    if consecutive_times_critical >= CRITICAL_THRESHOLD {
                        health_check.set_status(consul::CheckStatus::Critical);

                        error!(
                            "Health check {} failed {} times consecutively, marking as critical.",
                            health_check.name, consecutive_times_critical
                        );
                    }
                    match consul.register_node(health_check.clone().into()).await {
                        Ok(_) => {
                            debug!(
                                "Health check {} registered successfully.",
                                health_check.name
                            );
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

        Ok(Self { health_check, task })
    }

    fn stop(&mut self) {
        // Logic to stop the health check task
        self.task.abort();
    }
}

struct NodeHealthChecker {
    tasks: Arc<RwLock<HashMap<String, HealthCheckRunner>>>,
    consul: consul::Consul,
    node: consul::Node,
    deleted_node_webhook_url: String,
    check_watcher: Option<tokio::task::JoinHandle<()>>,
    node_watcher: Option<tokio::task::JoinHandle<()>>,
}

impl NodeHealthChecker {
    fn new(node: consul::Node, consul: consul::Consul, critical_node_webhook_url: String) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            consul,
            node,
            deleted_node_webhook_url: critical_node_webhook_url,
            check_watcher: None,
            node_watcher: None,
        }
    }

    async fn all_checks_critical(tasks: Arc<RwLock<HashMap<String, HealthCheckRunner>>>) -> bool {
        if tasks.read().await.is_empty() {
            return false;
        }

        tasks.read().await.values().all(|checker| {
            info!(
                "health status for: {} is {}",
                checker.health_check.name, checker.health_check.status
            );

            checker.health_check.status == consul::CheckStatus::Critical
        })
    }

    fn start(&mut self) {
        if self.check_watcher.is_none() {
            let node_name = self.node.name.clone();
            let consul = self.consul.clone();
            let check_watcher = tokio::spawn({
                let tasks = self.tasks.clone();

                async move {
                    let mut stream = consul.watch_health_checks(&node_name);
                    while let Some(event) = stream.next().await {
                        match event {
                            HealthCheckEvent::Added(check) => {
                                if let Ok(health_checker) =
                                    HealthCheckRunner::try_new(check.clone(), consul.clone())
                                {
                                    tasks
                                        .write()
                                        .await
                                        .insert(check.name.clone(), health_checker);

                                    info!("Healthcheck added: {:?}", check);
                                }
                            }
                            HealthCheckEvent::Removed(check) => {
                                if let Some(mut health_checker) =
                                    tasks.write().await.remove(&check.name)
                                {
                                    health_checker.stop();
                                }
                            }
                            HealthCheckEvent::Updated(check) => {
                                if let Some(mut health_checker) =
                                    tasks.write().await.remove(&check.name)
                                {
                                    health_checker.stop();
                                }
                                if let Ok(health_checker) =
                                    HealthCheckRunner::try_new(check.clone(), consul.clone())
                                {
                                    tasks
                                        .write()
                                        .await
                                        .insert(check.name.clone(), health_checker);

                                    info!("Healthcheck added: {:?}", check);
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
                let tasks = self.tasks.clone();
                let node = self.node.clone();
                let consul = self.consul.clone();
                let deleted_node_webhook_url = self.deleted_node_webhook_url.clone();

                async move {
                    loop {
                        // Sleep for a while before checking again
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        if NodeHealthChecker::all_checks_critical(tasks.clone()).await {
                            info!(
                                "All checks for node {} are critical, removing node.",
                                node.name.clone()
                            );
                            if let Err(e) = consul.deregister_node(node.clone().into()).await {
                                error!("Failed to deregister node {}: {}", node.name.clone(), e);
                            }

                            if let Err(e) = reqwest::Client::new()
                                .delete(&deleted_node_webhook_url)
                                .json(&json!({
                                    "node": node.name.clone(),
                                }))
                                .send()
                                .await
                            {
                                error!(
                                    "Failed to notify deleted node webhook for {}: {}",
                                    node.name.clone(),
                                    e
                                );
                            } else {
                                info!("Deleted node webhook notified for {}", node.name.clone());

                                break;
                            }
                        } else {
                            info!("Node {} is healthy, no action needed.", node.name.clone());
                        }
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
    deleted_node_webhook_url: String,
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
                            args.deleted_node_webhook_url.clone(),
                        );
                        node_health_checker.start();

                        node_health_checkers
                            .write()
                            .await
                            .insert(node.name.clone(), node_health_checker);
                    }
                    NodeEvent::Removed(node) => {
                        info!("Node removed {:?}", node);
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
