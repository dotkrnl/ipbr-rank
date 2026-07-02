use crate::aggregate::missing_safe_avg;
use crate::coefficients::{Coefficients, MetricTransform, PenaltiesConfig};
use crate::model::ModelRecord;
use crate::normalize::{as_score_0_100, robust_norm, tail_penalty_norm};
use std::collections::BTreeMap;

const ROLE_KEYS: &[&str] = &["I_raw", "P_raw", "B_raw", "R"];

pub fn compute_scores(records: &mut [ModelRecord]) {
    let coef = Coefficients::load_embedded().expect("embedded coefficients are valid");
    compute_scores_with(records, &coef);
}

pub fn compute_scores_with(records: &mut [ModelRecord], coef: &Coefficients) {
    let penalties = coef.penalties.clone().unwrap_or_default();
    let aggregation = coef.aggregation.clone().unwrap_or_default();
    normalize_population(records, coef, &penalties);
    compute_composite_metrics(records, coef, &aggregation);
    aggregate_groups(records, coef, &aggregation);
    compute_role_scores(records, coef, &aggregation);
}

/// Compute each composite metric as a missing-safe weighted average of its
/// input metrics (post-normalization). The result is written into `r.metrics`
/// under the composite's name so that group aggregation can consume it as if
/// it were a regular metric. Provenance for missing inputs is recorded with a
/// `<composite>/` prefix in `MissingInfo` so it doesn't collide with group
/// missingness.
fn compute_composite_metrics(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    aggregation: &crate::coefficients::AggregationConfig,
) {
    for r in records.iter_mut() {
        for (name, weights) in &coef.composite_metrics {
            let prefix = format!("{name}/");
            let (value, _shrunk) =
                missing_safe_avg(&r.metrics, weights, &mut r.missing, &prefix, aggregation);
            r.metrics.insert(name.clone(), value);
        }
    }
}

fn normalize_population(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    penalties: &PenaltiesConfig,
) {
    for (metric_key, def) in &coef.metrics {
        let all_pop: Vec<f64> = records
            .iter()
            .filter_map(|r| r.raw_metrics.get(metric_key).copied())
            .filter(|v| v.is_finite())
            .collect();
        let direct_pop: Vec<f64> = records
            .iter()
            .filter(|r| {
                !r.synthesized.contains_key(metric_key) && !r.override_reported.contains(metric_key)
            })
            .filter_map(|r| r.raw_metrics.get(metric_key).copied())
            .filter(|v| v.is_finite())
            .collect();
        let pop = if direct_pop.len() >= 2 {
            &direct_pop
        } else {
            &all_pop
        };
        for r in records.iter_mut() {
            let raw = match r.raw_metrics.get(metric_key) {
                Some(v) if v.is_finite() => *v,
                _ => continue,
            };
            let normed = match def.transform {
                MetricTransform::AsScore => as_score_0_100(raw),
                MetricTransform::Percentile => {
                    robust_norm(raw, pop, def.higher_better, def.log_scale)
                }
                MetricTransform::TailPenalty => {
                    tail_penalty_norm(raw, pop, def.higher_better, def.log_scale)
                }
            };
            if let Some(v) = normed {
                let final_value = if let Some(provenance) = r.synthesized.get(metric_key) {
                    // Pull conservative synthesized values toward the 50
                    // baseline so they act as a softer signal than direct
                    // measurements. Same-series forward fills are explicit
                    // version-advance priors and carry no synthesis pull.
                    let penalty = provenance.category.penalty(penalties.synthesis);
                    v * (1.0 - penalty) + 50.0 * penalty
                } else {
                    v
                };
                let final_value = if r.override_reported.contains(metric_key) {
                    // Manual overrides are public, cited reported values. The
                    // configurable override penalty defaults to zero, but the
                    // hook remains for coefficient experiments.
                    final_value * (1.0 - penalties.override_reported)
                        + 50.0 * penalties.override_reported
                } else {
                    final_value
                };
                r.metrics.insert(metric_key.clone(), final_value);
            }
        }
    }
}

fn aggregate_groups(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    aggregation: &crate::coefficients::AggregationConfig,
) {
    for r in records.iter_mut() {
        for (group_key, weights) in &coef.group_weights {
            let prefix = format!("{group_key}/");
            let (v, shrunk) =
                missing_safe_avg(&r.metrics, weights, &mut r.missing, &prefix, aggregation);
            r.groups.insert(group_key.clone(), v);
            if shrunk {
                r.missing.groups_shrunk.insert(group_key.clone());
            }
        }
    }
}

