use ipbr_core::MissingInfo;
use ipbr_core::aggregate::missing_safe_avg;
use ipbr_core::coefficients::{AggregationConfig, Coefficients};
use ipbr_core::ingest::ingest_rows;
use ipbr_core::model::{ModelRecord, RawRow, Vendor};
use proptest::prelude::*;
use serde_json::json;
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

#[test]
fn real_rows_always_win_over_synthesized_and_penalty_is_bounded() {
    // Deterministic property: for every scored metric, a real row should
    // clobber a synthesized row regardless of input order, and a synthesized
    // row should be pulled toward 50 by no more than the configured penalty.
    let coef = Coefficients::load_embedded().unwrap();
    let penalty = coef.penalties.as_ref().map(|p| p.synthesis).unwrap_or(0.15);

    let make_record = |canonical: &str| {
        let mut r = ModelRecord::new(
            canonical.to_string(),
            canonical.to_string(),
            Vendor::Other("test".to_string()),
        );
        r.aliases.insert(canonical.to_string());
        r
    };

    let make_real = |metric: &str, value: f64| {
        let mut fields: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        fields.insert(metric.to_string(), json!(value));
        RawRow {
            source_id: "source".to_string(),
            model_name: "target".to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
            synthesis_category: None,
        }
    };

    let make_synth = |metric: &str, value: f64| {
        let mut fields: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        fields.insert(metric.to_string(), json!(value));
        RawRow {
            source_id: "source".to_string(),
            model_name: "target".to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: Some("donor".to_string()),
            synthesis_category: Some(ipbr_core::SynthesisCategory::Conservative),
        }
    };

    for metric in [
        "TerminalBench",
        "LMArenaText",
        "SWEBenchVerified",
        "MMLUPro",
    ] {
        // synth then real
        let mut records = vec![make_record("target")];
        let rows = vec![make_synth(metric, 100.0), make_real(metric, 80.0)];
        ingest_rows(&mut records, rows);
        let val = records[0].raw_metrics.get(metric).copied().unwrap();
        assert!(
            (val - 80.0).abs() < 1e-9,
            "real row should overwrite synthesized for {metric}"
        );
        assert!(
            !records[0].synthesized.contains_key(metric),
            "synthesized provenance should be cleared for {metric}"
        );

        // real then synth
        let mut records = vec![make_record("target")];
        let rows = vec![make_real(metric, 80.0), make_synth(metric, 100.0)];
        ingest_rows(&mut records, rows);
        let val = records[0].raw_metrics.get(metric).copied().unwrap();
        assert!(
            (val - 80.0).abs() < 1e-9,
            "real row should win regardless of order for {metric}"
        );

        // synth-only: normalized value should be pulled toward 50 by
        // at most the configured synthesis penalty.
        let mut records = vec![make_record("low"), make_record("target")];
        records[0].raw_metrics.insert(metric.to_string(), 0.0);
        records[1].raw_metrics.insert(metric.to_string(), 100.0);
        records[1].synthesized.insert(
            metric.to_string(),
            ipbr_core::SynthesisProvenance {
                source_id: "source".to_string(),
                from: "donor".to_string(),
                category: ipbr_core::SynthesisCategory::Conservative,
            },
        );
        ipbr_core::compute_scores_with(&mut records, &coef);
        let synth_norm = records[1].metrics.get(metric).copied().unwrap_or(50.0);
        // normalized direct top = 100; conservative synth = 100*(1-p)+50*p
        let expected = 100.0 * (1.0 - penalty) + 50.0 * penalty;
        assert!(
            (synth_norm - expected).abs() < 0.5,
            "synthesized {metric} should be pulled toward 50 by ~{penalty}, got {synth_norm}"
        );
    }
}
