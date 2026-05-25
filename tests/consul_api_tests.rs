mod consul;

use consul::{
    TestCheckDefinition, TestHealthCheck, TestServiceRegistration, register_node_service_for_test,
};
use consul_reaper::consul::{CheckStatus, Node};
use mockito::Server;
use serde_json::json;
use std::collections::HashMap;

fn test_node() -> Node {
    serde_json::from_value(json!({
        "Node": "test-node",
        "Address": "192.168.1.100",
        "ModifyIndex": 0,
    }))
    .expect("valid test node")
}

#[tokio::test]
async fn test_register_node_service_for_test_request_body() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("PUT", "/v1/catalog/register")
        .match_body(mockito::Matcher::JsonString(r#"{"Node":"test-node","Address":"192.168.1.100","Service":{"ID":"web1","Service":"web","Port":8080,"Tags":["http","api"],"Meta":{"label":"test"}},"SkipNodeUpdate":true,"Checks":[{"Name":"web-check","CheckID":"web","Status":"passing","ServiceID":"web1","Notes":"Web service check","Output":"Web service is healthy","Definition":{"Interval":"10s","Timeout":"5s","HTTP":"http://192.168.1.100:8080/health"}}]}"#.to_string()))
        .with_status(200)
        .with_body("true")
        .create();

    let result = register_node_service_for_test(
        &server.url(),
        &test_node(),
        TestServiceRegistration {
            id: Some("web1".to_string()),
            name: "web".to_string(),
            port: 8080,
            tags: Some(vec!["http".to_string(), "api".to_string()]),
            meta: Some(HashMap::from([("label".to_string(), "test".to_string())])),
            checks: vec![TestHealthCheck {
                name: "web-check".to_string(),
                check_id: Some("web".to_string()),
                status: CheckStatus::Passing,
                service_id: None,
                notes: Some("Web service check".to_string()),
                output: Some("Web service is healthy".to_string()),
                definition: Some(TestCheckDefinition {
                    interval: Some("10s".to_string()),
                    timeout: Some("5s".to_string()),
                    http: Some("http://192.168.1.100:8080/health".to_string()),
                    tcp: None,
                }),
            }],
        },
    )
    .await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_register_node_service_for_test_server_error() {
    let mut server = Server::new_async().await;
    let _m = server
        .mock("PUT", "/v1/catalog/register")
        .with_status(500)
        .with_body("Internal Server Error")
        .create();

    let result = register_node_service_for_test(
        &server.url(),
        &test_node(),
        TestServiceRegistration {
            id: Some("web1".to_string()),
            name: "web".to_string(),
            port: 8080,
            tags: None,
            meta: None,
            checks: Vec::new(),
        },
    )
    .await;

    assert!(result.is_err());
}
