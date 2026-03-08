use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
#[cfg(feature = "redis-store")]
use anyhow::Context;
use async_trait::async_trait;
use tokio::sync::RwLock;

#[async_trait]
pub trait DistributedStore: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<String>>;
    async fn set(&self, key: &str, value: &str, ttl: Duration) -> Result<()>;
    async fn incr(&self, key: &str, delta: u64) -> Result<u64>;
    async fn expire(&self, key: &str, ttl: Duration) -> Result<()>;
}

#[derive(Clone, Default)]
pub struct MemoryStore {
    inner: Arc<RwLock<HashMap<String, MemoryEntry>>>,
}

#[derive(Clone)]
struct MemoryEntry {
    value: String,
    expires_at: Option<Instant>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DistributedStore for MemoryStore {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        {
            let guard = self.inner.read().await;
            if let Some(entry) = guard.get(key) {
                if entry.expires_at.map(|ts| Instant::now() >= ts).unwrap_or(false) {
                    drop(guard);
                    let mut guard = self.inner.write().await;
                    guard.remove(key);
                    return Ok(None);
                }
                return Ok(Some(entry.value.clone()));
            }
        }
        Ok(None)
    }

    async fn set(&self, key: &str, value: &str, ttl: Duration) -> Result<()> {
        let expires_at = if ttl.is_zero() {
            None
        } else {
            Some(Instant::now() + ttl)
        };

        let mut guard = self.inner.write().await;
        guard.insert(
            key.to_string(),
            MemoryEntry {
                value: value.to_string(),
                expires_at,
            },
        );
        Ok(())
    }

    async fn incr(&self, key: &str, delta: u64) -> Result<u64> {
        let mut guard = self.inner.write().await;
        let now = Instant::now();

        let current = match guard.get(key) {
            Some(entry) if entry.expires_at.map(|ts| now >= ts).unwrap_or(false) => {
                guard.remove(key);
                0
            }
            Some(entry) => entry.value.parse::<u64>().unwrap_or(0),
            None => 0,
        };

        let next = current.saturating_add(delta);
        let ttl = guard.get(key).and_then(|entry| entry.expires_at);
        guard.insert(
            key.to_string(),
            MemoryEntry {
                value: next.to_string(),
                expires_at: ttl,
            },
        );
        Ok(next)
    }

    async fn expire(&self, key: &str, ttl: Duration) -> Result<()> {
        let mut guard = self.inner.write().await;
        if let Some(entry) = guard.get_mut(key) {
            entry.expires_at = if ttl.is_zero() {
                None
            } else {
                Some(Instant::now() + ttl)
            };
        }
        Ok(())
    }
}

#[cfg(feature = "redis-store")]
#[derive(Clone)]
pub struct RedisStore {
    client: redis::Client,
}

#[cfg(feature = "redis-store")]
impl RedisStore {
    pub fn new(client: redis::Client) -> Self {
        Self { client }
    }

    pub fn from_url(url: &str) -> Result<Self> {
        let client = redis::Client::open(url).context("invalid redis url")?;
        Ok(Self::new(client))
    }

    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection> {
        self.client
            .get_multiplexed_tokio_connection()
            .await
            .context("failed to connect to redis")
    }
}

#[cfg(feature = "redis-store")]
#[async_trait]
impl DistributedStore for RedisStore {
    async fn get(&self, key: &str) -> Result<Option<String>> {
        use redis::AsyncCommands;

        let mut conn = self.conn().await?;
        conn.get(key).await.context("redis GET failed")
    }

    async fn set(&self, key: &str, value: &str, ttl: Duration) -> Result<()> {
        use redis::AsyncCommands;

        let mut conn = self.conn().await?;
        let ttl_secs = ttl.as_secs().max(1);
        conn.set_ex(key, value, ttl_secs)
            .await
            .context("redis SETEX failed")
    }

    async fn incr(&self, key: &str, delta: u64) -> Result<u64> {
        use redis::AsyncCommands;

        let mut conn = self.conn().await?;
        let value: u64 = conn
            .incr(key, delta)
            .await
            .context("redis INCR failed")?;
        Ok(value)
    }

    async fn expire(&self, key: &str, ttl: Duration) -> Result<()> {
        use redis::AsyncCommands;

        let mut conn = self.conn().await?;
        let ttl_secs = ttl.as_secs().max(1);
        let _: bool = conn
            .expire(key, ttl_secs as i64)
            .await
            .context("redis EXPIRE failed")?;
        Ok(())
    }
}
