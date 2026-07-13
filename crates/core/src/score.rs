use crate::coefficients::{Coefficients, EvidenceConfig, MetricEligibility, MetricTransform};
use crate::model::{
    EligibilityQualificationPath, EvidenceCoverage, EvidenceSummary, MissingInfo, ModelRecord,
};
use crate::normalize::{anchored_logistic_norm, as_score_0_100, robust_norm, tail_penalty_norm};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const ROLE_KEYS: &[&str] = &["I_raw", "P_raw", "B_raw", "R"];
const EPS: f64 = 1e-12;

/// Coverage below this fraction flags a group as `groups_shrunk` — a
/// presentation-only diagnostic. Capability itself uses available same-product
/// evidence and never switches formula at this threshold.
fn shrink_coverage_cutoff(cfg: &crate::coefficients::AggregationConfig) -> f64 {
    (cfg.trust_threshold + cfg.trust_transition_width / 2.0).clamp(0.0, 1.0)
}

#[derive(Debug, Clone)]
struct Signal {
    value: f64,
    evidence: EvidenceCoverage,
}

#[derive(Debug, Clone)]
struct AggregateEvaluation {
    signal: Option<Signal>,
    evidence: EvidenceCoverage,
}

pub fn compute_scores(records: &mut [ModelRecord]) {
    let coef = Coefficients::load_embedded().expect("embedded coefficients are valid");
    compute_scores_with(records, &coef);
}

pub fn compute_scores_with(records: &mut [ModelRecord], coef: &Coefficients) {
    let aggregation = coef.aggregation.clone().unwrap_or_default();
    let evidence_cfg = coef.evidence.clone().unwrap_or_default();

    // Derived state must not leak across repeated scoring passes.
    for record in records.iter_mut() {
        record.metrics.clear();
        record.groups.clear();
        record.scores = Default::default();
        record.missing.metrics.clear();
        record.missing.groups_shrunk.clear();
        record.evidence = EvidenceSummary::default();
    }

    let mut metric_signals = normalize_population(records, coef, &evidence_cfg);
    compute_composite_metrics(records, coef, &evidence_cfg, &mut metric_signals);
    let group_signals =
        aggregate_groups(records, coef, &aggregation, &evidence_cfg, &metric_signals);
    compute_role_scores(
        records,
        coef,
        &evidence_cfg,
        &metric_signals,
        &group_signals,
    );
}

/// Balanced eligibility avoids an all-four-role veto: at least three roles
/// must qualify independently, and the remaining role must still have 20%
/// (configurable) direct current evidence. Numeric Balanced capability remains
/// the unchanged arithmetic mean of the four role scores.
pub fn balanced_is_provisional(model: &ModelRecord) -> bool {
    static CONFIG: OnceLock<EvidenceConfig> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        Coefficients::load_embedded()
            .expect("embedded coefficients are valid")
            .evidence
            .unwrap_or_default()
    });
    balanced_is_provisional_with(model, config)
}

pub fn balanced_is_provisional_with(model: &ModelRecord, config: &EvidenceConfig) -> bool {
    let qualifying_roles = ROLE_KEYS
        .iter()
        .filter_map(|role| model.evidence.roles.get(*role))
        .filter(|coverage| !coverage.provisional)
        .count();
    if qualifying_roles < 3 {
        return true;
    }
    if qualifying_roles == ROLE_KEYS.len() {
        return false;
    }

    let min_direct = config.balanced_min_fourth_direct.clamp(0.0, 1.0);
    !ROLE_KEYS.iter().all(|role| {
        model
            .evidence
            .roles
            .get(*role)
            .is_some_and(|coverage| !coverage.provisional || coverage.direct + EPS >= min_direct)
    })
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
    evidence_cfg: &EvidenceConfig,
    metric_signals: &mut [BTreeMap<String, Signal>],
) {
    let prior = evidence_cfg.prior();
    for (r, signals) in records.iter_mut().zip(metric_signals.iter_mut()) {
        for (name, weights) in &coef.composite_metrics {
            let prefix = format!("{name}/");
            let score_weights: BTreeMap<String, f64> = weights
                .iter()
                .filter(|(metric, _)| {
                    metric_eligibility(coef, metric) != MetricEligibility::HistoricalSupport
                })
                .map(|(metric, weight)| (metric.clone(), *weight))
                .collect();
            let evaluated = if coef.precedence_composites.contains(name) {
                precedence_signal(signals, &score_weights, Some(&mut r.missing), &prefix)
            } else {
                aggregate_signals(
                    signals,
                    &score_weights,
                    Some(&mut r.missing),
                    &prefix,
                    prior,
                )
            };
            // Crucial distinction: a fully absent composite stays absent.
            // Partial composites contain explicit prior replacements and
            // preserve their recursive evidence coverage.
            if let Some(signal) = evaluated.signal {
                r.metrics.insert(name.clone(), signal.value);
                signals.insert(name.clone(), signal);
            }
        }
    }
}

