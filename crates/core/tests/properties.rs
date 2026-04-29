use ipbr_core::MissingInfo;
use ipbr_core::aggregate::missing_safe_avg;
use ipbr_core::coefficients::AggregationConfig;
use proptest::prelude::*;
use std::collections::BTreeMap;

fn build_weights(values: &[f64]) -> BTreeMap<String, f64> {
    values
        .iter()
        .enumerate()
        .map(|(i, w)| (format!("m{i}"), *w))
        .collect()
}

fn build_metrics(values: &[Option<f64>]) -> BTreeMap<String, f64> {
    values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|x| (format!("m{i}"), x)))
        .collect()
}

proptest! {
    #[test]
    fn decreasing_present_metric_cannot_increase_group(
        weights in proptest::collection::vec(0.05f64..1.0, 1..6),
        scores in proptest::collection::vec(0.0f64..100.0, 1..6),
        delta in 0.5f64..40.0,
        target_idx in 0usize..6,
    ) {
        let n = weights.len().min(scores.len());
        prop_assume!(n > 0);
        let weights = &weights[..n];
        let mut scores: Vec<f64> = scores[..n].to_vec();
        let i = target_idx % n;
        let metric_values: Vec<Option<f64>> = scores.iter().map(|v| Some(*v)).collect();

        let w_map = build_weights(weights);
        let m_map = build_metrics(&metric_values);
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (before, _) = missing_safe_avg(&m_map, &w_map, &mut missing, "", &cfg);

        scores[i] = (scores[i] - delta).max(0.0);
        let metric_values_after: Vec<Option<f64>> = scores.iter().map(|v| Some(*v)).collect();
        let m_map_after = build_metrics(&metric_values_after);
        let mut missing_after = MissingInfo::new();
        let (after, _) = missing_safe_avg(&m_map_after, &w_map, &mut missing_after, "", &cfg);
        prop_assert!(
            after <= before + 1e-9,
            "decreasing a metric raised group score: before={before}, after={after}"
        );
    }

    #[test]
    fn group_score_is_key_order_invariant(
        weights in proptest::collection::vec(0.1f64..1.0, 2..6),
        scores in proptest::collection::vec(0.0f64..100.0, 2..6),
    ) {
        let n = weights.len().min(scores.len());
        prop_assume!(n >= 2);
        let weights = &weights[..n];
        let scores = &scores[..n];

        let w1: BTreeMap<String, f64> = weights
            .iter()
            .enumerate()
            .map(|(i, w)| (format!("m{i}"), *w))
            .collect();
        let m1: BTreeMap<String, f64> = scores
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("m{i}"), *v))
            .collect();

        // Permuted: zzz prefix forces different traversal? BTreeMap is always
        // sorted by key, but renaming keys forces a different ordering.
        let w2: BTreeMap<String, f64> = weights
            .iter()
            .enumerate()
            .map(|(i, w)| (format!("z{i}"), *w))
            .collect();
        let m2: BTreeMap<String, f64> = scores
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("z{i}"), *v))
            .collect();

        let mut missing1 = MissingInfo::new();
        let mut missing2 = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v1, _) = missing_safe_avg(&m1, &w1, &mut missing1, "", &cfg);
        let (v2, _) = missing_safe_avg(&m2, &w2, &mut missing2, "", &cfg);
        prop_assert!((v1 - v2).abs() < 1e-9);
    }

    #[test]
    fn fully_missing_group_yields_50(
        weights in proptest::collection::vec(0.05f64..1.0, 1..6),
    ) {
        let w_map = build_weights(&weights);
        let m_map: BTreeMap<String, f64> = BTreeMap::new();
        let mut missing = MissingInfo::new();
        let cfg = AggregationConfig::default();
        let (v, _) = missing_safe_avg(&m_map, &w_map, &mut missing, "", &cfg);
        prop_assert!((v - 50.0).abs() < 1e-9);
    }
}
