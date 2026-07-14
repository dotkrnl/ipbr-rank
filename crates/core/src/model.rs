use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MissingInfo {
    pub metrics: BTreeSet<MetricKey>,
    pub groups_shrunk: BTreeSet<GroupKey>,
}

/// Evidence coverage carried from leaf benchmark observations through
/// composites, groups, and role scores. The evidence-class shares are nominal
/// path weights: `direct` and `missing` sum to one (within floating-point
/// tolerance). `effective` applies the configured reliability to the direct
/// share. Every scored observation is a direct same-product measurement.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidenceCoverage {
    #[serde(default)]
    pub direct: f64,
    #[serde(default)]
    pub missing: f64,
    #[serde(default)]
    pub effective: f64,
    /// Distinct configured benchmark families with direct evidence.
    #[serde(default)]
    pub direct_families: BTreeSet<String>,
    #[serde(default)]
    pub family_count: usize,
    /// Direct coverage after retaining only role-relevant `core` metrics and
    /// renormalizing their configured weights. Supplemental and historical
    /// sources cannot burden this denominator.
    #[serde(default)]
    pub core_direct: f64,
    #[serde(default)]
    pub core_direct_families: BTreeSet<String>,
    #[serde(default)]
    pub core_family_count: usize,
    /// Direct, role-relevant retired benchmark families. These can establish
    /// historical breadth but never contribute to current numeric scores.
    #[serde(default)]
    pub historical_direct_families: BTreeSet<String>,
    #[serde(default)]
    pub historical_family_count: usize,
    /// The first explicit eligibility path satisfied by this role.
    #[serde(default)]
    pub qualification_path: EligibilityQualificationPath,
    /// Role scores that fail every configured current, core, and historical-
    /// breadth qualification path remain computable, but are provisional.
    #[serde(default)]
    pub provisional: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EligibilityQualificationPath {
    Standard,
    Breadth,
    CoreStandard,
    CoreCorroborated,
    CoreBreadth,
    HistoricalBreadth,
    #[default]
    Unqualified,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSummary {
    #[serde(default)]
    pub groups: BTreeMap<GroupKey, EvidenceCoverage>,
    #[serde(default)]
    pub roles: BTreeMap<String, EvidenceCoverage>,
}

impl MissingInfo {
    pub fn new() -> Self {
        Self::default()
    }
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
    /// Map a free-form vendor label (case-insensitive, with the handful of
    /// spelling variants sources emit) onto a canonical vendor. Unknown labels
    /// are preserved verbatim as `Other`.
    pub fn from_label(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => Self::Openai,
            "anthropic" => Self::Anthropic,
            "google" => Self::Google,
            "moonshot" | "moonshotai" => Self::Moonshot,
            "zai" | "z-ai" | "z.ai" => Self::Zai,
            "xai" => Self::Xai,
            "alibaba" => Self::Alibaba,
            "deepseek" => Self::Deepseek,
            "mistral" => Self::Mistral,
            "meta" => Self::Meta,
            "minimax" => Self::Minimax,
            "nvidia" => Self::Nvidia,
            "baidu" => Self::Baidu,
            "tencent" => Self::Tencent,
            "inclusionai" => Self::Inclusionai,
            "xiaomi" => Self::Xiaomi,
            other => Self::Other(other.to_string()),
        }
    }

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

pub type SourceId = String;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleScores {
    pub i_raw: f64,
    pub p_raw: f64,
    pub b_raw: f64,
    pub r: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub canonical_id: String,
    pub display_name: String,
    pub vendor: Vendor,
    pub aliases: BTreeSet<String>,
    pub sources: BTreeSet<SourceId>,
    pub raw_metrics: BTreeMap<MetricKey, f64>,
    pub metrics: BTreeMap<MetricKey, f64>,
    pub groups: BTreeMap<GroupKey, f64>,
    pub scores: RoleScores,
    pub missing: MissingInfo,
    /// Metrics whose winning observation came from the checked-in curated
    /// override source. These are direct same-product observations, but the
    /// marker preserves lower precedence than a native public source and
    /// keeps their citations attached to the winning value.
    #[serde(default)]
    pub curated_overrides: BTreeSet<MetricKey>,
    /// Winning source for each raw metric after evidence and effort
    /// precedence have been applied.
    #[serde(default)]
    pub metric_sources: BTreeMap<MetricKey, SourceId>,
    /// Human-readable citations/notes emitted by the winning observation for a
    /// metric, whether a manual override or a native source attaching
    /// provenance such as automatic product routing. Serialized as `citation`.
    #[serde(default)]
    pub metric_citations: BTreeMap<MetricKey, String>,
    #[serde(default)]
    pub evidence: EvidenceSummary,
}

impl ModelRecord {
    pub fn new(canonical_id: String, display_name: String, vendor: Vendor) -> Self {
        Self {
            canonical_id,
            display_name,
            vendor,
            aliases: BTreeSet::new(),
            sources: BTreeSet::new(),
            raw_metrics: BTreeMap::new(),
            metrics: BTreeMap::new(),
            groups: BTreeMap::new(),
            scores: RoleScores::default(),
            missing: MissingInfo::new(),
            curated_overrides: BTreeSet::new(),
            metric_sources: BTreeMap::new(),
            metric_citations: BTreeMap::new(),
            evidence: EvidenceSummary::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRow {
    pub source_id: SourceId,
    pub model_name: String,
    pub vendor_hint: Option<String>,
    pub fields: BTreeMap<String, serde_json::Value>,
}