fn precedence_signal(
    signals: &BTreeMap<String, Signal>,
    weights: &BTreeMap<String, f64>,
    missing_info: Option<&mut MissingInfo>,
    prefix: &str,
) -> AggregateEvaluation {
    let selected = weights
        .iter()
        .filter(|(_, weight)| weight.is_finite() && **weight > 0.0)
        .filter_map(|(key, weight)| signals.get(key).map(|signal| (key, *weight, signal)))
        .max_by(|(left_key, left_weight, _), (right_key, right_weight, _)| {
            left_weight
                .total_cmp(right_weight)
                .then_with(|| right_key.cmp(left_key))
        });

    if let Some((_, _, signal)) = selected {
        return AggregateEvaluation {
            signal: Some(signal.clone()),
            evidence: signal.evidence.clone(),
        };
    }

    if let Some(info) = missing_info {
        for (key, weight) in weights {
            if weight.is_finite() && *weight > 0.0 {
                info.metrics.insert(format!("{prefix}{key}"));
            }
        }
    }
    AggregateEvaluation {
        signal: None,
        evidence: EvidenceCoverage {
            missing: 1.0,
            ..Default::default()
        },
    }
}

fn normalize_population(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    evidence_cfg: &EvidenceConfig,
) -> Vec<BTreeMap<String, Signal>> {
    let prior = evidence_cfg.prior();
    let mut signals = vec![BTreeMap::new(); records.len()];
    for (metric_key, def) in &coef.metrics {
        let pop: Vec<f64> = records
            .iter()
            .filter_map(|r| r.raw_metrics.get(metric_key).copied())
            .filter(|v| v.is_finite())
            .collect();
        for (idx, r) in records.iter_mut().enumerate() {
            let raw = match r.raw_metrics.get(metric_key) {
                Some(v) if v.is_finite() => *v,
                _ => continue,
            };
            let normed = match (def.anchor_low, def.anchor_high) {
                (Some(low), Some(high)) => {
                    anchored_logistic_norm(raw, low, high, def.higher_better, def.log_scale)
                }
                _ => match def.transform {
                    MetricTransform::AsScore => as_score_0_100(raw),
                    MetricTransform::Percentile => {
                        robust_norm(raw, &pop, def.higher_better, def.log_scale)
                    }
                    MetricTransform::TailPenalty => {
                        tail_penalty_norm(raw, &pop, def.higher_better, def.log_scale)
                    }
                },
            };
            if let Some(v) = normed {
                let family = def.family.clone().unwrap_or_else(|| metric_key.clone());
                let mut direct_families = BTreeSet::new();
                direct_families.insert(family);
                let reliability = evidence_cfg.reliability(evidence_cfg.direct_reliability);
                let mut coverage = EvidenceCoverage {
                    direct: 1.0,
                    direct_families,
                    family_count: 1,
                    ..Default::default()
                };
                coverage.effective = reliability;
                let final_value = prior + reliability * (v - prior);
                r.metrics.insert(metric_key.clone(), final_value);
                signals[idx].insert(
                    metric_key.clone(),
                    Signal {
                        value: final_value,
                        evidence: coverage,
                    },
                );
            }
        }
    }
    signals
}

fn aggregate_groups(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    aggregation: &crate::coefficients::AggregationConfig,
    evidence_cfg: &EvidenceConfig,
    metric_signals: &[BTreeMap<String, Signal>],
) -> Vec<BTreeMap<String, Signal>> {
    let prior = evidence_cfg.prior();
    let mut group_signals = vec![BTreeMap::new(); records.len()];
    for (idx, r) in records.iter_mut().enumerate() {
        for (group_key, weights) in &coef.group_weights {
            let prefix = format!("{group_key}/");
            let score_weights: BTreeMap<String, f64> = weights
                .iter()
                .filter(|(metric, _)| {
                    metric_eligibility(coef, metric) != MetricEligibility::HistoricalSupport
                })
                .map(|(metric, weight)| (metric.clone(), *weight))
                .collect();
            let evaluated = aggregate_signals(
                &metric_signals[idx],
                &score_weights,
                Some(&mut r.missing),
                &prefix,
                prior,
            );
            r.groups.insert(
                group_key.clone(),
                evaluated
                    .signal
                    .as_ref()
                    .map(|signal| signal.value)
                    .unwrap_or(prior),
            );
            if evaluated.evidence.effective < shrink_coverage_cutoff(aggregation) {
                r.missing.groups_shrunk.insert(group_key.clone());
            }
            r.evidence
                .groups
                .insert(group_key.clone(), evaluated.evidence.clone());
            if let Some(signal) = evaluated.signal {
                group_signals[idx].insert(group_key.clone(), signal);
            }
        }
    }
    group_signals
}

