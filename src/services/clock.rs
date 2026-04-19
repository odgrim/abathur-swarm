//! Injectable clock abstraction for deterministic testing of time-dependent services.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::time::Duration;

#[async_trait]
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
    async fn sleep(&self, duration: Duration);
}

pub type DynClock = Arc<dyn Clock>;

pub struct SystemClock;

impl Default for SystemClock {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
    async fn sleep(&self, d: Duration) {
        tokio::time::sleep(d).await
    }
}

pub fn system_clock() -> DynClock {
    Arc::new(SystemClock)
}
