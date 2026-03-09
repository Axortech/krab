use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait ApiService: Send + Sync {
    async fn start(&self) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            name: "unknown".to_string(),
            host: "127.0.0.1".to_string(),
            port: 8080,
        }
    }
}