fn compute_role_scores(
    records: &mut [ModelRecord],
    coef: &Coefficients,
    evidence_cfg: &EvidenceConfig,
    metric_signals: &[BTreeMap<String, Signal>],
    group_signals: &[BTreeMap<String, Signal>],
) {
    let prior = evidence_cfg.prior();
    let role_leaf_weights: BTreeMap<&str, BTreeMap<String, f64>> = ROLE_KEYS
        .iter()
        .filter_map(|role| {
            coef.final_score_weights
                .get(*role)
                .map(|_| (*role, flatten_role_leaf_weights(coef, role)))
        })
        .collect();

    for (idx, r) in records.iter_mut().enumerate() {
        let mut role_values: BTreeMap<&str, f64> = BTreeMap::new();
        for &role in ROLE_KEYS {
            let Some(group_weights) = coef.final_score_weights.get(role) else {
                continue;
            };

            // Keep high-level missing group diagnostics even though final
            // scores are evaluated from flattened, family-capped leaves.
            for (group, weight) in group_weights {
                if weight.is_finite() && *weight > 0.0 && !group_signals[idx].contains_key(group) {
                    r.missing.metrics.insert(format!("{role}/{group}"));
                }
            }

            let leaves = role_leaf_weights.get(role).cloned().unwrap_or_default();
            let mut family_leaves: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
            for (metric, weight) in &leaves {
                let family = coef
                    .metrics
                    .get(metric)
                    .and_then(|def| def.family.clone())
                    .unwrap_or_else(|| metric.clone());
                *family_leaves
                    .entry(family)
                    .or_default()
                    .entry(metric.clone())
                    .or_default() += *weight;
            }

            let family_masses: BTreeMap<String, f64> = family_leaves
                .iter()
                .map(|(family, leaves)| (family.clone(), positive_weight_sum(leaves)))
                .collect();
            let capped_weights = cap_family_weights(&family_masses, evidence_cfg.max_family_weight);
            let mut family_signals = BTreeMap::new();
            let mut family_evidence = BTreeMap::new();
            for (family, leaf_weights) in &family_leaves {
                let evaluated =
                    aggregate_signals(&metric_signals[idx], leaf_weights, None, "", prior);
                family_evidence.insert(family.clone(), evaluated.evidence.clone());
                if let Some(signal) = evaluated.signal {
                    family_signals.insert(family.clone(), signal);
                }
            }
            let evaluated = aggregate_signals(&family_signals, &capped_weights, None, "", prior);
            let value = evaluated
                .signal
                .as_ref()
                .map(|signal| signal.value)
                .unwrap_or(prior);
            // Numeric capability is estimated from available same-product
            // evidence, while coverage always retains the full nominal path,
            // including prior-only sibling fills and truly missing leaves.
            let mut coverage = aggregate_evidence(&family_evidence, &capped_weights);
            let core = core_role_coverage(
                coef,
                &metric_signals[idx],
                &leaves,
                evidence_cfg.max_family_weight,
            );
            coverage.core_direct = core.direct;
            coverage.core_direct_families = core.direct_families;
            coverage.core_family_count = coverage.core_direct_families.len();
            coverage.historical_direct_families =
                historical_role_families(coef, &metric_signals[idx], role);
            coverage.historical_family_count = coverage.historical_direct_families.len();
            coverage.qualification_path = role_qualification_path(&coverage, evidence_cfg);
            coverage.provisional = matches!(
                coverage.qualification_path,
                EligibilityQualificationPath::Unqualified
            );
            r.evidence.roles.insert(role.to_string(), coverage);
            role_values.insert(role, value);
        }
        r.scores.i_raw = *role_values.get("I_raw").unwrap_or(&50.0);
        r.scores.p_raw = *role_values.get("P_raw").unwrap_or(&50.0);
        r.scores.b_raw = *role_values.get("B_raw").unwrap_or(&50.0);
        r.scores.r = *role_values.get("R").unwrap_or(&50.0);
    }
}

fn role_qualification_path(
    coverage: &EvidenceCoverage,
    config: &EvidenceConfig,
) -> EligibilityQualificationPath {
    let has_representative_core = coverage.core_direct + EPS
        >= config.representative_min_core_direct.clamp(0.0, 1.0)
        && coverage.core_family_count >= config.representative_min_core_families;
    if has_representative_core
        && coverage.direct + EPS >= config.provisional_min_direct.clamp(0.0, 1.0)
        && coverage.family_count >= config.provisional_min_families
    {
        return EligibilityQualificationPath::Standard;
    }
    if has_representative_core
        && coverage.direct + EPS >= config.provisional_breadth_min_direct.clamp(0.0, 1.0)
        && coverage.family_count >= config.provisional_breadth_min_families
    {
        return EligibilityQualificationPath::Breadth;
    }
    if coverage.core_direct + EPS >= config.provisional_min_direct.clamp(0.0, 1.0)
        && coverage.core_family_count >= config.provisional_min_families
    {
        return EligibilityQualificationPath::CoreStandard;
    }
    if coverage.core_direct + EPS >= config.core_corroborated_min_direct.clamp(0.0, 1.0)
        && coverage.core_family_count >= config.core_corroborated_min_families
    {
        return EligibilityQualificationPath::CoreCorroborated;
    }
    if coverage.core_direct + EPS >= config.provisional_breadth_min_direct.clamp(0.0, 1.0)
        && coverage.core_family_count >= config.provisional_breadth_min_families
    {
        return EligibilityQualificationPath::CoreBreadth;
    }

    let total_families = coverage
        .direct_families
        .union(&coverage.historical_direct_families)
        .count();
    if coverage.direct + EPS >= config.historical_min_current_direct.clamp(0.0, 1.0)
        && coverage.family_count >= config.historical_min_current_families
        && coverage.historical_family_count >= config.historical_min_families
        && total_families >= config.historical_min_total_families
    {
        return EligibilityQualificationPath::HistoricalBreadth;
    }

    EligibilityQualificationPath::Unqualified
}

