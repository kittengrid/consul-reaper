use thiserror::Error;
use tokio::time::timeout;
use tracing::debug;

pub struct TCPChecker {
    address: String,
    timeout: u64,
}
#[derive(Debug, Clone, Error)]
pub enum HealthCheckError {
    Timeout,
    Error(String),
}

impl std::fmt::Display for HealthCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthCheckError::Timeout => write!(f, "TCP connection timed out"),
            HealthCheckError::Error(e) => write!(f, "TCP connection error: {}", e),
        }
    }
}

impl TCPChecker {
    pub fn new(address: String, timeout: u64) -> Self {
        TCPChecker { address, timeout }
    }

    pub async fn check(&self) -> Result<(), HealthCheckError> {
        let timeout_duration = std::time::Duration::from_secs(self.timeout);
        debug!("Checking TCP connection to {}", self.address);
        match timeout(
            timeout_duration,
            tokio::net::TcpStream::connect(&self.address),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(HealthCheckError::Error(e.to_string())),
            Err(_) => Err(HealthCheckError::Timeout),
        }
    }
}

pub struct HTTPChecker {
    address: String,
    timeout: u64,
}

impl HTTPChecker {
    pub fn new(url: String, timeout: u64) -> Self {
        HTTPChecker {
            address: url,
            timeout,
        }
    }

    pub async fn check(&self) -> Result<(), HealthCheckError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout))
            .build()
            .unwrap();

        match client.get(&self.address).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(HealthCheckError::Error(format!(
                        "HTTP request failed with status: {}",
                        response.status()
                    )))
                }
            }
            Err(e) => Err(HealthCheckError::Timeout),
        }
    }
}
