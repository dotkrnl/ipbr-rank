use crate::aggregate::missing_safe_avg;
use crate::coefficients::{Coefficients, MetricTransform};
use crate::model::{ModelRecord, Vendor};
use crate::normalize::{as_score_0_100, robust_norm, tail_penalty_norm};
use std::collections::BTreeMap;

const ROLE_KEYS: &[&str] = &["I_raw", "P_raw", "B_raw", "R"];

pub fn compute_scores(records: &mut [ModelRecord]) {
    let coef = Coefficients::load_embedded().expect("embedded coefficients are valid");
    compute_scores_with(records, &coef);
}

pub fn compute_scores_with(records: &mut [ModelRecord], coef: &Coefficients) {
    normalize_population(records, coef);
    compute_composite_metrics(records, coef);
    compute_aisl_perspectives(records, coef);
    aggregate_groups(records, coef);
    compute_role_scores(records, coef);
    apply_reviewer_reservation(records, coef);
}

/// Compute the AI Stupid Level perspective groups (A_I / A_P / A_B / A_R)
/// from the seven `AI_*` axis metrics. The resulting values are written
/// directly into `r.groups` so role aggregation can consume them as if
/// they were aggregated from `[group_weights.A_X]` tables. We keep them
/// in their own config block (`ai_stupid_perspective_weights`) instead of
/// folding into `group_weights` because they aren't conceptually weighted
/// averages of independent leaderboards — they're a fixed re-projection
/// of one source's seven-axis output.
fn compute_aisl_perspectives(records: &mut [ModelRecord], coef: &Coefficients) {
    for r in records.iter_mut() {
        for (perspective, weights) in &coef.ai_stupid_perspective_weights {
            let prefix = format!("{perspective}/");
            let value = missing_safe_avg(&r.metrics, weights, &mut r.missing, &prefix);
            r.groups.insert(perspective.clone(), value);
        }
    }
}

/// Compute each composite metric as a missing-safe weighted average of its
/// input metrics (post-normalization). The result is written into `r.metrics`
/// under the composite's name so that group aggregation can consume it as if
/// it were a regular metric. Provenance for missing inputs is recorded with a
/// `<composite>/` prefix in `MissingInfo` so it doesn't collide with group
/// missingness.
fn compute_composite_metrics(records: &mut [ModelRecord], coef: &Coefficients) {
    for r in records.iter_mut() {
        for (name, weights) in &coef.composite_metrics {
            let prefix = format!("{name}/");
            let value = missing_safe_avg(&r.metrics, weights, &mut r.missing, &prefix);
            r.metrics.insert(name.clone(), value);
        }
    }
}

/// Conservative discount applied to per-metric values that came in via
/// sibling synthesis rather than a direct measurement. The synthesized
/// score is blended toward 50 by `SYNTHESIS_PENALTY` so the model can't
/// fully claim a sibling's strengths — the gap reflects our genuine
/// uncertainty about whether the donor's score transfers cleanly.
const SYNTHESIS_PENALTY: f64 = 0.15;

fn normalize_population(records: &mut [ModelRecord], coef: &Coefficients) {
    for (metric_key, def) in &coef.metrics {
        let pop: Vec<f64> = records
            .iter()
            .filter_map(|r| r.raw_metrics.get(metric_key).copied())
            .filter(|v| v.is_finite())
            .collect();
        for r in records.iter_mut() {
            let raw = match r.raw_metrics.get(metric_key) {
                Some(v) if v.is_finite() => *v,
                _ => continue,
            };
            let normed = match def.transform {
                MetricTransform::AsScore => as_score_0_100(raw),
                MetricTransform::Percentile => {
                    robust_norm(raw, &pop, def.higher_better, def.log_scale)
                }
                MetricTransform::TailPenalty => {
                    tail_penalty_norm(raw, &pop, def.higher_better, def.log_scale)
                }
            };
            if let Some(v) = normed {
                let final_value = if r.synthesized.contains_key(metric_key) {
                    // Pull synthesized values toward the 50-baseline so they
                    // act as a softer signal than direct measurements.
                    v * (1.0 - SYNTHESIS_PENALTY) + 50.0 * SYNTHESIS_PENALTY
                } else {
                    v
                };
                r.metrics.insert(metric_key.clone(), final_value);
            }
        }
    }
}