/// Compute direct coverage over current core metrics only. Core weights are
/// renormalized before the usual family cap, so absent specialist sources do
/// not dilute eligibility while correlated core leaves remain controlled.
fn core_role_coverage(
    coef: &Coefficients,
    metric_signals: &BTreeMap<String, Signal>,
    role_leaves: &BTreeMap<String, f64>,
    max_family_weight: f64,
) -> EvidenceCoverage {
    let mut family_leaves: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for (metric, weight) in role_leaves {
        if metric_eligibility(coef, metric) != MetricEligibility::Core {
            continue;
        }
        let family = coef
            .metrics
            .get(metric)
            .and_then(|def| def.family.clone())
            .unwrap_or_else(|| metric.clone());
        family_leaves
            .entry(family)
            .or_default()
            .insert(metric.clone(), *weight);
    }
    if family_leaves.is_empty() {
        return EvidenceCoverage::default();
    }

    let family_masses: BTreeMap<String, f64> = family_leaves
        .iter()
        .map(|(family, leaves)| (family.clone(), positive_weight_sum(leaves)))
        .collect();
    let capped_weights = cap_family_weights(&family_masses, max_family_weight);
    let family_evidence: BTreeMap<String, EvidenceCoverage> = family_leaves
        .iter()
        .map(|(family, leaves)| {
            (
                family.clone(),
                aggregate_signals(metric_signals, leaves, None, "", 50.0).evidence,
            )
        })
        .collect();
    aggregate_evidence(&family_evidence, &capped_weights)
}

fn metric_eligibility(coef: &Coefficients, metric: &str) -> MetricEligibility {
    fn visit(
        coef: &Coefficients,
        metric: &str,
        visiting: &mut BTreeSet<String>,
    ) -> MetricEligibility {
        if let Some(def) = coef.metrics.get(metric) {
            return def.eligibility;
        }
        let Some(inputs) = coef.composite_metrics.get(metric) else {
            return MetricEligibility::Supplemental;
        };
        if !visiting.insert(metric.to_string()) {
            return MetricEligibility::Supplemental;
        }
        let mut classes = inputs
            .iter()
            .filter(|(_, weight)| weight.is_finite() && **weight > 0.0)
            .map(|(input, _)| visit(coef, input, visiting));
        let first = classes.next().unwrap_or(MetricEligibility::Supplemental);
        let result = if classes.all(|class| class == first) {
            first
        } else {
            MetricEligibility::Supplemental
        };
        visiting.remove(metric);
        result
    }

    visit(coef, metric, &mut BTreeSet::new())
}

fn historical_role_families(
    coef: &Coefficients,
    metric_signals: &BTreeMap<String, Signal>,
    role: &str,
) -> BTreeSet<String> {
    coef.metrics
        .iter()
        .filter(|(_, def)| {
            def.eligibility == MetricEligibility::HistoricalSupport
                && def.eligibility_roles.contains(role)
        })
        .filter_map(|(metric, def)| {
            metric_signals
                .get(metric)
                .filter(|signal| signal.evidence.direct > 1.0 - EPS)
                .map(|_| def.family.clone().unwrap_or_else(|| metric.clone()))
        })
        .collect()
}

fn aggregate_signals(
    signals: &BTreeMap<String, Signal>,
    weights: &BTreeMap<String, f64>,
    mut missing_info: Option<&mut MissingInfo>,
    prefix: &str,
    _prior: f64,
) -> AggregateEvaluation {
    let total = positive_weight_sum(weights);
    if total <= EPS {
        return AggregateEvaluation {
            signal: None,
            evidence: EvidenceCoverage {
                missing: 1.0,
                ..Default::default()
            },
        };
    }

    let mut value_sum = 0.0;
    let mut observed_weight = 0.0;
    let mut evidence = EvidenceCoverage::default();
    for (key, weight) in weights {
        if !weight.is_finite() || *weight <= 0.0 {
            continue;
        }
        if let Some(signal) = signals.get(key).filter(|signal| signal.value.is_finite()) {
            if signal.evidence.effective > EPS {
                observed_weight += weight;
                value_sum += weight * signal.value;
            }
            evidence.direct += weight * signal.evidence.direct;
            evidence.reported += weight * signal.evidence.reported;
            evidence.missing += weight * signal.evidence.missing;
            evidence.effective += weight * signal.evidence.effective;
            evidence
                .direct_families
                .extend(signal.evidence.direct_families.iter().cloned());
        } else {
            evidence.missing += weight;
            if let Some(info) = missing_info.as_deref_mut() {
                info.metrics.insert(format!("{prefix}{key}"));
            }
        }
    }

    evidence.direct /= total;
    evidence.reported /= total;
    evidence.missing /= total;
    evidence.effective /= total;
    evidence.family_count = evidence.direct_families.len();
    let signal = (observed_weight > EPS).then(|| Signal {
        value: (value_sum / observed_weight).clamp(0.0, 100.0),
        evidence: evidence.clone(),
    });
    AggregateEvaluation { signal, evidence }
}

