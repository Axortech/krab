use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn find_first_by_tenant(&self, tenant_id: &str) -> Result<Option<UserRecord>>;
}
