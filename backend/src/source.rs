pub mod pcc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::TenderCandidate;

/// Boundary between BidRadar and an upstream procurement data source.
#[async_trait]
pub trait TenderSource: Send + Sync {
    fn source_name(&self) -> &'static str;

    async fn list_notices(&self, date: NaiveDate) -> Result<Vec<TenderCandidate>>;
}
