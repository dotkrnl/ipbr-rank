use crate::model::{ModelRecord, Vendor};
use std::collections::{BTreeMap, BTreeSet};

const VENDOR_COLON_PREFIXES: &[&str] =
    &["openai:", "anthropic:", "google:", "moonshotai:", "z.ai:"];

const ORG_ALIASES: &[(&str, &str)] = &[
    ("moonshot ai", "moonshot"),
    // OpenRouter/canonical IDs use the attached form `moonshotai/...`, which
    // has no separator to split, so without this the org never normalizes to
    // the `moonshot` vendor enum and Moonshot models lose the fuzzy-match
    // vendor bonus.
    ("moonshotai", "moonshot"),
    ("z ai", "zai"),
];

pub fn normalize_vendor_hint(s: &str) -> String {
    normalize_name(s)
}

const KNOWN_SUFFIXES: &[&str] = &[
    "non reasoning",
    "reasoning",
    "thinking",
    "adaptive",
    "default",
    "medium",
    "high",
    "low",
    // Scale & ARC Prize parenthetical effort tags ("(Max)", "(xHigh)")
    "max",
    "xhigh",
];

const DISTINCT_VARIANT_TOKENS: &[&str] = &[
    "beta", "chat", "codex", "fast", "flash", "image", "instant", "lite", "mini", "minimal",
    "multi", "nano", "latest", "preview", "pro", "turbo", "vision",
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
    while let Some(next) = strip_one_known_suffix(&current) {
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
                if alias_ck != input_ck && !fuzzy_variant_match_allowed(input, cand) {
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

fn fuzzy_variant_match_allowed(input: &str, candidate: &str) -> bool {
    let input_ck = compact_key(input);
    let candidate_ck = compact_key(candidate);
    if input_ck.is_empty() || candidate_ck.is_empty() {
        return false;
    }

    if !input_ck.contains(&candidate_ck) {
        if candidate_extends_input_with_digit(&input_ck, &candidate_ck) {
            // `minimax-m2` is not `minimax-m2.5`, and `gemini-pro` is not a
            // dated/generation-specific Gemini Pro entry.
            return false;
        }
        return candidate_ck.contains(&input_ck);
    }

    // Vision-style suffixes such as `glm-4.6v`/`glm-4-6v-reasoning` are
    // distinct models, not harmless effort or endpoint tags for `glm-4.6`.
    if input_extends_candidate_with_char(&input_ck, &candidate_ck, 'v') {
        return false;
    }
    if input_extends_candidate_with_digit(&input_ck, &candidate_ck) {
        return false;
    }

    let input_norm = normalize_name(input);
    let candidate_norm = normalize_name(candidate);
    if has_attached_v_variant(&input_norm, &candidate_norm) {
        return false;
    }
    for token in DISTINCT_VARIANT_TOKENS {
        if has_token(&input_norm, token) && !has_token(&candidate_norm, token) {
            return false;
        }
    }
    true
}

fn has_token(normalized: &str, token: &str) -> bool {
    normalized.split_whitespace().any(|part| part == token)
}

fn has_attached_v_variant(input: &str, candidate: &str) -> bool {
    input
        .split_whitespace()
        .zip(candidate.split_whitespace())
        .any(|(input_token, candidate_token)| {
            input_token.len() == candidate_token.len() + 1
                && input_token
                    .strip_suffix('v')
                    .is_some_and(|base| base == candidate_token)
        })
}

fn candidate_extends_input_with_digit(input_ck: &str, candidate_ck: &str) -> bool {
    let mut offset = 0;
    while let Some(found) = candidate_ck[offset..].find(input_ck) {
        let end = offset + found + input_ck.len();
        if candidate_ck[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return true;
        }
        offset = end;
    }
    false
}

fn input_extends_candidate_with_digit(input_ck: &str, candidate_ck: &str) -> bool {
    let mut offset = 0;
    while let Some(found) = input_ck[offset..].find(candidate_ck) {
        let end = offset + found + candidate_ck.len();
        if input_ck[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return true;
        }
        offset = end;
    }
    false
}

fn input_extends_candidate_with_char(input_ck: &str, candidate_ck: &str, suffix: char) -> bool {
    let mut offset = 0;
    while let Some(found) = input_ck[offset..].find(candidate_ck) {
        let end = offset + found + candidate_ck.len();
        if input_ck[end..].starts_with(suffix) {
            return true;
        }
        offset = end;
    }
    false
}

pub fn match_record(
    records: &[ModelRecord],
    input: &str,
    vendor_hint: Option<&str>,
) -> Option<usize> {
    AliasIndex::build(records).match_record(input, vendor_hint)
}

/// A normalized or compact alias key claimed by two distinct canonical models.
/// `AliasIndex` resolves these first-record-wins and silently, so a colliding
/// alias reroutes another model's benchmark rows with no other signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasCollision {
    pub kind: &'static str,
    pub key: String,
    pub first: String,
    pub second: String,
}

/// Detect alias keys shared across distinct canonical models. Emits a CRITICAL
/// warning per collision to stderr (no logging dependency in `ipbr-core`) and
/// returns them so the CLI can fail loudly and tests can assert on the set.
pub fn warn_alias_collisions(records: &[ModelRecord]) -> Vec<AliasCollision> {
    let mut normalized: BTreeMap<String, usize> = BTreeMap::new();
    let mut compact: BTreeMap<String, usize> = BTreeMap::new();
    let mut collisions = Vec::new();
    for (idx, record) in records.iter().enumerate() {
        let keys = std::iter::once(record.canonical_id.as_str())
            .chain(std::iter::once(record.display_name.as_str()))
            .chain(record.aliases.iter().map(String::as_str));
        // Dedupe keys within a record so a model that lists the same alias
        // twice (e.g. canonical_id == display_name) doesn't self-collide.
        let mut seen_here: BTreeSet<(&'static str, String)> = BTreeSet::new();
        for key in keys {
            for (kind, candidate, seen) in [
                ("normalized", normalize_name(key), &mut normalized),
                ("compact", compact_key(key), &mut compact),
            ] {
                if candidate.is_empty() || !seen_here.insert((kind, candidate.clone())) {
                    continue;
                }
                match seen.get(&candidate) {
                    Some(&other) if other != idx => {
                        let collision = AliasCollision {
                            kind,
                            key: candidate.clone(),
                            first: records[other].canonical_id.clone(),
                            second: record.canonical_id.clone(),
                        };
                        eprintln!(
                            "CRITICAL: {kind} alias key {:?} is shared by {} and {} — benchmark rows may be attributed to the wrong model; disambiguate in data/required_aliases.toml",
                            collision.key, collision.first, collision.second
                        );
                        collisions.push(collision);
                    }
                    Some(_) => {}
                    None => {
                        seen.insert(candidate, idx);
                    }
                }
            }
        }
    }
    collisions
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
        // The attached OpenRouter/canonical org spelling normalizes to the
        // same `moonshot` vendor token as the spaced form.
        assert_eq!(normalize_name("moonshotai/kimi-k2.7"), "moonshot kimi k2.7");
        assert!(vendor_matches(&Vendor::Moonshot, "moonshotai"));
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
    fn lifecycle_suffixes_require_explicit_aliases() {
        let recs = vec![
            rec("openai/gpt-5.5", Vendor::Openai, &["gpt-5.5"]),
            rec(
                "google/gemini-3-flash-preview",
                Vendor::Google,
                &["gemini-3-flash-preview"],
            ),
            rec(
                "anthropic/claude-fable-5",
                Vendor::Anthropic,
                &["claude-fable-5", "claude-fable-latest"],
            ),
        ];
        let idx = AliasIndex::build(&recs);

        // A lifecycle label can denote a different build or a moving route.
        // It must never be collapsed to an otherwise similar stable name.
        assert_eq!(idx.lookup_exact("gpt-5.5-preview", Some("openai")), None);
        assert_eq!(idx.match_record("gpt-5.5-preview", Some("openai")), None);
        assert_eq!(idx.lookup_exact("gpt-5.5-latest", Some("openai")), None);
        assert_eq!(idx.match_record("gpt-5.5-latest", Some("openai")), None);

        // Source-audited lifecycle spellings remain available as explicit
        // aliases on the intended canonical record.
        assert_eq!(
            idx.lookup_exact("gemini-3-flash-preview", Some("google")),
            Some(1)
        );
        assert_eq!(
            idx.lookup_exact("claude-fable-latest", Some("anthropic")),
            Some(2)
        );
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
            "example/mystery-preview",
            Vendor::Other("example".into()),
            &["mystery preview"],
        )];
        let idx = AliasIndex::build(&recs);
        assert_eq!(idx.match_record("acme-mystery-preview", None), Some(0));
    }

    #[test]
    fn match_record_rejects_distinct_variants_without_explicit_alias() {
        let recs = vec![
            rec("openai/gpt-5.4", Vendor::Openai, &["gpt-5.4"]),
            rec("openai/gpt-5.2", Vendor::Openai, &["gpt-5.2"]),
            rec(
                "minimax/minimax-m2.5",
                Vendor::Other("minimax".into()),
                &["minimax-m2.5"],
            ),
            rec("z-ai/glm-4.6", Vendor::Zai, &["glm-4.6"]),
            rec("xai/grok-4.20", Vendor::Xai, &["x-ai/grok-4.20"]),
        ];
        let idx = AliasIndex::build(&recs);
        for (input, vendor) in [
            ("gpt-5.4-chat", Some("openai")),
            ("gpt-5.4-mini-high", Some("openai")),
            ("gpt-5.4-nano-high", Some("openai")),
            ("gpt-5.4-instant", Some("openai")),
            ("gpt-5.4-preview-2026-01-01", Some("openai")),
            ("gpt-5.2-codex", Some("openai")),
            ("gpt-5.2-turbo", Some("openai")),
            ("minimax-m2", Some("minimax")),
            ("grok-4.1", Some("xai")),
            ("grok-4.20-beta1", Some("xai")),
            ("glm-4.6-flash", Some("zai")),
            ("glm-4.6-image", Some("zai")),
            ("glm-4.6v", Some("zai")),
            ("glm-4-6v-reasoning", Some("zai")),
            ("glm-4.6v-turbo", Some("zai")),
            ("x-ai/grok-4.20-multi-agent", Some("xai")),
            ("x-ai/grok-4.20-multi-agent-20260309", Some("xai")),
        ] {
            assert!(
                idx.match_record(input, vendor).is_none(),
                "{input} should require an explicit alias"
            );
        }
    }

    #[test]
    fn match_below_threshold_returns_none() {
        let recs = vec![rec("openai/gpt-5.5", Vendor::Openai, &["gpt 5.5"])];
        let idx = AliasIndex::build(&recs);
        assert!(idx.match_record("xy", None).is_none());
    }

    #[test]
    fn warn_alias_collisions_flags_shared_keys_only() {
        let clean = vec![
            rec("openai/gpt-5.5", Vendor::Openai, &["gpt-5.5"]),
            rec(
                "anthropic/claude-opus-4.7",
                Vendor::Anthropic,
                &["opus-4.7"],
            ),
        ];
        assert!(warn_alias_collisions(&clean).is_empty());

        // Second model aliases a key that normalizes/compacts to the first.
        let colliding = vec![
            rec("openai/gpt-5.5", Vendor::Openai, &["gpt-5.5"]),
            rec("acme/clone", Vendor::Other("acme".into()), &["gpt 5.5"]),
        ];
        let collisions = warn_alias_collisions(&colliding);
        assert!(
            collisions
                .iter()
                .any(|c| c.first == "openai/gpt-5.5" && c.second == "acme/clone"),
            "{collisions:?}"
        );
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
}