fn aggregate_evidence(
    evidence_by_key: &BTreeMap<String, EvidenceCoverage>,
    weights: &BTreeMap<String, f64>,
) -> EvidenceCoverage {
    let total = positive_weight_sum(weights);
    if total <= EPS {
        return EvidenceCoverage {
            missing: 1.0,
            ..Default::default()
        };
    }
    let mut result = EvidenceCoverage::default();
    for (key, weight) in weights {
        if !weight.is_finite() || *weight <= 0.0 {
            continue;
        }
        if let Some(evidence) = evidence_by_key.get(key) {
            result.direct += weight * evidence.direct;
            result.reported += weight * evidence.reported;
            result.missing += weight * evidence.missing;
            result.effective += weight * evidence.effective;
            result
                .direct_families
                .extend(evidence.direct_families.iter().cloned());
        } else {
            result.missing += weight;
        }
    }
    result.direct /= total;
    result.reported /= total;
    result.missing /= total;
    result.effective /= total;
    result.family_count = result.direct_families.len();
    result
}

fn positive_weight_sum(weights: &BTreeMap<String, f64>) -> f64 {
    weights
        .values()
        .filter(|weight| weight.is_finite() && **weight > 0.0)
        .sum()
}

fn flatten_role_leaf_weights(coef: &Coefficients, role: &str) -> BTreeMap<String, f64> {
    let mut leaves = BTreeMap::new();
    let Some(groups) = coef.final_score_weights.get(role) else {
        return leaves;
    };
    let role_total = positive_weight_sum(groups);
    if role_total <= EPS {
        return leaves;
    }
    for (group, role_weight) in groups {
        if !role_weight.is_finite() || *role_weight <= 0.0 {
            continue;
        }
        let Some(metrics) = coef.group_weights.get(group) else {
            continue;
        };
        let group_total = positive_weight_sum(metrics);
        if group_total <= EPS {
            continue;
        }
        for (metric, metric_weight) in metrics {
            if !metric_weight.is_finite() || *metric_weight <= 0.0 {
                continue;
            }
            let path_weight = role_weight / role_total * metric_weight / group_total;
            expand_metric_leaves(coef, metric, path_weight, &mut leaves, &mut BTreeSet::new());
        }
    }
    leaves
}

fn expand_metric_leaves(
    coef: &Coefficients,
    metric: &str,
    weight: f64,
    leaves: &mut BTreeMap<String, f64>,
    visiting: &mut BTreeSet<String>,
) {
    if metric_eligibility(coef, metric) == MetricEligibility::HistoricalSupport {
        return;
    }
    if coef.precedence_composites.contains(metric) {
        *leaves.entry(metric.to_string()).or_default() += weight;
        return;
    }
    let Some(inputs) = coef.composite_metrics.get(metric) else {
        *leaves.entry(metric.to_string()).or_default() += weight;
        return;
    };
    if !visiting.insert(metric.to_string()) {
        // Invalid cycles are treated as a missing leaf instead of recursing.
        *leaves.entry(metric.to_string()).or_default() += weight;
        return;
    }
    let total = positive_weight_sum(inputs);
    if total <= EPS {
        *leaves.entry(metric.to_string()).or_default() += weight;
    } else {
        for (input, input_weight) in inputs {
            if input_weight.is_finite() && *input_weight > 0.0 {
                expand_metric_leaves(coef, input, weight * input_weight / total, leaves, visiting);
            }
        }
    }
    visiting.remove(metric);
}

