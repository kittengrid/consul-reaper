use crate::consul::Consul;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ConsulNodeReaper {
    consul: Consul,
    nodes: Vec<Node>,
}

impl ConsulNodeReaper {
    pub fn new(consul: Consul) -> Self {
        Self {
            consul,
            nodes: Vec::new(),
        }
    }
}
