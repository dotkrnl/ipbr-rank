pub mod site;
pub mod toml_output;

use std::collections::BTreeMap;

use ipbr_core::{Coefficients, ModelRecord, RoleScores, SourceSummary};

pub struct Scoreboard {
    pub models: Vec<ModelRecord>,
    pub coefficients: Coefficients,
    pub generated_at: String,
    pub generator: String,
    pub methodology: String,
    pub source_summary: BTreeMap<String, SourceSummary>,
    /// Prior role scores keyed by canonical_id. Render-only; never persisted
    /// to TOML. Absent canonical_id → no delta rendered for that model.
    pub prev_scores: Option<BTreeMap<String, RoleScores>>,
}
