use crate::coefficients::AggregationConfig;
use crate::model::MissingInfo;
use std::collections::BTreeMap;

const EPS: f64 = 1e-12;
pub const SHRINK_TARGET: f64 = 50.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AggregateResult {
    /// `None` means every positively weighted input was absent. Callers that
    /// must expose a numeric compatibility value may substitute `prior` while
    /// still preserving the absence bit.
    pub value: Option<f64>,
    pub observed_coverage: f64,
    pub shrunk: bool,
}

pub fn shrink_coverage_cutoff(cfg: &AggregationConfig) -> f64 {
    (cfg.trust_threshold + cfg.trust_transition_width / 2.0).clamp(0.0, 1.0)
}

/// Continuous prior-replacement aggregation. Every missing nominal weight is
/// assigned the prior; there is no threshold at which the missing penalty
/// suddenly disappears. Consequently, observing a below-prior result cannot
/// raise a score merely by crossing a coverage threshold.
pub fn prior_replacement_avg(
    metrics: &BTreeMap<String, f64>,
    weights: &BTreeMap<String, f64>,
    missing_info: &mut MissingInfo,
    prefix: &str,
    prior: f64,
    cfg: &AggregationConfig,
) -> AggregateResult {
    let prior = if prior.is_finite() {
        prior.clamp(0.0, 100.0)
    } else {
        SHRINK_TARGET
    };
    let total_weight: f64 = weights
        .values()
        .copied()
        .filter(|w| w.is_finite() && *w > 0.0)
        .sum();
    if total_weight.abs() < EPS {
        return AggregateResult {
            value: None,
            observed_coverage: 0.0,
            shrunk: true,
        };
    }

    let mut present_weight = 0.0;
    let mut weighted_sum = 0.0;
    for (key, w) in weights {
        if !w.is_finite() || *w <= 0.0 {
            continue;
        }
        match metrics.get(key) {
            Some(v) if v.is_finite() => {
                present_weight += w;
                weighted_sum += w * v;
            }
            _ => {
                missing_info.metrics.insert(format!("{prefix}{key}"));
            }
        }
    }

    let w_present = present_weight / total_weight;
    let shrunk = w_present < shrink_coverage_cutoff(cfg);
    let value = (weighted_sum + prior * (total_weight - present_weight)) / total_weight;
    AggregateResult {
        value: (present_weight >= EPS).then(|| value.clamp(0.0, 100.0)),
        observed_coverage: w_present.clamp(0.0, 1.0),
        shrunk,
    }
}

/// Backwards-compatible numeric wrapper around [`prior_replacement_avg`].
/// Fully absent aggregates still return 50 here, but callers that need to
/// propagate missingness (notably composite scoring) use the richer API.
pub fn missing_safe_avg(
    metrics: &BTreeMap<String, f64>,
    weights: &BTreeMap<String, f64>,
    missing_info: &mut MissingInfo,
    prefix: &str,
    cfg: &AggregationConfig,
) -> (f64, bool) {
    let result = prior_replacement_avg(metrics, weights, missing_info, prefix, SHRINK_TARGET, cfg);
    (result.value.unwrap_or(SHRINK_TARGET), result.shrunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn all_missing_returns_50() {
        let metrics = BTreeMap::new();
        let w = weights(&[("a", 0.5), ("b", 0.5)]);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "", &cfg);
        assert!((v - 50.0).abs() < 1e-9);
        assert!(shrunk, "all missing should be marked shrunk");
        assert_eq!(missing.metrics.len(), 2);
    }

    #[test]
    fn all_present_uses_weighted_mean() {
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 80.0), ("b".to_string(), 60.0)]
            .into_iter()
            .collect();
        let w = weights(&[("a", 0.5), ("b", 0.5)]);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "", &cfg);
        assert!((v - 70.0).abs() < 1e-9);
        assert!(!shrunk, "full coverage should not be shrunk");
        assert!(missing.metrics.is_empty());
    }

    #[test]
    fn sparse_coverage_shrinks_toward_50() {
        // Only 50 % of weight present — below the 0.70 trust threshold, so
        // the result is still pulled toward the 50-baseline.
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();
        let w = weights(&[("a", 0.5), ("b", 0.5)]);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "g/", &cfg);
        assert!((v - 75.0).abs() < 1e-9, "expected 75, got {v}");
        assert!(shrunk, "50 % coverage should be shrunk");
        assert!(missing.metrics.contains("g/b"));
    }

    #[test]
    fn near_full_coverage_still_replaces_missing_weight_with_prior() {
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 80.0), ("b".to_string(), 100.0)]
            .into_iter()
            .collect();
        let w = weights(&[("a", 0.4), ("b", 0.4), ("c", 0.2)]);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "", &cfg);
        // Nominal score: .4*80 + .4*100 + .2*50 = 82.
        assert!((v - 82.0).abs() < 1e-9, "expected 82, got {v}");
        assert!(!shrunk, "80 % coverage should not be shrunk");
        assert!(missing.metrics.contains("c"));
    }

    #[test]
    fn shrunk_flag_uses_configured_transition_ceiling() {
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 80.0)].into_iter().collect();
        let w = weights(&[("a", 0.65), ("b", 0.35)]);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig {
            trust_threshold: 0.50,
            trust_transition_width: 0.10,
        };

        let (_v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "", &cfg);

        assert!(
            !shrunk,
            "65% coverage is above the configured 55% transition ceiling"
        );
    }

    #[test]
    fn nan_metric_treated_as_missing() {
        // 50 % of weight present — same shrink behavior as the sparse case.
        let metrics: BTreeMap<String, f64> = [("a".to_string(), f64::NAN), ("b".to_string(), 60.0)]
            .into_iter()
            .collect();
        let w = weights(&[("a", 0.5), ("b", 0.5)]);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "", &cfg);
        assert!((v - 55.0).abs() < 1e-9);
        assert!(shrunk, "NaN metric should count as missing");
        assert!(missing.metrics.contains("a"));
    }

    #[test]
    fn empty_weights_returns_50() {
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();
        let w: BTreeMap<String, f64> = BTreeMap::new();
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "", &cfg);
        assert!((v - 50.0).abs() < 1e-9);
        assert!(shrunk, "empty weights should be marked shrunk");
    }

    #[test]
    fn rich_result_preserves_fully_missing_state() {
        let metrics = BTreeMap::new();
        let w = weights(&[("a", 1.0)]);
        let mut missing = MissingInfo::new();
        let result = prior_replacement_avg(
            &metrics,
            &w,
            &mut missing,
            "",
            50.0,
            &AggregationConfig::default(),
        );
        assert_eq!(result.value, None);
        assert_eq!(result.observed_coverage, 0.0);
    }

    #[test]
    fn adding_below_prior_observation_cannot_raise_score() {
        let w = weights(&[("a", 0.7), ("b", 0.1), ("c", 0.2)]);
        let before: BTreeMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();
        let after: BTreeMap<String, f64> = [("a".to_string(), 100.0), ("b".to_string(), 0.0)]
            .into_iter()
            .collect();
        let cfg = AggregationConfig::default();
        let mut missing = MissingInfo::new();
        let (before, _) = missing_safe_avg(&before, &w, &mut missing, "", &cfg);
        let mut missing = MissingInfo::new();
        let (after, _) = missing_safe_avg(&after, &w, &mut missing, "", &cfg);
        assert!(after < before, "before={before}, after={after}");
    }
}
