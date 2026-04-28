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
pub struct Coefficients {
    pub ai_stupid_perspective_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub group_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub final_score_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub reviewer_reservation: BTreeMap<String, f64>,
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
        assert_eq!(c.reviewer_reservation.len(), 3);
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
}
