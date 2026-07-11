use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisRequest {
    pub owner: String,
    pub repo: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Resolving,
    Downloading,
    Extracting,
    Analyzing,
    Completed,
    CompletedPartial,
    Failed,
}
impl JobState {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::CompletedPartial | Self::Failed
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentState {
    Pending,
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentResult {
    pub analyzer: String,
    pub state: InstrumentState,
    pub coverage: Value,
    pub observations: Value,
    pub limitations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactResult {
    pub repository: RepositoryProvenance,
    pub instruments: BTreeMap<String, InstrumentResult>,
    pub completed_instruments: usize,
    pub failed_instruments: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryProvenance {
    pub full_name: String,
    pub repository_id: u64,
    pub commit: String,
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobResponse {
    pub analysis_id: String,
    pub state: JobState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<CompactResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorEnvelope>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: ErrorEnvelope,
}
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
}