/// Redistribute family mass with water-filling so no configured family
/// exceeds the cap. If there are too few families for the requested cap to
/// sum to one, the smallest feasible cap (1 / family_count) is used.
fn cap_family_weights(
    original: &BTreeMap<String, f64>,
    configured_cap: f64,
) -> BTreeMap<String, f64> {
    let mut normalized: BTreeMap<String, f64> = original
        .iter()
        .filter(|(_, weight)| weight.is_finite() && **weight > 0.0)
        .map(|(key, weight)| (key.clone(), *weight))
        .collect();
    let total: f64 = normalized.values().sum();
    if total <= EPS {
        return BTreeMap::new();
    }
    for weight in normalized.values_mut() {
        *weight /= total;
    }
    let count = normalized.len();
    let cap = if configured_cap.is_finite() {
        configured_cap.clamp(0.0, 1.0).max(1.0 / count as f64)
    } else {
        1.0
    };
    if cap >= 1.0 - EPS {
        return normalized;
    }

    let mut result = BTreeMap::new();
    let mut remaining: BTreeSet<String> = normalized.keys().cloned().collect();
    let mut remaining_mass = 1.0;
    loop {
        let original_remaining: f64 = remaining.iter().map(|key| normalized[key]).sum();
        if remaining.is_empty() || original_remaining <= EPS {
            break;
        }
        let offenders: Vec<String> = remaining
            .iter()
            .filter(|key| remaining_mass * normalized[*key] / original_remaining > cap + EPS)
            .cloned()
            .collect();
        if offenders.is_empty() {
            for key in remaining {
                result.insert(
                    key.clone(),
                    remaining_mass * normalized[&key] / original_remaining,
                );
            }
            break;
        }
        for key in offenders {
            result.insert(key.clone(), cap);
            remaining.remove(&key);
            remaining_mass -= cap;
        }
    }
    result
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
    fn role_qualification_accepts_standard_or_breadth_path() {
        let config = EvidenceConfig::default();
        let coverage = |direct, family_count| EvidenceCoverage {
            direct,
            family_count,
            core_direct: config.representative_min_core_direct,
            core_family_count: config.representative_min_core_families,
            ..Default::default()
        };

        assert_eq!(
            role_qualification_path(&coverage(0.60, 3), &config),
            EligibilityQualificationPath::Standard
        );
        assert_eq!(
            role_qualification_path(&coverage(0.35, 5), &config),
            EligibilityQualificationPath::Breadth
        );
        assert_eq!(
            role_qualification_path(&coverage(0.59, 4), &config),
            EligibilityQualificationPath::Unqualified
        );
        assert_eq!(
            role_qualification_path(&coverage(0.34, 8), &config),
            EligibilityQualificationPath::Unqualified
        );
        assert_eq!(
            role_qualification_path(&coverage(0.80, 2), &config),
            EligibilityQualificationPath::Unqualified
        );

        let mut corroborated = coverage(0.20, 2);
        corroborated.core_direct = 0.50;
        corroborated.core_family_count = 4;
        assert_eq!(
            role_qualification_path(&corroborated, &config),
            EligibilityQualificationPath::CoreCorroborated
        );

        let core_breadth = EvidenceCoverage {
            direct: 0.20,
            family_count: 5,
            core_direct: 0.35,
            core_family_count: 5,
            ..Default::default()
        };
        assert_eq!(
            role_qualification_path(&core_breadth, &config),
            EligibilityQualificationPath::CoreBreadth
        );
    }

    #[test]
    fn supplemental_favorable_subset_does_not_bypass_representative_core_gate() {
        let config = EvidenceConfig::default();
        let favorable_supplemental_subset = EvidenceCoverage {
            direct: 0.64,
            family_count: 5,
            core_direct: 0.34,
            core_family_count: 3,
            ..Default::default()
        };

        assert_eq!(
            role_qualification_path(&favorable_supplemental_subset, &config),
            EligibilityQualificationPath::Unqualified
        );

        let too_few_core_families = EvidenceCoverage {
            core_direct: 0.60,
            core_family_count: 2,
            ..favorable_supplemental_subset
        };
        assert_eq!(
            role_qualification_path(&too_few_core_families, &config),
            EligibilityQualificationPath::Unqualified
        );
    }

    #[test]
    fn supplemental_absence_does_not_dilute_core_eligibility() {
        let mut coef = Coefficients::load_embedded().unwrap();
        coef.evidence = Some(EvidenceConfig {
            max_family_weight: 1.0,
            ..Default::default()
        });
        coef.group_weights.insert(
            "BUILD".to_string(),
            [
                ("TerminalBench21".to_string(), 0.10),
                ("SWERebench".to_string(), 0.10),
                ("SciCode".to_string(), 0.10),
                ("DeepSWE".to_string(), 0.35),
                ("MCPAtlas".to_string(), 0.35),
            ]
            .into_iter()
            .collect(),
        );
        coef.final_score_weights.insert(
            "B_raw".to_string(),
            [("BUILD".to_string(), 1.0)].into_iter().collect(),
        );

        let mut records = vec![make_record(
            "core-only",
            Vendor::Other("x".into()),
            &[
                ("TerminalBench21", 70.0),
                ("SWERebench", 45.0),
                ("SciCode", 48.0),
            ],
        )];
        compute_scores_with(&mut records, &coef);

        let coverage = &records[0].evidence.roles["B_raw"];
        assert!((coverage.direct - 0.30).abs() < 1e-9, "{coverage:?}");
        assert!((coverage.core_direct - 1.0).abs() < 1e-9, "{coverage:?}");
        assert_eq!(coverage.core_family_count, 3, "{coverage:?}");
        assert_eq!(
            coverage.qualification_path,
            EligibilityQualificationPath::CoreStandard
        );
        assert!(!coverage.provisional);
    }

    #[test]
    fn historical_support_never_changes_numeric_scores() {
        let mut coef = Coefficients::load_embedded().unwrap();
        coef.group_weights.insert(
            "BUILD".to_string(),
            [("TerminalBench".to_string(), 1.0)].into_iter().collect(),
        );
        coef.final_score_weights.insert(
            "B_raw".to_string(),
            [("BUILD".to_string(), 1.0)].into_iter().collect(),
        );
        let mut records = vec![
            make_record("low", Vendor::Other("x".into()), &[("TerminalBench", 20.0)]),
            make_record(
                "high",
                Vendor::Other("y".into()),
                &[("TerminalBench", 80.0)],
            ),
        ];
        compute_scores_with(&mut records, &coef);

        for record in &records {
            assert!((record.scores.b_raw - 50.0).abs() < 1e-9);
            let coverage = &record.evidence.roles["B_raw"];
            assert_eq!(coverage.direct, 0.0);
            assert_eq!(coverage.historical_family_count, 1);
            assert!(
                coverage
                    .historical_direct_families
                    .contains("terminal_bench")
            );
        }
    }

    #[test]
    fn historical_path_requires_current_and_retired_breadth() {
        let config = EvidenceConfig::default();
        let qualifying = EvidenceCoverage {
            direct: 0.25,
            direct_families: ["current-a", "current-b", "current-c"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            family_count: 3,
            historical_direct_families: ["retired-a", "retired-b"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            historical_family_count: 2,
            ..Default::default()
        };
        assert_eq!(
            role_qualification_path(&qualifying, &config),
            EligibilityQualificationPath::HistoricalBreadth
        );

        let mut too_little_current = qualifying.clone();
        too_little_current.direct = 0.249;
        assert_eq!(
            role_qualification_path(&too_little_current, &config),
            EligibilityQualificationPath::Unqualified
        );
        let mut too_little_history = qualifying;
        too_little_history.historical_direct_families.pop_last();
        too_little_history.historical_family_count = 1;
        assert_eq!(
            role_qualification_path(&too_little_history, &config),
            EligibilityQualificationPath::Unqualified
        );
    }

    #[test]
    fn balanced_requires_three_roles_and_direct_support_in_the_fourth() {
        let config = EvidenceConfig::default();
        let mut model = make_record("model", Vendor::Other("x".into()), &[]);
        for role in ROLE_KEYS {
            model.evidence.roles.insert(
                role.to_string(),
                EvidenceCoverage {
                    direct: 0.80,
                    provisional: false,
                    ..Default::default()
                },
            );
        }
        model.evidence.roles.get_mut("R").unwrap().provisional = true;
        model.evidence.roles.get_mut("R").unwrap().direct = 0.20;
        assert!(!balanced_is_provisional_with(&model, &config));

        model.evidence.roles.get_mut("R").unwrap().direct = 0.199;
        assert!(balanced_is_provisional_with(&model, &config));

        model.evidence.roles.get_mut("B_raw").unwrap().provisional = true;
        model.evidence.roles.get_mut("R").unwrap().direct = 0.80;
        assert!(balanced_is_provisional_with(&model, &config));
    }

    #[test]
    fn curated_override_metric_values_use_direct_reliability() {
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
            .curated_overrides
            .insert("TerminalBench".to_string());

        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("TerminalBench").copied().unwrap();
        let curated = records[2].metrics.get("TerminalBench").copied().unwrap();
        assert!(direct > 95.0, "direct={direct}");
        assert!(
            (curated - direct).abs() < 1e-9,
            "curated same-product observations should use full direct reliability, got curated={curated}, direct={direct}"
        );
    }

    #[test]
    fn curated_override_metrics_set_the_direct_normalization_baseline() {
        let mut coef = Coefficients::load_embedded().unwrap();
        coef.metrics.get_mut("TerminalBench").unwrap().anchor_low = None;
        coef.metrics.get_mut("TerminalBench").unwrap().anchor_high = None;
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
            .curated_overrides
            .insert("TerminalBench".to_string());

        compute_scores_with(&mut records, &coef);

        let direct = records[1].metrics.get("TerminalBench").copied().unwrap();
        let curated = records[2].metrics.get("TerminalBench").copied().unwrap();
        assert!(
            direct < 75.0,
            "curated direct observation should participate in the direct baseline, got {direct}"
        );
        assert!(curated > direct, "curated={curated}, direct={direct}");
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
            (high_composite - 100.0).abs() < 0.01,
            "expected ~100, got {high_composite}"
        );
        let low_composite = records[0].metrics.get("SWEComposite").copied().unwrap();
        assert!(
            (low_composite - 0.0).abs() < 0.1,
            "expected ~0, got {low_composite}"
        );
        // BUILD group should now be exactly the composite (since it's the only weight).
        let high_code = records[1].groups.get("BUILD").copied().unwrap();
        assert!((high_code - 100.0).abs() < 0.01, "BUILD={high_code}");
    }

    #[test]
    fn composite_metric_handles_partial_inputs() {
        let mut coef = Coefficients::load_embedded().unwrap();
        coef.group_weights.insert(
            "BUILD".to_string(),
            [("SWEComposite".to_string(), 1.0)].into_iter().collect(),
        );

        // Only one of the three SWE inputs is present. Capability remains the
        // available same-product estimate; coverage carries the uncertainty.
        let mut records = vec![
            make_record("low/x", Vendor::Other("a".into()), &[("SWERebench", 0.0)]),
            make_record("hi/y", Vendor::Other("b".into()), &[("SWERebench", 100.0)]),
        ];
        compute_scores_with(&mut records, &coef);

        let high = records[1].metrics.get("SWEComposite").copied().unwrap();
        assert!(
            (high - 100.0).abs() < 0.01,
            "expected available-evidence estimate near 100, got {high}"
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
        for composite in coef.composite_metrics.keys() {
            assert!(
                !records[0].metrics.contains_key(composite),
                "fully missing composite {composite} must remain absent"
            );
        }
        assert!(
            records[0]
                .evidence
                .roles
                .values()
                .all(|coverage| coverage.missing > 0.999 && coverage.provisional)
        );
    }

    #[test]
    fn role_evidence_recurses_through_leaf_classes() {
        let mut coef = Coefficients::load_embedded().unwrap();
        coef.evidence = Some(EvidenceConfig {
            max_family_weight: 1.0,
            ..Default::default()
        });
        coef.group_weights.insert(
            "BUILD".to_string(),
            [
                ("TerminalBench21".to_string(), 0.5),
                ("SWEBenchPro".to_string(), 0.5),
            ]
            .into_iter()
            .collect(),
        );
        coef.final_score_weights.insert(
            "B_raw".to_string(),
            [("BUILD".to_string(), 1.0)].into_iter().collect(),
        );
        coef.metrics.get_mut("TerminalBench21").unwrap().family = Some("terminal".into());
        coef.metrics.get_mut("SWEBenchPro").unwrap().family = Some("swe".into());

        let mut records = vec![
            make_record(
                "low/x",
                Vendor::Other("a".into()),
                &[("TerminalBench21", 0.0), ("SWEBenchPro", 0.0)],
            ),
            make_record(
                "hi/y",
                Vendor::Other("b".into()),
                &[("TerminalBench21", 100.0), ("SWEBenchPro", 100.0)],
            ),
        ];
        records[1].curated_overrides.insert("SWEBenchPro".into());
        compute_scores_with(&mut records, &coef);

        let evidence = &records[1].evidence.roles["B_raw"];
        assert!((evidence.direct - 1.0).abs() < 1e-9, "{evidence:?}");
        assert!(evidence.reported.abs() < 1e-9, "{evidence:?}");
        assert!((evidence.effective - 1.0).abs() < 1e-9, "{evidence:?}");
        assert_eq!(
            evidence.direct_families,
            ["swe".to_string(), "terminal".to_string()].into()
        );
        assert_eq!(evidence.family_count, 2);
        assert!(evidence.provisional);
    }

    #[test]
    fn anchored_scores_do_not_move_when_unrelated_models_join_cohort() {
        let mut coef = Coefficients::load_embedded().unwrap();
        let def = coef.metrics.get_mut("TerminalBench").unwrap();
        def.anchor_low = Some(0.0);
        def.anchor_high = Some(100.0);
        let mut initial = vec![
            make_record("a", Vendor::Other("a".into()), &[("TerminalBench", 25.0)]),
            make_record("b", Vendor::Other("b".into()), &[("TerminalBench", 75.0)]),
        ];
        compute_scores_with(&mut initial, &coef);
        let before = initial[1].metrics["TerminalBench"];

        let mut expanded = vec![
            make_record("a", Vendor::Other("a".into()), &[("TerminalBench", 25.0)]),
            make_record("b", Vendor::Other("b".into()), &[("TerminalBench", 75.0)]),
            make_record(
                "outlier",
                Vendor::Other("c".into()),
                &[("TerminalBench", 10_000.0)],
            ),
        ];
        compute_scores_with(&mut expanded, &coef);
        assert!((expanded[1].metrics["TerminalBench"] - before).abs() < 1e-12);
    }

    #[test]
    fn family_cap_water_fills_excess_weight() {
        let weights = [
            ("a".to_string(), 0.70),
            ("b".to_string(), 0.10),
            ("c".to_string(), 0.10),
            ("d".to_string(), 0.10),
        ]
        .into_iter()
        .collect();
        let capped = cap_family_weights(&weights, 0.30);
        assert!((capped.values().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!((capped["a"] - 0.30).abs() < 1e-9);
        for family in ["b", "c", "d"] {
            assert!((capped[family] - 0.70 / 3.0).abs() < 1e-9);
        }
    }

    #[test]
    fn precedence_composite_uses_official_then_fallback_without_missing_penalty() {
        let coef = Coefficients::load_embedded().unwrap();
        let mut records = vec![
            make_record(
                "both",
                Vendor::Other("a".into()),
                &[("TerminalBench21", 70.0), ("AATerminalBench21", 40.0)],
            ),
            make_record(
                "fallback",
                Vendor::Other("b".into()),
                &[("AATerminalBench21", 70.0)],
            ),
        ];
        compute_scores_with(&mut records, &coef);
        assert_eq!(
            records[0].metrics["TerminalBench21Composite"],
            records[0].metrics["TerminalBench21"]
        );
        assert_eq!(
            records[1].metrics["TerminalBench21Composite"],
            records[1].metrics["AATerminalBench21"]
        );
    }
}
