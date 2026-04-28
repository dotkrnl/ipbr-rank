pub mod site;
pub mod toml_output;

use std::collections::BTreeMap;

use ipbr_core::{Coefficients, ModelRecord, SourceSummary};

pub struct Scoreboard {
    pub models: Vec<ModelRecord>,
    pub coefficients: Coefficients,
    pub generated_at: String,
    pub generator: String,
    pub methodology: String,
    pub source_summary: BTreeMap<String, SourceSummary>,
}
