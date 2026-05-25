mod consul;
mod http_service;

use consul::{
    TestCheckDefinition, TestHealthCheck, TestServiceRegistration, register_node_for_test,
    register_node_service_for_test,
};
use consul_reaper::consul::{CheckStatus, Node};
use http_service::HttpService;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::process::{Child, Command};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};

struct ReaperProcess {
    child: Child,
}

impl ReaperProcess {
    fn start(consul_http_addr: &str, wg_network: &str, events_webhook_url: &str) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_consul-reaper"))
            .arg("--consul-http-addr")
            .arg(consul_http_addr)
            .arg("--wg-network")
            .arg(wg_network)
            .arg("--events-webhook-url")
            .arg(events_webhook_url)
            .arg("--delete-threshold")
            .arg("3")
            .spawn()
            .expect("start consul-reaper");

        Self { child }
    }
}

impl Drop for ReaperProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn random_node(wg_network: &str) -> Node {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    serde_json::from_value(json!({
        "Node": format!("events-webhook-test-node-{}-{}", timestamp, rand::random::<u64>()),
        "Address": format!("10.99.0.{}", (timestamp % 254) + 1),
        "Meta": {
            "external_probe": "true",
            "wg_network": wg_network,
        },
        "ModifyIndex": 0,
    }))
    .expect("valid test node")
}

async fn consul_node_exists(node_name: &str) -> bool {
    let response = reqwest::get(format!(
        "http://localhost:8500/v1/catalog/node/{}",
        node_name
    ))
    .await
    .expect("query consul node");

    if !response.status().is_success() {
        return false;
    }

    let body: Value = response.json().await.expect("valid consul response");
    !body.get("Node").is_none_or(Value::is_null)
}

async fn received_request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap_or_default().len()
}

async fn wait_for_webhook_after<F>(
    server: &MockServer,
    start_at: usize,
    timeout: Duration,
    matches: F,
) where
    F: Fn(&wiremock::Request) -> bool,
{
    tokio::time::timeout(timeout, async {
        loop {
            let requests = server.received_requests().await.unwrap_or_default();
            if requests.iter().skip(start_at).any(&matches) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("timed out waiting for webhook request");
}

async fn wait_for_next_webhook<F>(server: &MockServer, timeout: Duration, matches: F)
where
    F: Fn(&wiremock::Request) -> bool,
{
    let start_at = received_request_count(server).await;
    wait_for_webhook_after(server, start_at, timeout, matches).await;
}

async fn run_while_waiting_for_webhook<F, Fut, T>(
    server: &MockServer,
    timeout: Duration,
    matches: F,
    operation: Fut,
) -> T
where
    F: Fn(&wiremock::Request) -> bool,
    Fut: Future<Output = T>,
{
    let start_at = received_request_count(server).await;
    let wait = wait_for_webhook_after(server, start_at, timeout, matches);
    let ((), result) = tokio::join!(wait, operation);
    result
}

fn json_body(request: &wiremock::Request) -> Value {
    serde_json::from_slice(&request.body).unwrap_or(Value::Null)
}

#[tokio::test]
#[ignore = "requires local Consul on localhost:8500"]
async fn reaper_sends_events_webhooks_for_node_and_service_lifecycle() {
    let wg_network = format!("events-webhook-test-{}", rand::random::<u64>());
    let webhook = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(204))
        .mount(&webhook)
        .await;
    let _reaper = ReaperProcess::start("http://localhost:8500", &wg_network, &webhook.uri());

    let node = random_node(&wg_network);
    let node_name = node.name.clone();
    register_node_for_test(
        "http://localhost:8500",
        &node,
        HashMap::from([
            ("external_probe".to_string(), "true".to_string()),
            ("wg_network".to_string(), wg_network.clone()),
        ]),
    )
    .await
    .unwrap();

    let mut service = HttpService::new().await.unwrap();
    let service_id = "events-webhook-service";
    run_while_waiting_for_webhook(
        &webhook,
        Duration::from_secs(10),
        |request| {
            let body = json_body(request);
            request.method.as_str() == "PATCH"
                && request.url.path() == "/service"
                && body["service"]["id"] == service_id
                && body["service"]["meta"]["id"] == service_id
                && body["network_status"] == "registered"
        },
        register_node_service_for_test(
            "http://localhost:8500",
            &node,
            TestServiceRegistration {
                id: Some(service_id.to_string()),
                name: "events-webhook-service".to_string(),
                port: service.port(),
                tags: Some(vec!["test".to_string()]),
                meta: Some(HashMap::from([("id".to_string(), service_id.to_string())])),
                checks: vec![TestHealthCheck {
                    name: "events-webhook-http-check".to_string(),
                    check_id: Some("events-webhook-http-check".to_string()),
                    status: CheckStatus::Passing,
                    service_id: Some(service_id.to_string()),
                    notes: Some("events webhook HTTP check".to_string()),
                    output: None,
                    definition: Some(TestCheckDefinition {
                        interval: Some("1s".to_string()),
                        timeout: Some("1s".to_string()),
                        http: Some(format!("http://127.0.0.1:{}/", service.port())),
                        tcp: None,
                    }),
                }],
            },
        ),
    )
    .await
    .unwrap();
    println!("Registered service {} on node {}", service_id, node_name);

    println!("Waiting for service {} to become healthy", service_id);
    wait_for_next_webhook(&webhook, Duration::from_secs(10), |request| {
        let body = json_body(request);
        request.method.as_str() == "PATCH"
            && request.url.path() == "/service"
            && body["service"]["id"] == service_id
            && body["service"]["meta"]["id"] == service_id
            && body["network_status"] == "healthy"
    })
    .await;

    println!(
        "Going to stop service {} on node {} to trigger unhealthy status",
        service_id, node_name
    );
    run_while_waiting_for_webhook(
        &webhook,
        Duration::from_secs(10),
        |request| {
            let body = json_body(request);
            request.method.as_str() == "PATCH"
                && request.url.path() == "/service"
                && body["service"]["id"] == service_id
                && body["service"]["meta"]["id"] == service_id
                && body["network_status"] == "unhealthy"
        },
        service.stop(),
    )
    .await;

    run_while_waiting_for_webhook(
        &webhook,
        Duration::from_secs(10),
        |request| {
            let body = json_body(request);
            request.method.as_str() == "PATCH"
                && request.url.path() == "/service"
                && body["service"]["id"] == service_id
                && body["service"]["meta"]["id"] == service_id
                && body["network_status"] == "healthy"
        },
        service.start(),
    )
    .await
    .unwrap();

    let start_at = received_request_count(&webhook).await;
    let unhealthy_wait =
        wait_for_webhook_after(&webhook, start_at, Duration::from_secs(15), |request| {
            let body = json_body(request);
            request.method.as_str() == "PATCH"
                && request.url.path() == "/service"
                && body["service"]["id"] == service_id
                && body["service"]["meta"]["id"] == service_id
                && body["network_status"] == "unhealthy"
        });
    let node_deleted_wait =
        wait_for_webhook_after(&webhook, start_at, Duration::from_secs(15), |request| {
            let body = json_body(request);
            request.method.as_str() == "DELETE"
                && request.url.path() == "/node"
                && body["node"] == node_name
        });
    let ((), (), ()) = tokio::join!(unhealthy_wait, node_deleted_wait, service.stop());

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!consul_node_exists(&node_name).await);
}
