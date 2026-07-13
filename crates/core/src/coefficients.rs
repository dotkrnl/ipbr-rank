use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Stable raw-unit anchors. When both are configured, the lower and
    /// upper anchors map to approximately 5 and 95 through an asymptotic
    /// logistic curve. They are transformed into log space when `log_scale`
    /// is true.
    #[serde(default)]
    pub anchor_low: Option<f64>,
    #[serde(default)]
    pub anchor_high: Option<f64>,
    /// Independent benchmark/source family used to collapse correlated
    /// observations and cap family influence in final role scores.
    #[serde(default)]
    pub family: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceConfig {
    #[serde(default = "default_prior_score")]
    pub prior_score: f64,
    #[serde(default = "default_direct_reliability")]
    pub direct_reliability: f64,
    #[serde(default = "default_reported_reliability")]
    pub reported_reliability: f64,
    #[serde(default = "default_conservative_synthesis_reliability")]
    pub conservative_synthesis_reliability: f64,
    #[serde(default = "default_same_series_synthesis_reliability")]
    pub same_series_synthesis_reliability: f64,
    #[serde(default = "default_stronger_successor_synthesis_reliability")]
    pub stronger_successor_synthesis_reliability: f64,
    #[serde(default = "default_provisional_min_direct")]
    pub provisional_min_direct: f64,
    #[serde(default = "default_provisional_min_families")]
    pub provisional_min_families: usize,
    #[serde(default = "default_provisional_breadth_min_direct")]
    pub provisional_breadth_min_direct: f64,
    #[serde(default = "default_provisional_breadth_min_families")]
    pub provisional_breadth_min_families: usize,
    #[serde(default = "default_max_family_weight")]
    pub max_family_weight: f64,
}

/// Metadata for the fixed raw-unit normalization anchors embedded in the
/// coefficient set. The values live on each `MetricDef`; this block makes the
/// derivation cohort and policy explicit for API consumers and audits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizationConfig {
    pub anchor_version: String,
    pub snapshot_date: String,
    pub derivation: String,
    pub low_quantile: f64,
    pub high_quantile: f64,
    #[serde(default)]
    pub reported_fallback: bool,
}

fn default_prior_score() -> f64 {
    50.0
}

fn default_direct_reliability() -> f64 {
    1.0
}

fn default_reported_reliability() -> f64 {
    0.60
}

fn default_conservative_synthesis_reliability() -> f64 {
    0.0
}

fn default_same_series_synthesis_reliability() -> f64 {
    0.0
}

fn default_stronger_successor_synthesis_reliability() -> f64 {
    0.0
}

fn default_provisional_min_direct() -> f64 {
    0.60
}

fn default_provisional_min_families() -> usize {
    3
}

fn default_provisional_breadth_min_direct() -> f64 {
    0.35
}

fn default_provisional_breadth_min_families() -> usize {
    5
}

fn default_max_family_weight() -> f64 {
    1.0
}

impl Default for EvidenceConfig {
    fn default() -> Self {
        Self {
            prior_score: default_prior_score(),
            direct_reliability: default_direct_reliability(),
            reported_reliability: default_reported_reliability(),
            conservative_synthesis_reliability: default_conservative_synthesis_reliability(),
            same_series_synthesis_reliability: default_same_series_synthesis_reliability(),
            stronger_successor_synthesis_reliability:
                default_stronger_successor_synthesis_reliability(),
            provisional_min_direct: default_provisional_min_direct(),
            provisional_min_families: default_provisional_min_families(),
            provisional_breadth_min_direct: default_provisional_breadth_min_direct(),
            provisional_breadth_min_families: default_provisional_breadth_min_families(),
            max_family_weight: default_max_family_weight(),
        }
    }
}

impl EvidenceConfig {
    pub fn prior(&self) -> f64 {
        if self.prior_score.is_finite() {
            self.prior_score.clamp(0.0, 100.0)
        } else {
            default_prior_score()
        }
    }