fn aggregate_groups(records: &mut [ModelRecord], coef: &Coefficients) {
    for r in records.iter_mut() {
        for (group_key, weights) in &coef.group_weights {
            let prefix = format!("{group_key}/");
            let v = missing_safe_avg(&r.metrics, weights, &mut r.missing, &prefix);
            r.groups.insert(group_key.clone(), v);
        }
    }
}

fn compute_role_scores(records: &mut [ModelRecord], coef: &Coefficients) {
    for r in records.iter_mut() {
        let mut role_values: BTreeMap<&str, f64> = BTreeMap::new();
        for &role in ROLE_KEYS {
            let weights = match coef.final_score_weights.get(role) {
                Some(w) => w,
                None => continue,
            };
            let prefix = format!("{role}/");
            let v = missing_safe_avg(&r.groups, weights, &mut r.missing, &prefix);
            role_values.insert(role, v);
        }
        r.scores.i_raw = *role_values.get("I_raw").unwrap_or(&50.0);
        r.scores.p_raw = *role_values.get("P_raw").unwrap_or(&50.0);
        r.scores.b_raw = *role_values.get("B_raw").unwrap_or(&50.0);
        r.scores.r = *role_values.get("R").unwrap_or(&50.0);
    }
}

fn apply_reviewer_reservation(records: &mut [ModelRecord], coef: &Coefficients) {
    let i_w = coef
        .reviewer_reservation
        .get("I_adj")
        .copied()
        .unwrap_or(0.0);
    let p_w = coef
        .reviewer_reservation
        .get("P_adj")
        .copied()
        .unwrap_or(0.0);
    let b_w = coef
        .reviewer_reservation
        .get("B_adj")
        .copied()
        .unwrap_or(0.0);

    let max_r_all = records
        .iter()
        .map(|r| r.scores.r)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut max_r_outside: BTreeMap<String, f64> = BTreeMap::new();
    for r in records.iter() {
        let outside = records
            .iter()
            .filter(|other| !same_vendor(&other.vendor, &r.vendor))
            .map(|other| other.scores.r)
            .fold(f64::NEG_INFINITY, f64::max);
        max_r_outside
            .entry(r.vendor.as_str().to_string())
            .or_insert(outside);
    }

    for r in records.iter_mut() {
        let outside = max_r_outside
            .get(r.vendor.as_str())
            .copied()
            .unwrap_or(f64::NEG_INFINITY);
        let l_v = if outside.is_finite() && max_r_all.is_finite() {
            (max_r_all - outside).max(0.0)
        } else {
            0.0
        };
        r.scores.i_adj = (r.scores.i_raw - i_w * l_v).clamp(0.0, 100.0);
        r.scores.p_adj = (r.scores.p_raw - p_w * l_v).clamp(0.0, 100.0);
        r.scores.b_adj = (r.scores.b_raw - b_w * l_v).clamp(0.0, 100.0);
    }
}

