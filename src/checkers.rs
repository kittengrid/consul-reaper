use crate::consul::CheckDefinition;
use thiserror::Error;
use tokio::time::timeout;
use tracing::debug;

#[async_trait::async_trait]
pub trait HealthChecker {
    async fn check(&self) -> Result<(), HealthCheckError>;
}

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

impl From<&CheckDefinition> for TCPChecker {
    fn from(definition: &CheckDefinition) -> Self {
        TCPChecker {
            address: definition.tcp().unwrap().to_string(),
            timeout: definition.timeout_in_seconds(),
        }
    }
}

#[async_trait::async_trait]
impl HealthChecker for TCPChecker {
    async fn check(&self) -> Result<(), HealthCheckError> {
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

impl From<&CheckDefinition> for HTTPChecker {
    fn from(definition: &CheckDefinition) -> Self {
        HTTPChecker {
            address: definition.http().unwrap().to_string(),
            timeout: definition.timeout_in_seconds(),
        }
    }
}

#[async_trait::async_trait]
impl HealthChecker for HTTPChecker {
    async fn check(&self) -> Result<(), HealthCheckError> {
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
            Err(_) => Err(HealthCheckError::Timeout),
        }
    }
}
