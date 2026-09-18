use anyhow::Result;
use chrono::NaiveDate;

use crate::domain::TenderCandidate;

/// Boundary between BidRadar and an upstream procurement data source.
///
/// A concrete PCC adapter will be implemented in GOAL-001 without leaking
/// endpoint/HTML/CSV details into the rule engine or persistence layer.
pub trait TenderSource {
    fn source_name(&self) -> &'static str;

    fn list_notices(
        &self,
        date: NaiveDate,
    ) -> impl std::future::Future<Output = Result<Vec<TenderCandidate>>> + Send;
}