fn same_vendor(a: &Vendor, b: &Vendor) -> bool {
    a.as_str() == b.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelRecord;

    fn make_record(id: &str, vendor: Vendor, raw: &[(&str, f64)]) -> ModelRecord {
        let mut r = ModelRecord::new(id.to_string(), id.to_string(), vendor);
        for (k, v) in raw {
            r.raw_metrics.insert(k.to_string(), *v);
        }
        r
    }

    #[test]
    fn synthesized_metric_values_are_pulled_toward_50() {
        use crate::model::SynthesisProvenance;
        let coef = Coefficients::load_embedded().unwrap();
        // Three records: low, high (direct), high (synthesized). The two
        // high records share a raw value but the synthesized one should
        // be pulled toward 50 by SYNTHESIS_PENALTY.
        let mut records = vec![
            make_record("l/low", Vendor::Other("l".into()), &[("AI_correctness", 0.0)]),
            make_record("d/direct", Vendor::Other("d".into()), &[("AI_correctness", 100.0)]),
            make_record("s/synth", Vendor::Other("s".into()), &[("AI_correctness", 100.0)]),
        ];
        records[2].synthesized.insert(
            "AI_correctness".to_string(),
            SynthesisProvenance {
                source_id: "aistupidlevel".to_string(),
                from: "d/direct".to_string(),
            },
        );
        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("AI_correctness").copied().unwrap();
        let synth = records[2].metrics.get("AI_correctness").copied().unwrap();
        // Direct value should percentile-normalize to ~100 (top of pop).
        assert!(direct > 95.0, "direct={direct}");
        // Synthesized value should be 100 * 0.85 + 50 * 0.15 = 92.5.
        assert!(
            (synth - 92.5).abs() < 0.5,
            "synthesized AI_correctness should pull toward 50, got {synth} (direct={direct})"
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
            assert!((r.scores.i_adj - 50.0).abs() < 1e-9);
            assert!((r.scores.p_adj - 50.0).abs() < 1e-9);
            assert!((r.scores.b_adj - 50.0).abs() < 1e-9);
        }
    }

    #[test]
    fn reviewer_reservation_zero_when_top_vendor_ties() {
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "a/x",
                Vendor::Other("a".into()),
                &[("AI_correctness", 100.0)],
            ),
            make_record(
                "b/y",
                Vendor::Other("b".into()),
                &[("AI_correctness", 100.0)],
            ),
        ];
        compute_scores_with(&mut records, &coef);
        for r in &records {
            assert!(
                (r.scores.i_raw - r.scores.i_adj).abs() < 1e-6,
                "tied top should leave i_adj == i_raw"
            );
            assert!((r.scores.b_raw - r.scores.b_adj).abs() < 1e-6);
        }
    }

    #[test]
    fn reviewer_reservation_penalises_sole_top_vendor() {
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "a/x",
                Vendor::Other("a".into()),
                &[
                    ("AI_correctness", 100.0),
                    ("AI_spec", 100.0),
                    ("AI_code", 100.0),
                    ("AI_efficiency", 100.0),
                    ("AI_stability", 100.0),
                    ("AI_refusal", 100.0),
                    ("AI_recovery", 100.0),
                    ("JudgeBench", 100.0),
                ],
            ),
            make_record(
                "b/y",
                Vendor::Other("b".into()),
                &[
                    ("AI_correctness", 0.0),
                    ("AI_spec", 0.0),
                    ("AI_code", 0.0),
                    ("AI_efficiency", 0.0),
                    ("AI_stability", 0.0),
                    ("AI_refusal", 0.0),
                    ("AI_recovery", 0.0),
                ],
            ),
        ];
        compute_scores_with(&mut records, &coef);
        let top = &records[0];
        let l_v = top.scores.r - records[1].scores.r;
        assert!(l_v > 0.0, "vendor a is sole top reviewer, got l_v={l_v}");
        let expected_b_adj = (top.scores.b_raw - 0.32 * l_v).max(0.0);
        assert!(
            (top.scores.b_adj - expected_b_adj).abs() < 1e-6,
            "b_adj mismatch: {} vs {}",
            top.scores.b_adj,
            expected_b_adj
        );
    }

    #[test]
    fn composite_metric_blends_inputs_and_feeds_groups() {
        let mut coef = Coefficients::load_embedded().unwrap();
        // Strip BUILD down to a single weight on SWEComposite to make the
        // arithmetic verifiable end-to-end.
        coef.group_weights
            .insert("BUILD".to_string(), [("SWEComposite".to_string(), 1.0)].into_iter().collect());

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
        coef.group_weights
            .insert("BUILD".to_string(), [("SWEComposite".to_string(), 1.0)].into_iter().collect());

        // Only one of the three SWE inputs is present — composite should
        // shrink toward 50 proportional to the missing weight.
        let mut records = vec![
            make_record("low/x", Vendor::Other("a".into()), &[("SWERebench", 0.0)]),
            make_record("hi/y", Vendor::Other("b".into()), &[("SWERebench", 100.0)]),
        ];
        compute_scores_with(&mut records, &coef);

        let high = records[1].metrics.get("SWEComposite").copied().unwrap();
        // SWERebench carries weight 0.35 of 1.00 in the composite — that's
        // below the 0.70 trust threshold, so the present-weighted mean (100)
        // still gets pulled toward 50: 100*0.35 + 50*0.65 = 67.5.
        assert!(
            (high - 67.5).abs() < 1e-6,
            "expected partial-coverage shrink to 67.5, got {high}"
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
