use crate::alias::AliasIndex;
use crate::model::{ModelRecord, RawRow};

#[derive(Debug, Default, Clone)]
pub struct IngestStats {
    pub matched: usize,
    pub unmatched: Vec<RawRow>,
}

pub fn ingest_rows(records: &mut [ModelRecord], rows: Vec<RawRow>) -> IngestStats {
    let mut stats = IngestStats::default();
    let snapshot: Vec<ModelRecord> = records.to_vec();
    let index = AliasIndex::build(&snapshot);

    let (real_rows, synthesized_rows): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .partition(|row| row.synthesized_from.is_none());

    for row in real_rows {
        ingest_real_row(records, &index, row, &mut stats);
    }
    for row in synthesized_rows {
        ingest_synthesized_row(records, &index, row, &mut stats);
    }

    stats
}

pub fn mark_synthesis_dominant(records: &mut [ModelRecord], per_model_cap: f64) {
    for record in records {
        let total_cells = record.raw_metrics.len();
        let synthesized_cells = record.synthesized.len();
        record.missing.synthesis_dominant =
            total_cells > 0 && (synthesized_cells as f64 / total_cells as f64) > per_model_cap;
    }
}

fn ingest_real_row(
    records: &mut [ModelRecord],
    index: &AliasIndex<'_>,
    row: RawRow,
    stats: &mut IngestStats,
) {
    match index.match_record(&row.model_name, row.vendor_hint.as_deref()) {
        Some(i) => {
            let record = &mut records[i];
            record.sources.insert(row.source_id);
            for (key, value) in row.fields {
                if let Some(num) = json_to_f64(&value) {
                    record.raw_metrics.insert(key.clone(), num);
                    record.synthesized.remove(&key);
                }
            }
            stats.matched += 1;
        }
        None => stats.unmatched.push(row),
    }
}

fn ingest_synthesized_row(
    records: &mut [ModelRecord],
    index: &AliasIndex<'_>,
    row: RawRow,
    stats: &mut IngestStats,
) {
    match index.match_record(&row.model_name, row.vendor_hint.as_deref()) {
        Some(i) => {
            let record = &mut records[i];
            let from = row
                .synthesized_from
                .clone()
                .expect("synthesized rows must carry synthesized_from");
            for (key, value) in row.fields {
                if record.raw_metrics.contains_key(&key) {
                    continue;
                }
                if let Some(num) = json_to_f64(&value) {
                    record.raw_metrics.insert(key.clone(), num);
                    record.synthesized.insert(
                        key,
                        crate::model::SynthesisProvenance {
                            source_id: row.source_id.clone(),
                            from: from.clone(),
                        },
                    );
                }
            }
            stats.matched += 1;
        }
        None => stats.unmatched.push(row),
    }
}

fn json_to_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Vendor;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn raw(source: &str, name: &str, fields: &[(&str, serde_json::Value)]) -> RawRow {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert(k.to_string(), v.clone());
        }
        RawRow {
            source_id: source.to_string(),
            model_name: name.to_string(),
            vendor_hint: None,
            fields: map,
            synthesized_from: None,
        }
    }

    #[test]
    fn matched_row_populates_raw_metrics() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5.5".to_string());
            r
        }];
        let rows = vec![raw(
            "openrouter",
            "gpt-5.5",
            &[
                ("ContextWindow", json!(128000)),
                ("OutputSpeed", json!(75.5)),
            ],
        )];
        let stats = ingest_rows(&mut records, rows);
        assert_eq!(stats.matched, 1);
        assert!(stats.unmatched.is_empty());
        assert_eq!(records[0].raw_metrics.get("ContextWindow"), Some(&128000.0));
        assert_eq!(records[0].raw_metrics.get("OutputSpeed"), Some(&75.5));
        assert!(records[0].sources.contains("openrouter"));
    }

    #[test]
    fn unmatched_row_collected_for_review() {
        let mut records: Vec<ModelRecord> = vec![];
        let rows = vec![raw("foo", "totally-unknown-model-zzz", &[])];
        let stats = ingest_rows(&mut records, rows);
        assert_eq!(stats.matched, 0);
        assert_eq!(stats.unmatched.len(), 1);
    }
}
