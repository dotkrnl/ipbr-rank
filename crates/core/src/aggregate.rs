use crate::model::MissingInfo;
use std::collections::BTreeMap;

const EPS: f64 = 1e-12;
pub const SHRINK_TARGET: f64 = 50.0;

/// Coverage threshold around which we transition from shrink-to-50 to
/// trusting the present-weight mean directly. The transition uses a smooth
/// step over a ±0.10 band (0.60 → 0.80) so that a tiny change in coverage
/// cannot cause a large discontinuous jump in the group score.
const FULL_COVERAGE_TRUST_THRESHOLD: f64 = 0.70;
const TRUST_TRANSITION_WIDTH: f64 = 0.20;

pub fn missing_safe_avg(
    metrics: &BTreeMap<String, f64>,
    weights: &BTreeMap<String, f64>,
    missing_info: &mut MissingInfo,
    prefix: &str,
) -> f64 {
    let total_weight: f64 = weights.values().copied().filter(|w| w.is_finite()).sum();
    if total_weight.abs() < EPS {
        return SHRINK_TARGET;
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
        return SHRINK_TARGET;
    }

    let present_mean = weighted_sum / present_weight;
    let w_present = present_weight / total_weight;

    // Old hard-step formula: shrink toward 50 proportional to missing weight.
    let shrink_value = present_mean * w_present + SHRINK_TARGET * (1.0 - w_present);

    // Smooth transition from full-shrink to no-shrink across
    // [threshold - width/2, threshold + width/2].
    let t = ((w_present - (FULL_COVERAGE_TRUST_THRESHOLD - TRUST_TRANSITION_WIDTH / 2.0))
        / TRUST_TRANSITION_WIDTH)
        .clamp(0.0, 1.0);
    let smooth_t = t * t * (3.0 - 2.0 * t);
    let value = shrink_value * (1.0 - smooth_t) + present_mean * smooth_t;

    value.clamp(0.0, 100.0)
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
        let v = missing_safe_avg(&metrics, &w, &mut missing, "");
        assert!((v - 50.0).abs() < 1e-9);
        assert_eq!(missing.metrics.len(), 2);
    }

    #[test]
    fn all_present_uses_weighted_mean() {
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 80.0), ("b".to_string(), 60.0)]
            .into_iter()
            .collect();
        let w = weights(&[("a", 0.5), ("b", 0.5)]);
        let mut missing = MissingInfo::new();
        let v = missing_safe_avg(&metrics, &w, &mut missing, "");
        assert!((v - 70.0).abs() < 1e-9);
        assert!(missing.metrics.is_empty());
    }

    #[test]
    fn sparse_coverage_shrinks_toward_50() {
        // Only 50 % of weight present — below the 0.70 trust threshold, so
        // the result is still pulled toward the 50-baseline.
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();
        let w = weights(&[("a", 0.5), ("b", 0.5)]);
        let mut missing = MissingInfo::new();
        let v = missing_safe_avg(&metrics, &w, &mut missing, "g/");
        assert!((v - 75.0).abs() < 1e-9, "expected 75, got {v}");
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
        let v = missing_safe_avg(&metrics, &w, &mut missing, "");
        // Present weighted mean: (0.4*80 + 0.4*100) / 0.8 = 90.
        assert!((v - 90.0).abs() < 1e-9, "expected 90, got {v}");
        assert!(missing.metrics.contains("c"));
    }

    #[test]
    fn nan_metric_treated_as_missing() {
        // 50 % of weight present — same shrink behavior as the sparse case.
        let metrics: BTreeMap<String, f64> = [("a".to_string(), f64::NAN), ("b".to_string(), 60.0)]
            .into_iter()
            .collect();
        let w = weights(&[("a", 0.5), ("b", 0.5)]);
        let mut missing = MissingInfo::new();
        let v = missing_safe_avg(&metrics, &w, &mut missing, "");
        assert!((v - 55.0).abs() < 1e-9);
        assert!(missing.metrics.contains("a"));
    }

    #[test]
    fn empty_weights_returns_50() {
        let metrics: BTreeMap<String, f64> = [("a".to_string(), 100.0)].into_iter().collect();
        let w: BTreeMap<String, f64> = BTreeMap::new();
        let mut missing = MissingInfo::new();
        let v = missing_safe_avg(&metrics, &w, &mut missing, "");
        assert!((v - 50.0).abs() < 1e-9);
    }
}
