use crate::coefficients::Coefficients;
use crate::model::{ModelRecord, SourceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: &str = "1.1.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSummary {
    pub status: String,
    pub rows: usize,
    pub matched: usize,
    pub unmatched: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scoreboard {
    pub schema_version: String,
    pub generated_at: String,
    pub generator_version: String,
    pub coefficients: Coefficients,
    pub source_summary: BTreeMap<SourceId, SourceSummary>,
    pub models: Vec<ModelRecord>,
}

impl Scoreboard {
    pub fn new(generated_at: String, coefficients: Coefficients, models: Vec<ModelRecord>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            generated_at,
            generator_version: env!("CARGO_PKG_VERSION").to_string(),
            coefficients,
            source_summary: BTreeMap::new(),
            models,
        }
    }
}