fn compute_role_scores(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    aggregation: &crate::coefficients::AggregationConfig,
) {
    for r in records.iter_mut() {
        let mut role_values: BTreeMap<&str, f64> = BTreeMap::new();
        for &role in ROLE_KEYS {
            let weights = match coef.final_score_weights.get(role) {
                Some(w) => w,
                None => continue,
            };
            let prefix = format!("{role}/");
            let (v, _shrunk) =
                missing_safe_avg(&r.groups, weights, &mut r.missing, &prefix, aggregation);
            role_values.insert(role, v);
        }
        r.scores.i_raw = *role_values.get("I_raw").unwrap_or(&50.0);
        r.scores.p_raw = *role_values.get("P_raw").unwrap_or(&50.0);
        r.scores.b_raw = *role_values.get("B_raw").unwrap_or(&50.0);
        r.scores.r = *role_values.get("R").unwrap_or(&50.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModelRecord, Vendor};

    fn make_record(id: &str, vendor: Vendor, raw: &[(&str, f64)]) -> ModelRecord {
        let mut r = ModelRecord::new(id.to_string(), id.to_string(), vendor);
        for (k, v) in raw {
            r.raw_metrics.insert(k.to_string(), *v);
        }
        r
    }

    #[test]
    fn synthesized_metric_values_are_pulled_toward_50() {
        use crate::model::{SynthesisCategory, SynthesisProvenance};
        let coef = Coefficients::load_embedded().unwrap();
        // Three records: low, high (direct), high (synthesized). The two
        // high records share a raw value but the synthesized one should
        // be pulled toward 50 by `[penalties].synthesis`.
        let mut records = vec![
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("TerminalBench", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("TerminalBench", 100.0)],
            ),
            make_record(
                "s/synth",
                Vendor::Other("s".into()),
                &[("TerminalBench", 100.0)],
            ),
        ];
        records[2].synthesized.insert(
            "TerminalBench".to_string(),
            SynthesisProvenance {
                source_id: "terminal_bench".to_string(),
                from: "d/direct".to_string(),
                category: SynthesisCategory::Conservative,
            },
        );
        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("TerminalBench").copied().unwrap();
        let synth = records[2].metrics.get("TerminalBench").copied().unwrap();
        // Direct value should percentile-normalize to ~100 (top of pop).
        assert!(direct > 95.0, "direct={direct}");
        // Synthesized value should be 100 * 0.85 + 50 * 0.15 = 92.5.
        assert!(
            (synth - 92.5).abs() < 0.5,
            "synthesized TerminalBench should pull toward 50, got {synth} (direct={direct})"
        );
    }

    #[test]
    fn same_series_forward_synthesized_metric_values_are_not_penalized() {
        use crate::model::{SynthesisCategory, SynthesisProvenance};
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("TerminalBench", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("TerminalBench", 100.0)],
            ),
            make_record(
                "s/synth",
                Vendor::Other("s".into()),
                &[("TerminalBench", 100.0)],
            ),
        ];
        records[2].synthesized.insert(
            "TerminalBench".to_string(),
            SynthesisProvenance {
                source_id: "terminal_bench".to_string(),
                from: "d/direct".to_string(),
                category: SynthesisCategory::SameSeriesForward,
            },
        );
        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("TerminalBench").copied().unwrap();
        let synth = records[2].metrics.get("TerminalBench").copied().unwrap();
        assert!(direct > 95.0, "direct={direct}");
        assert!(
            (synth - direct).abs() < 1e-9,
            "same-series forward synthesis should match direct normalized value, got synth={synth}, direct={direct}"
        );
    }

    #[test]
    fn override_reported_metric_values_match_direct_normalized_values() {
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("TerminalBench", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("TerminalBench", 100.0)],
            ),
            make_record(
                "o/override",
                Vendor::Other("o".into()),
                &[("TerminalBench", 100.0)],
            ),
        ];
        records[2]
            .override_reported
            .insert("TerminalBench".to_string());

        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("TerminalBench").copied().unwrap();
        let reported = records[2].metrics.get("TerminalBench").copied().unwrap();
        assert!(direct > 95.0, "direct={direct}");
        assert!(
            (reported - direct).abs() < 1e-9,
            "override-reported score should match the direct normalized value, got reported={reported}, direct={direct}"
        );
    }

    #[test]
    fn synthesized_metrics_do_not_set_normalization_baseline() {
        use crate::model::{SynthesisCategory, SynthesisProvenance};
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("TerminalBench", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("TerminalBench", 50.0)],
            ),
            make_record(
                "s/synth",
                Vendor::Other("s".into()),
                &[("TerminalBench", 1000.0)],
            ),
        ];
        records[2].synthesized.insert(
            "TerminalBench".to_string(),
            SynthesisProvenance {
                source_id: "terminal_bench".to_string(),
                from: "d/direct".to_string(),
                category: SynthesisCategory::Conservative,
            },
        );

        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("TerminalBench").copied().unwrap();
        assert!(
            direct > 95.0,
            "synthesized outlier should not stretch direct normalization baseline, got {direct}"
        );
    }

    #[test]
    fn override_reported_metrics_do_not_set_normalization_baseline_when_direct_population_exists() {
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("TerminalBench", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("TerminalBench", 50.0)],
            ),
            make_record(
                "o/override",
                Vendor::Other("o".into()),
                &[("TerminalBench", 1000.0)],
            ),
        ];
        records[2]
            .override_reported
            .insert("TerminalBench".to_string());

        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("TerminalBench").copied().unwrap();
        assert!(
            direct > 95.0,
            "override outlier should not stretch direct normalization baseline, got {direct}"
        );
    }

    #[test]
    fn pipeline_runs_end_to_end_with_no_metrics() {
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record("a/x", Vendor::Other("a".into()), &[]),
            make_record("b/y", Vendor::Other("b".into()), &[]),
        ];
        compute_scores_with(&mut records, &coef);
        for r in &records {
            assert!((r.scores.i_raw - 50.0).abs() < 1e-9);
            assert!((r.scores.p_raw - 50.0).abs() < 1e-9);
            assert!((r.scores.b_raw - 50.0).abs() < 1e-9);
            assert!((r.scores.r - 50.0).abs() < 1e-9);
        }
    }

    #[test]
    fn composite_metric_blends_inputs_and_feeds_groups() {
        let mut coef = Coefficients::load_embedded().unwrap();
        // Strip BUILD down to a single weight on SWEComposite to make the
        // arithmetic verifiable end-to-end.
        coef.group_weights.insert(
            "BUILD".to_string(),
            [("SWEComposite".to_string(), 1.0)].into_iter().collect(),
        );

        // Two records both holding the same composite inputs at the
        // population extremes so percentile normalization yields 0/100.
        let mut records = vec![
            make_record(
                "low/x",
                Vendor::Other("a".into()),
                &[
                    ("SWERebench", 0.0),
                    ("SWEBenchVerified", 0.0),
                    ("SWEBenchMultilingual", 0.0),
                    ("SWEBenchPro", 0.0),
                ],
            ),
            make_record(
                "hi/y",
                Vendor::Other("b".into()),
                &[
                    ("SWERebench", 100.0),
                    ("SWEBenchVerified", 100.0),
                    ("SWEBenchMultilingual", 100.0),
                    ("SWEBenchPro", 100.0),
                ],
            ),
        ];
        compute_scores_with(&mut records, &coef);

        let high_composite = records[1].metrics.get("SWEComposite").copied().unwrap();
        assert!(
            (high_composite - 100.0).abs() < 1e-6,
            "expected ~100, got {high_composite}"
        );
        let low_composite = records[0].metrics.get("SWEComposite").copied().unwrap();
        assert!(
            (low_composite - 0.0).abs() < 1e-6,
            "expected ~0, got {low_composite}"
        );
        // BUILD group should now be exactly the composite (since it's the only weight).
        let high_code = records[1].groups.get("BUILD").copied().unwrap();
        assert!((high_code - 100.0).abs() < 1e-6, "BUILD={high_code}");
    }

    #[test]
    fn composite_metric_handles_partial_inputs() {
        let mut coef = Coefficients::load_embedded().unwrap();
        coef.group_weights.insert(
            "BUILD".to_string(),
            [("SWEComposite".to_string(), 1.0)].into_iter().collect(),
        );

        // Only one of the three SWE inputs is present — composite should
        // shrink toward 50 proportional to the missing weight.
        let mut records = vec![
            make_record("low/x", Vendor::Other("a".into()), &[("SWERebench", 0.0)]),
            make_record("hi/y", Vendor::Other("b".into()), &[("SWERebench", 100.0)]),
        ];
        compute_scores_with(&mut records, &coef);

        let high = records[1].metrics.get("SWEComposite").copied().unwrap();
        // SWERebench carries weight 0.45 of 1.00 in the composite — that's
        // below the 0.70 trust threshold, so the present-weighted mean (100)
        // gets pulled toward 50: 100*0.45 + 50*0.55 = 72.5.
        assert!(
            (high - 72.5).abs() < 1e-6,
            "expected partial-coverage shrink to 72.5, got {high}"
        );
    }

    #[test]
    fn group_with_no_metrics_yields_50() {
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![make_record("a/x", Vendor::Other("a".into()), &[])];
        compute_scores_with(&mut records, &coef);
        for v in records[0].groups.values() {
            assert!((*v - 50.0).abs() < 1e-9);
        }
    }
}
