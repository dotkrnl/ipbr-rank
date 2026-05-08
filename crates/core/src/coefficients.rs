use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricTransform {
    Percentile,
    AsScore,
    /// Two-piece linear normalization for operational metrics where users
    /// don't perceive small differences linearly. The top 80 % of the
    /// population maps into a narrow 70-100 band (mild differentiation);
    /// only the bottom 20 % drops sharply into 0-70. Useful when "slow but
    /// usable" should look almost as good as "fast" but "extremely slow"
    /// should be visibly penalized.
    TailPenalty,
}

fn default_transform() -> MetricTransform {
    MetricTransform::AsScore
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDef {
    pub higher_better: bool,
    #[serde(default)]
    pub log_scale: bool,
    pub groups: Vec<String>,
    #[serde(default = "default_transform")]
    pub transform: MetricTransform,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisConfig {
    #[serde(default = "default_per_source_cap")]
    pub per_source_cap: f64,
    #[serde(default = "default_per_model_cap")]
    pub per_model_cap: f64,
}

fn default_per_source_cap() -> f64 {
    0.30
}

fn default_per_model_cap() -> f64 {
    0.50
}

impl Default for SynthesisConfig {
    fn default() -> Self {
        Self {
            per_source_cap: default_per_source_cap(),
            per_model_cap: default_per_model_cap(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationConfig {
    #[serde(default = "default_trust_threshold")]
    pub trust_threshold: f64,
    #[serde(default = "default_trust_width")]
    pub trust_transition_width: f64,
}

fn default_trust_threshold() -> f64 {
    0.70
}

fn default_trust_width() -> f64 {
    0.20
}

impl Default for AggregationConfig {
    fn default() -> Self {
        Self {
            trust_threshold: default_trust_threshold(),
            trust_transition_width: default_trust_width(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenaltiesConfig {
    #[serde(default = "default_synthesis_penalty")]
    pub synthesis: f64,
    #[serde(default = "default_override_penalty")]
    pub override_reported: f64,
    #[serde(default = "default_canary_deadband")]
    pub canary_health_deadband: f64,
    #[serde(default = "default_canary_floor")]
    pub canary_health_floor: f64,
    #[serde(default = "default_canary_max")]
    pub canary_max_role_penalty: f64,
}

fn default_synthesis_penalty() -> f64 {
    0.15
}

fn default_override_penalty() -> f64 {
    0.10
}

fn default_canary_deadband() -> f64 {
    60.0
}

fn default_canary_floor() -> f64 {
    20.0
}

fn default_canary_max() -> f64 {
    6.0
}

impl Default for PenaltiesConfig {
    fn default() -> Self {
        Self {
            synthesis: default_synthesis_penalty(),
            override_reported: default_override_penalty(),
            canary_health_deadband: default_canary_deadband(),
            canary_health_floor: default_canary_floor(),
            canary_max_role_penalty: default_canary_max(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coefficients {
    pub ai_stupid_perspective_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub group_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub final_score_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub metrics: BTreeMap<String, MetricDef>,
    /// Composite metrics combine several already-normalized input metrics
    /// into a single derived metric using the same missing-safe weighted
    /// average as group aggregation. Composites are computed after the raw
    /// metrics are normalized and inserted into `r.metrics`, so subsequent
    /// group aggregation can reference them by name. Inputs MUST be other
    /// metrics defined in `[metrics.X]`; composites cannot reference other
    /// composites (kept simple on purpose).
    #[serde(default)]
    pub composite_metrics: BTreeMap<String, BTreeMap<String, f64>>,
    #[serde(default)]
    pub synthesis: Option<SynthesisConfig>,
    #[serde(default)]
    pub penalties: Option<PenaltiesConfig>,
    #[serde(default)]
    pub aggregation: Option<AggregationConfig>,
}

const EMBEDDED_COEFFICIENTS: &str = include_str!("../../../data/coefficients.toml");

impl Coefficients {
    pub fn load_embedded() -> Result<Self, toml::de::Error> {
        toml::from_str(EMBEDDED_COEFFICIENTS)
    }

    pub fn load_from_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_coefficients_parse() {
        let c = Coefficients::load_embedded().expect("coefficients.toml must parse");
        assert_eq!(c.ai_stupid_perspective_weights.len(), 4);
        assert_eq!(c.group_weights.len(), 8);
        assert_eq!(c.final_score_weights.len(), 4);
        assert!(
            c.metrics.len() >= 20,
            "expected >=20 metrics, got {}",
            c.metrics.len()
        );
    }

    #[test]
    fn final_score_weights_sum_to_one() {
        let c = Coefficients::load_embedded().unwrap();
        for (role, weights) in &c.final_score_weights {
            let sum: f64 = weights.values().sum();
            assert!(
                (sum - 1.0).abs() < 1e-9,
                "{role} weights sum to {sum}, expected 1.0"
            );
        }
        assert!(
            c.final_score_weights["R"].contains_key("LM_ARENA_REVIEW_PROXY"),
            "R should use the renamed LM Arena review proxy group"
        );
        assert!(
            !c.final_score_weights["R"].contains_key("JUDGE"),
            "R should no longer expose the old JUDGE group"
        );
    }

    #[test]
    fn perspective_weights_sum_to_one() {
        let c = Coefficients::load_embedded().unwrap();
        for (perspective, weights) in &c.ai_stupid_perspective_weights {
            let sum: f64 = weights.values().sum();
            assert!(
                (sum - 1.0).abs() < 1e-9,
                "{perspective} weights sum to {sum}, expected 1.0"
            );
        }
    }

    #[test]
    fn group_weights_sum_to_one() {
        let c = Coefficients::load_embedded().unwrap();
        for (group, weights) in &c.group_weights {
            let sum: f64 = weights.values().sum();
            assert!(
                (sum - 1.0).abs() < 1e-9,
                "{group} weights sum to {sum}, expected 1.0"
            );
        }
    }

    #[test]
    fn composite_metrics_weights_sum_to_one() {
        let c = Coefficients::load_embedded().unwrap();
        for (name, weights) in &c.composite_metrics {
            let sum: f64 = weights.values().sum();
            assert!(
                (sum - 1.0).abs() < 1e-9,
                "{name} composite weights sum to {sum}, expected 1.0"
            );
        }
    }
}
