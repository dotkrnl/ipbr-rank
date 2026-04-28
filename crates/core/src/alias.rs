use crate::model::{ModelRecord, Vendor};
use std::collections::BTreeMap;

const VENDOR_COLON_PREFIXES: &[&str] =
    &["openai:", "anthropic:", "google:", "moonshotai:", "z.ai:"];

const ORG_ALIASES: &[(&str, &str)] = &[("moonshot ai", "moonshot"), ("z ai", "zai")];

pub fn normalize_vendor_hint(s: &str) -> String {
    normalize_name(s)
}

const KNOWN_SUFFIXES: &[&str] = &[
    "non reasoning",
    "reasoning",
    "thinking",
    "adaptive",
    "preview",
    "latest",
    "default",
    "medium",
    "high",
    "low",
    // Scale & ARC Prize parenthetical effort tags ("(Max)", "(xHigh)")
    "max",
    "xhigh",
];

fn html_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some(end) = s[i..].find(';')
        {
            let entity = &s[i + 1..i + end];
            let replacement = match entity {
                "amp" => Some("&"),
                "lt" => Some("<"),
                "gt" => Some(">"),
                "quot" => Some("\""),
                "apos" | "#39" => Some("'"),
                "nbsp" => Some(" "),
                _ => None,
            };
            if let Some(r) = replacement {
                out.push_str(r);
                i += end + 1;
                continue;
            }
            if let Some(rest) = entity.strip_prefix('#') {
                let n = if let Some(hex) = rest.strip_prefix('x').or_else(|| rest.strip_prefix('X'))
                {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    rest.parse::<u32>().ok()
                };
                if let Some(c) = n.and_then(char::from_u32) {
                    out.push(c);
                    i += end + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

pub fn normalize_name(s: &str) -> String {
    let s = html_unescape(s).to_lowercase();
    let mut s = s.trim().to_string();
    for prefix in VENDOR_COLON_PREFIXES {
        let space = format!("{} ", &prefix[..prefix.len() - 1]);
        s = s.replace(prefix, &space);
    }
    let s: String = s
        .chars()
        .map(|c| if c == '_' || c == '/' { ' ' } else { c })
        .collect();

    let chars: Vec<char> = s.chars().collect();
    let mut buf = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_alphanumeric() || c == ' ' {
            buf.push(c);
        } else if c == '.' {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let next = chars.get(i + 1).copied();
            if matches!(prev, Some(c) if c.is_ascii_digit())
                && matches!(next, Some(c) if c.is_ascii_digit())
            {
                buf.push('.');
            } else {
                buf.push(' ');
            }
        } else {
            buf.push(' ');
        }
    }

    let mut collapsed = String::with_capacity(buf.len());
    let mut last_space = true;
    for c in buf.chars() {
        if c == ' ' {
            if !last_space {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }
    let mut out = collapsed.trim().to_string();
    for (from, to) in ORG_ALIASES {
        out = out.replace(from, to);
    }
    out
}

pub fn compact_key(s: &str) -> String {
    normalize_name(s)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

pub fn strip_known_suffixes(input: &str) -> Vec<String> {
    let mut stripped = Vec::new();
    // Normalize before stripping so spaces, hyphens, underscores, and slashes
    // all use the same token-boundary rules.
    let mut current = normalize_name(input);
    loop {
        let Some(next) = strip_one_known_suffix(&current) else {
            break;
        };
        stripped.push(next.clone());
        current = next;
    }
    stripped
}

fn strip_one_known_suffix(input: &str) -> Option<String> {
    for suffix in KNOWN_SUFFIXES {
        if input == *suffix {
            return None;
        }
        let Some(prefix) = input.strip_suffix(suffix) else {
            continue;
        };
        let Some(prefix) = prefix.strip_suffix(' ') else {
            continue;
        };
        let prefix = prefix.trim_end();
        if !prefix.is_empty() {
            return Some(prefix.to_string());
        }
    }
    None
}

pub struct AliasIndex<'a> {
    by_norm: BTreeMap<String, usize>,
    by_compact: BTreeMap<String, usize>,
    records: &'a [ModelRecord],
}

impl<'a> AliasIndex<'a> {
    pub fn build(records: &'a [ModelRecord]) -> Self {
        let mut by_norm: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_compact: BTreeMap<String, usize> = BTreeMap::new();
        for (idx, r) in records.iter().enumerate() {
            let mut keys = Vec::new();
            keys.push(r.canonical_id.clone());
            keys.push(r.display_name.clone());
            for a in &r.aliases {
                keys.push(a.clone());
            }
            for k in &keys {
                let n = normalize_name(k);
                if !n.is_empty() {
                    by_norm.entry(n).or_insert(idx);
                }
                let c = compact_key(k);
                if !c.is_empty() {
                    by_compact.entry(c).or_insert(idx);
                }
            }
        }
        Self {
            by_norm,
            by_compact,
            records,
        }
    }

    pub fn lookup_exact(&self, input: &str, vendor_hint: Option<&str>) -> Option<usize> {
        let mut candidates = vec![input.to_string()];
        if let Some(v) = vendor_hint
            && !v.is_empty()
        {
            candidates.push(format!("{} {}", v, input));
            candidates.push(format!("{}/{}", v, input));
            candidates.push(format!("{}:{}", v, input));
        }
        for cand in &candidates {
            let n = normalize_name(cand);
            if let Some(&idx) = self.by_norm.get(&n) {
                return Some(idx);
            }
            let c = compact_key(cand);
            if let Some(&idx) = self.by_compact.get(&c) {
                return Some(idx);
            }
        }
        for cand in &candidates {
            for stripped in strip_known_suffixes(cand) {
                if let Some(&idx) = self.by_norm.get(&stripped) {
                    return Some(idx);
                }
                let c: String = stripped
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect();
                if let Some(&idx) = self.by_compact.get(&c) {
                    return Some(idx);
                }
            }
        }
        None
    }

    pub fn match_record(&self, input: &str, vendor_hint: Option<&str>) -> Option<usize> {
        if let Some(idx) = self.lookup_exact(input, vendor_hint) {
            return Some(idx);
        }
        let input_ck = compact_key(input);
        if input_ck.is_empty() {
            return None;
        }
        let threshold = std::cmp::max(12, (input_ck.len() as i32) / 2);

        let mut best: Option<(i32, usize)> = None;
        for (idx, r) in self.records.iter().enumerate() {
            let vendor_bonus = match vendor_hint {
                Some(v) if !v.is_empty() && vendor_matches(&r.vendor, v) => 20,
                _ => 0,
            };
            let mut candidates: Vec<String> = Vec::new();
            candidates.push(r.canonical_id.clone());
            candidates.push(r.display_name.clone());
            for a in &r.aliases {
                candidates.push(a.clone());
            }
            for cand in &candidates {
                let alias_ck = compact_key(cand);
                if alias_ck.is_empty() {
                    continue;
                }
                let score = if alias_ck == input_ck {
                    100 + vendor_bonus
                } else if alias_ck.contains(&input_ck) || input_ck.contains(&alias_ck) {
                    std::cmp::min(input_ck.len(), alias_ck.len()) as i32 + vendor_bonus
                } else {
                    continue;
                };
                if score >= threshold && best.is_none_or(|(s, _)| score > s) {
                    best = Some((score, idx));
                }
            }
        }
        best.map(|(_, idx)| idx)
    }
}

pub fn match_record(
    records: &[ModelRecord],
    input: &str,
    vendor_hint: Option<&str>,
) -> Option<usize> {
    AliasIndex::build(records).match_record(input, vendor_hint)
}

fn vendor_matches(vendor: &Vendor, hint: &str) -> bool {
    let hn = normalize_name(hint);
    let vn = normalize_name(vendor.as_str());
    !hn.is_empty() && hn == vn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelRecord;
    use std::collections::BTreeSet;

    fn rec(id: &str, vendor: Vendor, aliases: &[&str]) -> ModelRecord {
        let mut r = ModelRecord::new(id.to_string(), id.to_string(), vendor);
        r.aliases = aliases
            .iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>();
        r
    }

    #[test]
    fn normalize_preserves_decimal_point() {
        assert_eq!(normalize_name("Claude Opus 4.7"), "claude opus 4.7");
        assert_eq!(normalize_name("gpt-5.5"), "gpt 5.5");
    }

    #[test]
    fn normalize_drops_non_digit_dots() {
        assert_eq!(normalize_name("z.ai/glm"), "zai glm");
        assert_eq!(normalize_name("foo.bar"), "foo bar");
    }

    #[test]
    fn normalize_handles_vendor_colon_and_slash() {
        assert_eq!(normalize_name("openai:gpt-5.5"), "openai gpt 5.5");
        assert_eq!(
            normalize_name("anthropic/claude-opus-4.7"),
            "anthropic claude opus 4.7"
        );
    }

    #[test]
    fn normalize_org_aliases() {
        assert_eq!(normalize_name("Moonshot AI Kimi"), "moonshot kimi");
    }

    #[test]
    fn compact_key_strips_all_separators() {
        assert_eq!(compact_key("Claude Opus 4.7"), "claudeopus47");
        assert_eq!(compact_key("openai/gpt-5.5"), "openaigpt55");
    }

    #[test]
    fn html_unescape_basic() {
        assert_eq!(html_unescape("a &amp; b"), "a & b");
        assert_eq!(html_unescape("&#39;x&#39;"), "'x'");
    }

    #[test]
    fn match_exact_via_alias() {
        let recs = vec![
            rec(
                "anthropic/claude-opus-4.7",
                Vendor::Anthropic,
                &["claude opus 4.7", "claude-opus-4-7"],
            ),
            rec("openai/gpt-5.5", Vendor::Openai, &["gpt-5.5", "gpt 5.5"]),
        ];
        let idx = AliasIndex::build(&recs);
        assert_eq!(idx.match_record("Claude Opus 4.7", None), Some(0));
        assert_eq!(idx.match_record("gpt-5.5", None), Some(1));
    }

    #[test]
    fn match_vendor_prefixed_lookup() {
        let recs = vec![rec(
            "anthropic/claude-opus-4.7",
            Vendor::Anthropic,
            &["claude-opus-4-7"],
        )];
        let idx = AliasIndex::build(&recs);
        assert_eq!(
            idx.match_record("claude-opus-4-7", Some("anthropic")),
            Some(0)
        );
    }

    #[test]
    fn match_fuzzy_substring_with_vendor_bonus() {
        let recs = vec![
            rec(
                "anthropic/claude-opus-4.7",
                Vendor::Anthropic,
                &["claude opus 4.7"],
            ),
            rec("openai/gpt-5.5", Vendor::Openai, &["gpt 5.5"]),
        ];
        let idx = AliasIndex::build(&recs);
        let m = idx.match_record("claude-opus-4-7-thinking", Some("anthropic"));
        assert_eq!(m, Some(0));
    }

    #[test]
    fn lookup_exact_strips_each_known_suffix() {
        let recs = vec![rec("openai/gpt-5.5", Vendor::Openai, &["gpt-5.5"])];
        let idx = AliasIndex::build(&recs);
        for suffix in [
            "latest",
            "preview",
            "thinking",
            "non-reasoning",
            "reasoning",
            "adaptive",
            "high",
            "medium",
            "low",
            "default",
        ] {
            assert_eq!(
                idx.lookup_exact(&format!("gpt-5.5-{suffix}"), Some("openai")),
                Some(0),
                "suffix {suffix} should strip to the canonical alias"
            );
            assert_eq!(
                idx.lookup_exact(&format!("gpt-5.5_{suffix}"), Some("openai")),
                Some(0),
                "underscore suffix {suffix} should strip to the canonical alias"
            );
            assert_eq!(
                idx.lookup_exact(&format!("gpt-5.5 {suffix}"), Some("openai")),
                Some(0),
                "space suffix {suffix} should strip to the canonical alias"
            );
        }
    }

    #[test]
    fn lookup_exact_strips_longest_suffix_first() {
        let recs = vec![
            rec("openai/gpt-5", Vendor::Openai, &["gpt-5"]),
            rec("openai/gpt-5-non", Vendor::Openai, &["gpt-5-non"]),
        ];
        let idx = AliasIndex::build(&recs);
        assert_eq!(
            idx.lookup_exact("gpt-5-non-reasoning", Some("openai")),
            Some(0)
        );
        assert_eq!(
            idx.lookup_exact("gpt-5-non-reasoning-high", Some("openai")),
            Some(0)
        );
    }

    #[test]
    fn lookup_exact_strips_stacked_suffixes() {
        let recs = vec![rec("openai/gpt-5.5", Vendor::Openai, &["gpt-5.5"])];
        let idx = AliasIndex::build(&recs);
        assert_eq!(
            idx.lookup_exact("gpt-5-5-thinking-high", Some("openai")),
            Some(0)
        );
    }

    #[test]
    fn match_record_falls_through_to_fuzzy_when_stripped_form_misses() {
        let recs = vec![rec(
            "example/mystery-preview-x",
            Vendor::Other("example".into()),
            &["acme mystery preview x"],
        )];
        let idx = AliasIndex::build(&recs);
        assert_eq!(idx.match_record("mystery-preview", None), Some(0));
    }

    #[test]
    fn match_below_threshold_returns_none() {
        let recs = vec![rec("openai/gpt-5.5", Vendor::Openai, &["gpt 5.5"])];
        let idx = AliasIndex::build(&recs);
        assert!(idx.match_record("xy", None).is_none());
    }

    #[test]
    fn match_first_record_wins_collision() {
        let recs = vec![
            rec("a/foo", Vendor::Other("a".into()), &["foo"]),
            rec("b/foo", Vendor::Other("b".into()), &["foo"]),
        ];
        let idx = AliasIndex::build(&recs);
        assert_eq!(idx.match_record("foo", None), Some(0));
    }

    #[test]
    fn cache_fixture_matches_are_strict_superset_of_legacy_matching() {
        let records = crate::required_aliases::load_embedded()
            .expect("embedded required aliases should parse");
        let idx = AliasIndex::build(&records);
        let rows = cached_match_inputs();
        assert!(
            !rows.is_empty(),
            "cache fixtures should produce matcher inputs"
        );

        let legacy = legacy_match_pairs(&records, &rows);
        let current: BTreeSet<(String, String)> = rows
            .iter()
            .filter_map(|(source, name, vendor)| {
                idx.match_record(name, vendor.as_deref())
                    .map(|record_idx| (source.clone(), records[record_idx].canonical_id.clone()))
            })
            .collect();

        assert!(
            current.is_superset(&legacy),
            "suffix stripping must not reroute or drop existing cache matches"
        );
        let legacy_exact = legacy_exact_pairs(&records, &rows);
        let current_exact: BTreeSet<(String, String)> = rows
            .iter()
            .filter_map(|(source, name, vendor)| {
                idx.lookup_exact(name, vendor.as_deref())
                    .map(|record_idx| (source.clone(), records[record_idx].canonical_id.clone()))
            })
            .collect();
        assert!(current_exact.is_superset(&legacy_exact));
        // Existing fuzzy matching can already rescue some suffix variants, so
        // strictness is checked at the deterministic exact/compact tier.
        assert!(
            current_exact.len() > legacy_exact.len(),
            "current cache fixtures should gain at least one deterministic suffix-stripped match"
        );
    }

    fn legacy_exact_pairs(
        records: &[ModelRecord],
        rows: &[(String, String, Option<String>)],
    ) -> BTreeSet<(String, String)> {
        rows.iter()
            .filter_map(|(source, name, vendor)| {
                legacy_lookup_exact(records, name, vendor.as_deref())
                    .map(|record_idx| (source.clone(), records[record_idx].canonical_id.clone()))
            })
            .collect()
    }

    fn legacy_match_pairs(
        records: &[ModelRecord],
        rows: &[(String, String, Option<String>)],
    ) -> BTreeSet<(String, String)> {
        rows.iter()
            .filter_map(|(source, name, vendor)| {
                legacy_match_record(records, name, vendor.as_deref())
                    .map(|record_idx| (source.clone(), records[record_idx].canonical_id.clone()))
            })
            .collect()
    }

    fn legacy_match_record(
        records: &[ModelRecord],
        input: &str,
        vendor_hint: Option<&str>,
    ) -> Option<usize> {
        legacy_lookup_exact(records, input, vendor_hint).or_else(|| {
            let input_ck = compact_key(input);
            if input_ck.is_empty() {
                return None;
            }
            let threshold = std::cmp::max(12, (input_ck.len() as i32) / 2);
            let mut best: Option<(i32, usize)> = None;
            for (idx, r) in records.iter().enumerate() {
                let vendor_bonus = match vendor_hint {
                    Some(v) if !v.is_empty() && vendor_matches(&r.vendor, v) => 20,
                    _ => 0,
                };
                let mut candidates = Vec::new();
                candidates.push(r.canonical_id.clone());
                candidates.push(r.display_name.clone());
                candidates.extend(r.aliases.iter().cloned());
                for cand in &candidates {
                    let alias_ck = compact_key(cand);
                    if alias_ck.is_empty() {
                        continue;
                    }
                    let score = if alias_ck == input_ck {
                        100 + vendor_bonus
                    } else if alias_ck.contains(&input_ck) || input_ck.contains(&alias_ck) {
                        std::cmp::min(input_ck.len(), alias_ck.len()) as i32 + vendor_bonus
                    } else {
                        continue;
                    };
                    if score >= threshold && best.is_none_or(|(s, _)| score > s) {
                        best = Some((score, idx));
                    }
                }
            }
            best.map(|(_, idx)| idx)
        })
    }

    fn legacy_lookup_exact(
        records: &[ModelRecord],
        input: &str,
        vendor_hint: Option<&str>,
    ) -> Option<usize> {
        let mut by_norm: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_compact: BTreeMap<String, usize> = BTreeMap::new();
        for (idx, r) in records.iter().enumerate() {
            let mut keys = Vec::new();
            keys.push(r.canonical_id.clone());
            keys.push(r.display_name.clone());
            keys.extend(r.aliases.iter().cloned());
            for key in keys {
                by_norm.entry(normalize_name(&key)).or_insert(idx);
                by_compact.entry(compact_key(&key)).or_insert(idx);
            }
        }

        let mut candidates = vec![input.to_string()];
        if let Some(v) = vendor_hint
            && !v.is_empty()
        {
            candidates.push(format!("{} {}", v, input));
            candidates.push(format!("{}/{}", v, input));
            candidates.push(format!("{}:{}", v, input));
        }
        for cand in &candidates {
            if let Some(&idx) = by_norm.get(&normalize_name(cand)) {
                return Some(idx);
            }
            if let Some(&idx) = by_compact.get(&compact_key(cand)) {
                return Some(idx);
            }
        }
        None
    }

    fn cached_match_inputs() -> Vec<(String, String, Option<String>)> {
        let mut rows = Vec::new();
        rows.extend(lmarena_fixture_inputs());
        rows.extend(openrouter_fixture_inputs());
        rows.extend(swebench_fixture_inputs());
        rows.extend(aistupidlevel_fixture_inputs());
        rows
    }

    fn parse_fixture(payload: &str) -> serde_json::Value {
        serde_json::from_str(payload).expect("cache fixture should parse as JSON")
    }

    fn lmarena_fixture_inputs() -> Vec<(String, String, Option<String>)> {
        let payload = parse_fixture(include_str!("../../../cache/lmarena_overall.json"));
        let mut rows = BTreeSet::new();
        let configs = payload
            .get("configs")
            .and_then(serde_json::Value::as_object)
            .expect("LMArena fixture should contain configs");
        for pages in configs.values().filter_map(serde_json::Value::as_array) {
            for page in pages {
                let Some(page_rows) = page.get("rows").and_then(serde_json::Value::as_array) else {
                    continue;
                };
                for entry in page_rows {
                    let row = entry.get("row").unwrap_or(entry);
                    if row
                        .get("category")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("overall")
                        != "overall"
                    {
                        continue;
                    }
                    let Some(name) = row
                        .get("model_name")
                        .or_else(|| row.get("model"))
                        .or_else(|| row.get("name"))
                        .and_then(serde_json::Value::as_str)
                    else {
                        continue;
                    };
                    let vendor = row
                        .get("organization")
                        .or_else(|| row.get("creator"))
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned);
                    rows.insert(("lmarena".to_string(), name.to_string(), vendor));
                }
            }
        }
        rows.into_iter().collect()
    }

    fn openrouter_fixture_inputs() -> Vec<(String, String, Option<String>)> {
        let payload = parse_fixture(include_str!("../../../cache/openrouter_models.json"));
        payload
            .get("data")
            .and_then(serde_json::Value::as_array)
            .expect("OpenRouter fixture should contain data")
            .iter()
            .filter_map(|item| {
                let name = item
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        item.get("canonical_slug")
                            .and_then(serde_json::Value::as_str)
                    })
                    .or_else(|| item.get("name").and_then(serde_json::Value::as_str))?;
                let vendor = name.split('/').next().filter(|s| !s.is_empty());
                Some((
                    "openrouter".to_string(),
                    name.to_string(),
                    vendor.map(ToOwned::to_owned),
                ))
            })
            .collect()
    }

    fn swebench_fixture_inputs() -> Vec<(String, String, Option<String>)> {
        let payload = parse_fixture(include_str!("../../../cache/swebench_leaderboards.json"));
        let verified = payload
            .get("leaderboards")
            .and_then(serde_json::Value::as_array)
            .expect("SWE-bench fixture should contain leaderboards")
            .iter()
            .find(|board| {
                board
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| {
                        let name = name.to_ascii_lowercase();
                        name == "verified" || name == "swebench-verified"
                    })
            })
            .expect("SWE-bench fixture should contain verified board");
        verified
            .get("results")
            .and_then(serde_json::Value::as_array)
            .expect("SWE-bench verified board should contain results")
            .iter()
            .filter_map(|entry| {
                let name = entry.get("name").and_then(serde_json::Value::as_str)?;
                Some(("swebench".to_string(), extract_swebench_model(name), None))
            })
            .collect()
    }

    fn extract_swebench_model(name: &str) -> String {
        let trimmed = name.trim();
        let without_date = trimmed
            .strip_suffix(')')
            .and_then(|s| s.rsplit_once(" ("))
            .and_then(|(head, date)| {
                (date.len() == 10
                    && date.chars().enumerate().all(|(idx, ch)| {
                        matches!(idx, 4 | 7) && ch == '-'
                            || !matches!(idx, 4 | 7) && ch.is_ascii_digit()
                    }))
                .then_some(head)
            })
            .unwrap_or(trimmed);
        without_date
            .rsplit_once(" + ")
            .map(|(_, model)| model)
            .unwrap_or(without_date)
            .to_string()
    }

    fn aistupidlevel_fixture_inputs() -> Vec<(String, String, Option<String>)> {
        let payload = parse_fixture(include_str!("../../../cache/aistupidlevel_dashboard.json"));
        let data = payload.get("data").unwrap_or(&payload);
        data.get("modelScores")
            .and_then(serde_json::Value::as_array)
            .expect("AIStupidLevel fixture should contain model scores")
            .iter()
            .filter_map(|entry| {
                let name = entry
                    .get("name")
                    .or_else(|| entry.get("model"))
                    .and_then(serde_json::Value::as_str)?
                    .trim();
                if name.is_empty() {
                    return None;
                }
                let vendor = entry
                    .get("vendor")
                    .or_else(|| entry.get("provider"))
                    .and_then(serde_json::Value::as_str)
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty());
                Some(("aistupidlevel".to_string(), name.to_string(), vendor))
            })
            .collect()
    }
}
