use crate::{DomainError, PerformanceReading};
use async_trait::async_trait;

#[async_trait]
pub trait PerformanceRepository: Send + Sync {
    async fn read_performance(
        &self,
        serial: &str,
        package: Option<&str>,
    ) -> Result<PerformanceReading, DomainError>;

    async fn performance_packages(&self, serial: &str) -> Result<Vec<String>, DomainError>;
}
