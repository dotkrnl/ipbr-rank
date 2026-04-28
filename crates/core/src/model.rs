use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissingInfo {
    pub metrics: BTreeSet<MetricKey>,
    pub groups_shrunk: BTreeSet<GroupKey>,
    #[serde(default)]
    pub synthesis_dominant: bool,
}

impl MissingInfo {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SynthesisProvenance {
    pub source_id: SourceId,
    pub from: String,
}

pub type MetricKey = String;
pub type GroupKey = String;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Vendor {
    Openai,
    Anthropic,
    Google,
    Moonshot,
    Zai,
    Xai,
    Alibaba,
    Deepseek,
    Mistral,
    Meta,
    Minimax,
    Nvidia,
    Baidu,
    Tencent,
    Inclusionai,
    Xiaomi,
    #[serde(untagged)]
    Other(String),
}

impl Vendor {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::Moonshot => "moonshot",
            Self::Zai => "zai",
            Self::Xai => "xai",
            Self::Alibaba => "alibaba",
            Self::Deepseek => "deepseek",
            Self::Mistral => "mistral",
            Self::Meta => "meta",
            Self::Minimax => "minimax",
            Self::Nvidia => "nvidia",
            Self::Baidu => "baidu",
            Self::Tencent => "tencent",
            Self::Inclusionai => "inclusionai",
            Self::Xiaomi => "xiaomi",
            Self::Other(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingEffort {
    Low,
    Medium,
    High,
}

pub type SourceId = String;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleScores {
    pub i_raw: f64,
    pub p_raw: f64,
    pub b_raw: f64,
    pub r: f64,
    pub i_adj: f64,
    pub p_adj: f64,
    pub b_adj: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub canonical_id: String,
    pub display_name: String,
    pub vendor: Vendor,
    pub thinking_effort: Option<ThinkingEffort>,
    pub aliases: BTreeSet<String>,
    pub sources: BTreeSet<SourceId>,
    pub raw_metrics: BTreeMap<MetricKey, f64>,
    pub metrics: BTreeMap<MetricKey, f64>,
    pub groups: BTreeMap<GroupKey, f64>,
    pub scores: RoleScores,
    pub missing: MissingInfo,
    pub synthesized: BTreeMap<MetricKey, SynthesisProvenance>,
    #[serde(default)]
    pub override_reported: BTreeSet<MetricKey>,
}

impl ModelRecord {
    pub fn new(canonical_id: String, display_name: String, vendor: Vendor) -> Self {
        Self {
            canonical_id,
            display_name,
            vendor,
            thinking_effort: None,
            aliases: BTreeSet::new(),
            sources: BTreeSet::new(),
            raw_metrics: BTreeMap::new(),
            metrics: BTreeMap::new(),
            groups: BTreeMap::new(),
            scores: RoleScores::default(),
            missing: MissingInfo::new(),
            synthesized: BTreeMap::new(),
            override_reported: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRow {
    pub source_id: SourceId,
    pub model_name: String,
    pub vendor_hint: Option<String>,
    pub fields: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub synthesized_from: Option<String>,
}
