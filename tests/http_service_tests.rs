mod http_service;

use http_service::HttpService;

#[tokio::test]
async fn starts_on_random_port_and_responds() {
    let service = HttpService::new().await.unwrap();

    assert!(service.port() > 0);
    assert!(service.is_running());

    let response = reqwest::get(format!("http://127.0.0.1:{}/", service.port()))
        .await
        .unwrap();

    assert!(response.status().is_success());
    assert_eq!(response.text().await.unwrap(), "OK");
}

#[tokio::test]
async fn can_stop_and_start_again() {
    let mut service = HttpService::new().await.unwrap();
    let first_port = service.port();

    service.stop().await;
    assert!(!service.is_running());

    service.start().await.unwrap();
    assert!(service.is_running());
    assert_eq!(service.port(), first_port);

    let response = reqwest::get(format!("http://127.0.0.1:{}/", service.port()))
        .await
        .unwrap();

    assert!(response.status().is_success());
    assert_ne!(first_port, 0);
}
