use crate::coefficients::AggregationConfig;
use crate::model::MissingInfo;
use std::collections::BTreeMap;

const EPS: f64 = 1e-12;
pub const SHRINK_TARGET: f64 = 50.0;

pub fn shrink_coverage_cutoff(cfg: &AggregationConfig) -> f64 {
    (cfg.trust_threshold + cfg.trust_transition_width / 2.0).clamp(0.0, 1.0)
}

/// Returns the missing-safe weighted average and whether the group was
/// shrunk (present weight below the configured transition ceiling).
pub fn missing_safe_avg(
    metrics: &BTreeMap<String, f64>,
    weights: &BTreeMap<String, f64>,
    missing_info: &mut MissingInfo,
    prefix: &str,
    cfg: &AggregationConfig,
) -> (f64, bool) {
    let total_weight: f64 = weights.values().copied().filter(|w| w.is_finite()).sum();
    if total_weight.abs() < EPS {
        return (SHRINK_TARGET, true);
    }

    let mut present_weight = 0.0;
    let mut weighted_sum = 0.0;
    for (key, w) in weights {
        if !w.is_finite() {
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

    if present_weight.abs() < EPS {
        return (SHRINK_TARGET, true);
    }

    let present_mean = weighted_sum / present_weight;
    let w_present = present_weight / total_weight;

    // Old hard-step formula: shrink toward 50 proportional to missing weight.
    let shrink_value = present_mean * w_present + SHRINK_TARGET * (1.0 - w_present);

    // Smooth transition from full-shrink to no-shrink across
    // [threshold - width/2, threshold + width/2].
    let t = ((w_present - (cfg.trust_threshold - cfg.trust_transition_width / 2.0))
        / cfg.trust_transition_width)
        .clamp(0.0, 1.0);
    let smooth_t = t * t * (3.0 - 2.0 * t);
    let value = shrink_value * (1.0 - smooth_t) + present_mean * smooth_t;

    let shrunk = w_present < shrink_coverage_cutoff(cfg);
    (value.clamp(0.0, 100.0), shrunk)
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
    fn near_full_coverage_uses_present_mean() {
        // 80 % of weight present — at or above the trust threshold, so we
        // skip the shrink and report the present-weight mean directly.
        // Without this, models missing one minor source would be pulled
        // toward 50 even though the bulk of their coverage is intact.
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 80.0), ("b".to_string(), 100.0)]
            .into_iter()
            .collect();
        let w = weights(&[("a", 0.4), ("b", 0.4), ("c", 0.2)]);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, shrunk) = missing_safe_avg(&metrics, &w, &mut missing, "", &cfg);
        // Present weighted mean: (0.4*80 + 0.4*100) / 0.8 = 90.
        assert!((v - 90.0).abs() < 1e-9, "expected 90, got {v}");
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
}
