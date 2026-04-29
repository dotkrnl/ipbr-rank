use crate::aggregate::missing_safe_avg;
use crate::coefficients::{Coefficients, MetricTransform, PenaltiesConfig};
use crate::model::{ModelRecord, Vendor};
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
    compute_aisl_perspectives(records, coef, &aggregation);
    aggregate_groups(records, coef, &aggregation);
    compute_role_scores(records, coef, &aggregation);
    apply_canary_health_penalty(records, &penalties);
    apply_reviewer_reservation(records, coef);
}

/// Compute the AI Stupid Level perspective groups (A_I / A_P / A_B / A_R)
/// from the AISL capability-axis metrics. The resulting values are written
/// directly into `r.groups` so role aggregation can consume them as if
/// they were aggregated from `[group_weights.A_X]` tables. We keep them
/// in their own config block (`ai_stupid_perspective_weights`) instead of
/// folding into `group_weights` because they aren't conceptually weighted
/// averages of independent leaderboards — they're a fixed re-projection
/// of one source's capability suite output. `AI_canary_health` is excluded
/// and applied later as a penalty-only drift signal.
fn compute_aisl_perspectives(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    aggregation: &crate::coefficients::AggregationConfig,
) {
    // AISL synthesis discounting is applied once, at the metric level
    // (see `SYNTHESIS_PENALTY` in `normalize_population`). The previous
    // implementation also pulled the entire perspective toward 50 a second
    // time; that double-pull made AISL synthesis uniquely harsh compared
    // with synthesis on any other source family.
    for r in records.iter_mut() {
        for (perspective, weights) in &coef.ai_stupid_perspective_weights {
            let prefix = format!("{perspective}/");
            let (value, shrunk) =
                missing_safe_avg(&r.metrics, weights, &mut r.missing, &prefix, aggregation);
            r.groups.insert(perspective.clone(), value);
            if shrunk {
                r.missing.groups_shrunk.insert(perspective.clone());
            }
        }
    }
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

const CANARY_HEALTH_METRIC: &str = "AI_canary_health";

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
                let final_value = if r.synthesized.contains_key(metric_key) {
                    // Pull synthesized values toward the 50-baseline so they
                    // act as a softer signal than direct measurements.
                    v * (1.0 - penalties.synthesis) + 50.0 * penalties.synthesis
                } else {
                    v
                };
                let final_value = if r.override_reported.contains(metric_key) {
                    // Manual overrides are public but hand-curated. Keep
                    // them strong, while making them slightly softer than a
                    // directly ingested leaderboard row.
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

fn apply_canary_health_penalty(records: &mut [ModelRecord], penalties: &PenaltiesConfig) {
    for r in records.iter_mut() {
        if r.synthesized.contains_key(CANARY_HEALTH_METRIC) {
            continue;
        }
        let Some(health) = r.metrics.get(CANARY_HEALTH_METRIC).copied() else {
            continue;
        };
        // Deadband: healthy canaries (>= 60) get no penalty. Below the
        // deadband, ramp linearly to the full penalty at `CANARY_HEALTH_FLOOR`.
        let span = penalties.canary_health_deadband - penalties.canary_health_floor;
        if span <= 0.0 {
            continue;
        }
        let degradation = ((penalties.canary_health_deadband - health) / span).clamp(0.0, 1.0);
        let penalty = degradation * penalties.canary_max_role_penalty;
        if penalty <= 0.0 {
            continue;
        }
        r.scores.i_raw = (r.scores.i_raw - penalty).clamp(0.0, 100.0);
        r.scores.p_raw = (r.scores.p_raw - penalty).clamp(0.0, 100.0);
        r.scores.b_raw = (r.scores.b_raw - penalty).clamp(0.0, 100.0);
        r.scores.r = (r.scores.r - penalty).clamp(0.0, 100.0);
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

    // Per-vendor "max R among other vendors" — used to compute the vendor-
    // level review-axis lead.
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
        // Vendor-level lead (only positive when this vendor's top R sits
        // strictly above every other vendor's top R).
        let vendor_lead = if outside.is_finite() && max_r_all.is_finite() {
            (max_r_all - outside).max(0.0)
        } else {
            0.0
        };
        // Per-model share of that lead: 1.0 for the actual top-R model in
        // the vendor, 0.0 for siblings tied with the outside maximum, and
        // proportional in between. Stops the reservation tax from hitting
        // every model that happens to share a vendor with a strong reviewer.
        let model_share = if vendor_lead > 0.0 {
            ((r.scores.r - outside) / vendor_lead).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let l_v = vendor_lead * model_share;
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
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("AI_correctness", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("AI_correctness", 100.0)],
            ),
            make_record(
                "s/synth",
                Vendor::Other("s".into()),
                &[("AI_correctness", 100.0)],
            ),
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
    fn override_reported_metric_values_are_pulled_toward_50() {
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
            (reported - 95.0).abs() < 0.5,
            "override-reported score should get a 10% uncertainty pull toward 50, got {reported}"
        );
    }

    #[test]
    fn synthesized_metrics_do_not_set_normalization_baseline() {
        use crate::model::SynthesisProvenance;
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("AI_correctness", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("AI_correctness", 50.0)],
            ),
            make_record(
                "s/synth",
                Vendor::Other("s".into()),
                &[("AI_correctness", 1000.0)],
            ),
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
    fn canary_health_is_penalty_only() {
        let coef = Coefficients::load_embedded().unwrap();
        let base_metrics = [
            ("AI_correctness", 100.0),
            ("AI_spec", 100.0),
            ("AI_code", 100.0),
            ("AI_efficiency", 100.0),
            ("AI_stability", 100.0),
            ("AI_refusal", 100.0),
            ("AI_recovery", 100.0),
        ];
        let mut good_raw = base_metrics.to_vec();
        good_raw.push(("AI_canary_health", 100.0));
        let mut bad_raw = base_metrics.to_vec();
        bad_raw.push(("AI_canary_health", 25.0));
        let mut records = vec![
            make_record("missing/x", Vendor::Other("a".into()), &base_metrics),
            make_record("good/y", Vendor::Other("b".into()), &good_raw),
            make_record("bad/z", Vendor::Other("c".into()), &bad_raw),
        ];

        compute_scores_with(&mut records, &coef);

        let missing = records[0].scores.i_raw;
        let good = records[1].scores.i_raw;
        let bad = records[2].scores.i_raw;
        assert!(
            (good - missing).abs() < 1e-6,
            "healthy canary should not boost role scores: good={good}, missing={missing}"
        );
        assert!(
            bad < missing - 5.0,
            "bad canary should apply a visible penalty: bad={bad}, missing={missing}"
        );
    }

    #[test]
    fn canary_health_survives_deadband_leq_floor() {
        let mut coef = Coefficients::load_embedded().unwrap();
        let mut penalties = coef.penalties.clone().unwrap_or_default();
        penalties.canary_health_deadband = 20.0;
        penalties.canary_health_floor = 20.0;
        coef.penalties = Some(penalties);

        let mut records = vec![make_record(
            "a/x",
            Vendor::Other("a".into()),
            &[("AI_canary_health", 10.0)],
        )];
        compute_scores_with(&mut records, &coef);
        assert!(
            records[0].scores.i_raw.is_finite(),
            "span <= 0 must not produce NaN"
        );
    }

    #[test]
    fn all_synthesized_aisl_perspective_gets_partial_uncertainty_pull() {
        use crate::model::SynthesisProvenance;
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "l/low",
                Vendor::Other("l".into()),
                &[("AI_correctness", 0.0)],
            ),
            make_record(
                "d/direct",
                Vendor::Other("d".into()),
                &[("AI_correctness", 100.0)],
            ),
            make_record(
                "s/synth",
                Vendor::Other("s".into()),
                &[("AI_correctness", 100.0)],
            ),
        ];
        records[2].synthesized.insert(
            "AI_correctness".to_string(),
            SynthesisProvenance {
                source_id: "aistupidlevel".to_string(),
                from: "d/direct".to_string(),
            },
        );

        compute_scores_with(&mut records, &coef);

        let synth_ai = records[2].groups.get("A_I").copied().unwrap();
        // AI_correctness weight in A_I = 0.18, total A_I weight = 1.0.
        // Synth metric value = 92.5 (after metric-level SYNTHESIS_PENALTY).
        // Group missing-safe avg with present_weight 0.18 < 0.60 → full shrink:
        //   92.5 * 0.18 + 50 * 0.82 = 57.65.
        // Group-level synth pull was removed (single-layer discounting now);
        // the group value should equal the shrink result directly.
        let expected = 92.5 * 0.18 + 50.0 * 0.82;
        assert!(
            (synth_ai - expected).abs() < 1e-4,
            "synthesized AISL perspective should reflect metric-level pull only, got {synth_ai}"
        );
        let direct_ai = records[1].groups.get("A_I").copied().unwrap();
        assert!(
            direct_ai > 50.0,
            "direct AISL evidence should still score, got {direct_ai}"
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
                    ("LMArenaSearchDocument", 100.0),
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
        // SWERebench carries weight 0.30 of 1.00 in the composite — that's
        // below the 0.60 trust threshold, so the present-weighted mean (100)
        // gets pulled toward 50: 100*0.30 + 50*0.70 = 65.
        assert!(
            (high - 65.0).abs() < 1e-6,
            "expected partial-coverage shrink to 65, got {high}"
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