    pub fn reliability(&self, value: f64) -> f64 {
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
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
}

fn default_synthesis_penalty() -> f64 {
    0.15
}

fn default_override_penalty() -> f64 {
    0.0
}

impl Default for PenaltiesConfig {
    fn default() -> Self {
        Self {
            synthesis: default_synthesis_penalty(),
            override_reported: default_override_penalty(),
        }
    }
}

/// Per-source/vendor/canonical effort carve-outs. The default scoring set
/// is `default | medium | thinking | high | max/xhigh`; exceptions listed
/// here allow otherwise blocked variants to score when explicitly intended.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EffortPolicy {
    #[serde(default)]
    pub exceptions: Vec<EffortException>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EffortException {
    /// Effort name that would otherwise be blocked ("high", "max", ...).
    pub effort: String,
    /// Optional source id or prefix ending in `*`.
    #[serde(default)]
    pub source: Option<String>,
    /// Optional vendor name.
    #[serde(default)]
    pub vendor: Option<String>,
    /// Optional exact canonical id.
    #[serde(default)]
    pub canonical_id: Option<String>,
}

impl EffortPolicy {
    pub fn allows(&self, effort: &str, source_id: &str, vendor: &str, canonical_id: &str) -> bool {
        let effort_norm = effort.to_lowercase().replace(['-', '_'], " ");
        self.exceptions.iter().any(|rule| {
            if rule.effort.to_lowercase().replace(['-', '_'], " ") != effort_norm {
                return false;
            }
            if let Some(s) = &rule.source {
                if s.ends_with('*') {
                    if !source_id.starts_with(&s[..s.len() - 1]) {
                        return false;
                    }
                } else if s != source_id {
                    return false;
                }
            }
            if let Some(v) = &rule.vendor
                && v.to_lowercase() != vendor.to_lowercase()
            {
                return false;
            }
            if let Some(c) = &rule.canonical_id
                && c != canonical_id
            {
                return false;
            }
            true
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coefficients {
    #[serde(default)]
    pub ai_stupid_perspective_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub group_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub final_score_weights: BTreeMap<String, BTreeMap<String, f64>>,
    pub metrics: BTreeMap<String, MetricDef>,
    /// Composite metrics combine several already-normalized input metrics
    /// into a single derived metric using the same missing-safe weighted
    /// average as group aggregation. Composites are computed after the raw
    /// metrics are normalized and inserted into `r.metrics`, so subsequent
    /// group aggregation can consume them by name. Inputs MUST be other
    /// metrics defined in `[metrics.X]`; composites cannot reference other
    /// composites (kept simple on purpose).
    #[serde(default)]
    pub composite_metrics: BTreeMap<String, BTreeMap<String, f64>>,
    /// Composites whose weights encode source precedence. The scorer selects
    /// the highest-weight available input rather than averaging duplicate
    /// observations of the same benchmark.
    #[serde(default)]
    pub precedence_composites: BTreeSet<String>,
    #[serde(default)]
    pub synthesis: Option<SynthesisConfig>,
    #[serde(default)]
    pub penalties: Option<PenaltiesConfig>,
    #[serde(default)]
    pub aggregation: Option<AggregationConfig>,
    #[serde(default)]
    pub evidence: Option<EvidenceConfig>,
    #[serde(default)]
    pub normalization: Option<NormalizationConfig>,
    #[serde(default)]
    pub effort_policy: EffortPolicy,
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
    fn ranked_leaves(c: &Coefficients) -> BTreeSet<String> {
        fn expand(
            metric: &str,
            c: &Coefficients,
            out: &mut BTreeSet<String>,
            visiting: &mut BTreeSet<String>,
        ) {
            let Some(parts) = c
                .composite_metrics
                .get(metric)
                .filter(|_| !c.precedence_composites.contains(metric))
            else {
                out.insert(metric.to_string());
                return;
            };
            assert!(
                visiting.insert(metric.to_string()),
                "composite cycle at {metric}"
            );
            for part in parts.keys() {
                expand(part, c, out, visiting);
            }
            visiting.remove(metric);
        }

        let mut out = BTreeSet::new();
        for groups in c.final_score_weights.values() {
            for group in groups.keys() {
                let metrics = c
                    .group_weights
                    .get(group)
                    .unwrap_or_else(|| panic!("role references unknown group {group}"));
                for metric in metrics.keys() {
                    expand(metric, c, &mut out, &mut BTreeSet::new());
                }
            }
        }
        out
    }

    #[test]
    fn embedded_coefficients_parse() {
        let c = Coefficients::load_embedded().expect("coefficients.toml must parse");
        assert!(
            c.ai_stupid_perspective_weights.is_empty(),
            "AISL perspective weights should remain retired from active scoring"
        );
        assert!(c.group_weights.len() >= 8);
        assert_eq!(c.final_score_weights.len(), 4);
        assert!(
            c.metrics.len() >= 20,
            "expected >=20 metrics, got {}",
            c.metrics.len()
        );
        let evidence = c.evidence.expect("embedded evidence policy");
        assert_eq!(evidence.provisional_min_direct, 0.60);
        assert_eq!(evidence.provisional_min_families, 3);
        assert_eq!(evidence.provisional_breadth_min_direct, 0.35);
        assert_eq!(evidence.provisional_breadth_min_families, 5);
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

    #[test]
    fn every_ranked_leaf_has_fixed_anchors_and_family() {
        let c = Coefficients::load_embedded().unwrap();
        for metric in ranked_leaves(&c) {
            let def = c
                .metrics
                .get(&metric)
                .unwrap_or_else(|| panic!("ranked leaf {metric} has no definition"));
            if !c.precedence_composites.contains(&metric) {
                assert!(
                    def.anchor_low.is_some() && def.anchor_high.is_some(),
                    "ranked leaf {metric} must have fixed v2 anchors"
                );
                assert!(
                    def.anchor_low.unwrap() < def.anchor_high.unwrap(),
                    "ranked leaf {metric} anchors must be ordered"
                );
            }
            assert!(
                def.family
                    .as_deref()
                    .is_some_and(|family| !family.is_empty()),
                "ranked leaf {metric} must declare an evidence family"
            );
        }
        assert_eq!(
            c.normalization.as_ref().map(|n| n.anchor_version.as_str()),
            Some("2026-07-12.v2")
        );
    }

    #[test]
    fn operations_and_pricing_have_zero_rank_path() {
        let c = Coefficients::load_embedded().unwrap();
        let ranked = ranked_leaves(&c);
        for metric in ["OutputSpeed", "TTFT", "ContextWindow", "BlendedCost"] {
            assert!(
                !ranked.contains(metric),
                "{metric} is diagnostic-only and must never affect a role rank"
            );
        }
        assert!(
            c.final_score_weights
                .values()
                .all(|groups| groups.keys().all(|group| !group.starts_with("OPS_")))
        );
    }

    #[test]
    fn metric_group_metadata_matches_the_actual_scoring_graph() {
        fn expand(
            metric: &str,
            c: &Coefficients,
            out: &mut BTreeSet<String>,
            visiting: &mut BTreeSet<String>,
        ) {
            let Some(parts) = c
                .composite_metrics
                .get(metric)
                .filter(|_| !c.precedence_composites.contains(metric))
            else {
                out.insert(metric.to_string());
                return;
            };
            assert!(
                visiting.insert(metric.to_string()),
                "composite cycle at {metric}"
            );
            for part in parts.keys() {
                expand(part, c, out, visiting);
            }
            visiting.remove(metric);
        }

        let c = Coefficients::load_embedded().unwrap();
        let mut actual: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (group, metrics) in &c.group_weights {
            for metric in metrics.keys() {
                let mut leaves = BTreeSet::new();
                expand(metric, &c, &mut leaves, &mut BTreeSet::new());
                for leaf in leaves {
                    actual.entry(leaf).or_default().insert(group.clone());
                }
            }
        }
        for (metric, def) in &c.metrics {
            let declared: BTreeSet<_> = def.groups.iter().cloned().collect();
            let reached = actual.get(metric).cloned().unwrap_or_default();
            assert_eq!(
                declared, reached,
                "{metric} groups metadata must mirror its flattened group path"
            );
        }
    }

    #[test]
    fn direct_creative_and_judge_signals_replace_old_duplicates() {
        let c = Coefficients::load_embedded().unwrap();
        let ranked = ranked_leaves(&c);
        assert!(ranked.contains("EQBenchCreativeWriting"));
        assert!(ranked.contains("EQBenchJudgemark"));
        assert!(!ranked.contains("LMArenaCreativeOrOpenEnded"));
        assert!(!ranked.contains("ArtificialAnalysisReasoning"));
        assert!(!ranked.contains("GPQA_HLE_Reasoning"));
    }
}
