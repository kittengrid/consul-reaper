use consul_reaper::consul::{Consul, HealthCheckEvent, Node, NodeEvent};
use futures::StreamExt;
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

fn test_node(name: String, address: String, meta: Option<HashMap<String, String>>) -> Node {
    let mut value = json!({
        "Node": name,
        "Address": address,
        "ModifyIndex": 0,
    });

    if let Some(meta) = meta {
        value["Meta"] = json!(meta);
    }

    serde_json::from_value(value).expect("valid test node")
}

fn random_node() -> Node {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut meta = HashMap::new();
    meta.insert("env".to_string(), "integration-test".to_string());
    meta.insert("timestamp".to_string(), timestamp.to_string());

    test_node(
        format!("test-node-{}", rand::random::<u64>()),
        format!("192.168.1.{}", (timestamp % 254) + 1),
        Some(meta),
    )
}

#[tokio::test]
#[ignore = "requires local Consul on localhost:8500"]
async fn test_register_node_integration_success() {
    let consul = Consul::new("localhost", 8500);
    let node = random_node();

    let result = consul.register_node(node.clone().into()).await;
    assert!(result.is_ok(), "Failed to register node: {:?}", result);
}

#[tokio::test]
#[ignore = "requires local Consul on localhost:8500"]
async fn test_register_node_integration_minimal() {
    let consul = Consul::new("localhost", 8500);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let node = test_node(
        format!("minimal-node-{}", timestamp),
        format!("10.0.0.{}", (timestamp % 254) + 1),
        None,
    );

    let result = consul.register_node(node.clone().into()).await;
    assert!(
        result.is_ok(),
        "Failed to register minimal node: {:?}",
        result
    );
}

#[tokio::test]
#[ignore = "requires local Consul on localhost:8500"]
async fn test_register_multiple_nodes_integration() {
    let consul = Consul::new("localhost", 8500);

    for i in 0..3 {
        let node = test_node(
            format!("{}-{}", random_node().name, i),
            format!("172.16.0.{}", i + 10),
            None,
        );

        let result = consul.register_node(node.clone().into()).await;
        assert!(
            result.is_ok(),
            "Failed to register node {}: {:?}",
            i,
            result
        );
    }
}

#[tokio::test]
#[ignore = "requires local Consul on localhost:8500"]
async fn test_watch_added_nodes() {
    let consul = Consul::new("localhost", 8500);
    let mut stream = consul.watch_nodes("");

    let new_added_node = random_node();
    let new_deleted_node = random_node();
    let expected_node_added_name = new_added_node.name.clone();

    let task = tokio::spawn(async move {
        loop {
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
                        NodeEvent::Error(err) => panic!("Stream error: {}", err),
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
#[ignore = "requires local Consul on localhost:8500"]
async fn test_watch_removed_nodes() {
    let consul = Consul::new("localhost", 8500);
    let mut stream = consul.watch_nodes("");

    let new_deleted_node = random_node();
    let expected_node_removed_name = new_deleted_node.name.clone();

    consul
        .register_node(new_deleted_node.clone().into())
        .await
        .unwrap();

    let task = tokio::spawn(async move {
        loop {
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
                        NodeEvent::Error(err) => panic!("Stream error: {}", err),
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
#[ignore = "requires local Consul on localhost:8500"]
async fn test_watch_updated_nodes() {
    let consul = Consul::new("localhost", 8500);
    let mut stream = consul.watch_nodes("");

    let outer_node = random_node();
    let outer_node_name = outer_node.name.clone();
    let outer_node_address = outer_node.address().to_string();

    consul
        .register_node(outer_node.clone().into())
        .await
        .unwrap();

    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(node_event) = stream.next() => {
                    match node_event {
                        NodeEvent::Updated(node) => {
                            if node.name == outer_node_name {
                                assert_eq!(node.name, outer_node_name);
                                assert!(node.node_meta().is_some());
                                assert!(node.node_meta().unwrap().contains_key("modified"));
                                break;
                            }
                        },
                        NodeEvent::Added(_)|NodeEvent::Removed(_) => { },
                        NodeEvent::Error(err) => panic!("Stream error: {}", err),
                    }
                }
                else => break,
            }
        }
    });

    sleep(Duration::from_millis(1000)).await;
    let mut meta = HashMap::new();
    meta.insert("modified".to_string(), "true".to_string());
    let modified_node = test_node(outer_node.name.clone(), outer_node_address, Some(meta));

    consul
        .register_node(modified_node.clone().into())
        .await
        .unwrap();
    task.await.unwrap();
}

#[tokio::test]
#[ignore = "requires local Consul on localhost:8500"]
async fn test_watch_health_checks() {
    let consul = Consul::new("localhost", 8500);
    let node = random_node();
    let node_name = node.name.clone();

    consul.register_node(node.clone().into()).await.unwrap();

    let mut stream = consul.watch_health_checks(&node_name);

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
                        HealthCheckEvent::Error(err) => println!("Health check error: {}", err),
                    }

                    if events_received >= 1 {
                        break;
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(10)) => break,
            }
        }
    });

    let catalog_registration = json!({
        "Node": node.name,
        "Address": node.address(),
        "Service": {
            "ID": "web1",
            "Service": "web",
            "Port": 8080,
            "Tags": ["http", "api"]
        },
        "Checks": [
            {
                "Name": "web-check",
                "CheckID": "web",
                "Status": "passing",
                "ServiceID": "web1",
                "Notes": "Web service check",
                "Output": "Web service is healthy",
                "Definition": {
                    "Interval": "10s",
                    "Timeout": "5s",
                    "HTTP": format!("http://{}:8080/health", node.address())
                }
            },
            {
                "Name": "host-check",
                "Status": "passing",
                "Notes": "Web service check",
                "Output": "Web service is healthy",
                "Definition": {
                    "Interval": "10s",
                    "Timeout": "5s",
                    "TCP": "localhost:22"
                }
            }
        ]
    });

    let response = reqwest::Client::new()
        .put("http://localhost:8500/v1/catalog/register")
        .json(&catalog_registration)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    task.await.unwrap();
}
