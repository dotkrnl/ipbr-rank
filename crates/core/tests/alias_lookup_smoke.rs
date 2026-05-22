use ipbr_core::{alias::AliasIndex, required_aliases};

#[test]
fn lookup_new_2026_05_07_models() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        // (input, vendor_hint, expected canonical_id)
        ("glm-5", Some("zai"), "z-ai/glm-5"),
        ("kimi-k2-5", Some("moonshot"), "moonshotai/kimi-k2.5"),
        ("kimi-k2-5", Some("kimi"), "moonshotai/kimi-k2.5"),
        ("mimo-v2-5-0424", Some("xiaomi"), "xiaomi/mimo-v2.5"),
        ("mimo-v2-5-pro", Some("xiaomi"), "xiaomi/mimo-v2.5-pro"),
        ("minimax-m2-5", Some("minimax"), "minimax/minimax-m2.5"),
        ("minimax-m2-7", Some("minimax"), "minimax/minimax-m2.7"),
        ("qwen3-6-plus", Some("alibaba"), "qwen/qwen3.6-plus"),
    ];
    for &(input, vendor, expected) in cases {
        let matched = idx
            .match_record(input, vendor)
            .map(|i| records[i].canonical_id.as_str());
        assert_eq!(
            matched,
            Some(expected),
            "input={input:?} vendor={vendor:?} matched={matched:?} expected={expected:?}",
        );
    }
}

#[test]
fn lookup_refreshed_source_spellings() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        ("GPT Codex 5.3 High", Some("openai"), "openai/gpt-5.3-codex"),
        ("GLM-4.6 (T=1)", None, "z-ai/glm-4.6"),
        (
            "gpt-5.3-codex (codex-harness)",
            Some("openai"),
            "openai/gpt-5.3-codex",
        ),
        (
            "gpt-5.5-xhigh (codex-harness)",
            Some("openai"),
            "openai/gpt-5.5",
        ),
        (
            "gpt-5.4-medium (codex-harness)",
            Some("openai"),
            "openai/gpt-5.4",
        ),
        (
            "gemini-3-flash (thinking-minimal)",
            Some("google"),
            "google/gemini-3-flash",
        ),
        ("gpt-5.5-search", Some("openai"), "openai/gpt-5.5"),
        (
            "claude-opus-4-6-search",
            Some("anthropic"),
            "anthropic/claude-opus-4.6",
        ),
        (
            "gemini-3-pro-grounding",
            Some("google"),
            "google/gemini-3-pro",
        ),
    ];
    for &(input, vendor, expected) in cases {
        let matched = idx
            .match_record(input, vendor)
            .map(|i| records[i].canonical_id.as_str());
        assert_eq!(
            matched,
            Some(expected),
            "input={input:?} vendor={vendor:?} matched={matched:?} expected={expected:?}",
        );
    }
}

#[test]
fn fuzzy_lookup_rejects_distinct_lmarena_variants() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>)] = &[
        ("gpt-5.4-mini-high", Some("openai")),
        ("gpt-5.4-nano-high", Some("openai")),
        ("gpt-5.5-instant", Some("openai")),
        ("gpt-5.2-codex", Some("openai")),
        ("minimax-m2", Some("minimax")),
        ("glm-4.6v", Some("zai")),
    ];
    for &(input, vendor) in cases {
        let matched = idx
            .match_record(input, vendor)
            .map(|i| records[i].canonical_id.as_str());
        assert_eq!(
            matched, None,
            "input={input:?} vendor={vendor:?} should not match by fuzzy fallback"
        );
    }
}
