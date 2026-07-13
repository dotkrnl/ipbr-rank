use ipbr_core::{alias::AliasIndex, required_aliases};

#[test]
fn lookup_gpt_56_family() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        ("gpt-5.6", Some("openai"), "openai/gpt-5.6-sol"),
        (
            "gpt-5.6-sol-xhigh (codex-harness)",
            Some("openai"),
            "openai/gpt-5.6-sol",
        ),
        (
            "GPT-5.6 Terra (max)",
            Some("openai"),
            "openai/gpt-5.6-terra",
        ),
        (
            "gpt-5-6-luna-non-reasoning",
            Some("openai"),
            "openai/gpt-5.6-luna",
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
fn lookup_new_2026_05_07_models() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        // (input, vendor_hint, expected canonical_id)
        ("glm-5", Some("zai"), "z-ai/glm-5"),
        (
            "kimi-k2-7-code",
            Some("moonshot"),
            "moonshotai/kimi-k2.7-code",
        ),
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
fn lookup_2026_05_22_models() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        (
            "gemini-3-5-flash",
            Some("google"),
            "google/gemini-3.5-flash",
        ),
        ("qwen3.7-max-preview", Some("alibaba"), "qwen/qwen3.7-max"),
        ("grok-4-3", Some("xai"), "xai/grok-4.3"),
        ("muse-spark", Some("meta"), "meta/muse-spark"),
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
fn lookup_2026_06_12_models() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        ("gpt-5.2-codex", Some("openai"), "openai/gpt-5.2-codex"),
        ("gpt-5.4-mini-high", Some("openai"), "openai/gpt-5.4-mini"),
        (
            "deepseek-v3-2-reasoning-0925",
            Some("deepseek"),
            "deepseek/deepseek-v3.2",
        ),
        (
            "gemini-3-1-flash-lite",
            Some("google"),
            "google/gemini-3.1-flash-lite",
        ),
        ("grok-4.20-0309-reasoning", Some("xai"), "xai/grok-4.20"),
        (
            "qwen3-6-max-preview",
            Some("alibaba"),
            "qwen/qwen3.6-max-preview",
        ),
        (
            "qwen3-5-397b-a17b",
            Some("alibaba"),
            "qwen/qwen3.5-397b-a17b",
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
fn lookup_claude_fable_5() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        (
            "Claude Fable 5",
            Some("anthropic"),
            "anthropic/claude-fable-5",
        ),
        (
            "anthropic/claude-5-fable-20260609",
            Some("anthropic"),
            "anthropic/claude-fable-5",
        ),
        (
            "Claude Fable 5 (Adaptive Reasoning, Max Effort, Opus 4.8 Fallback)",
            Some("anthropic"),
            "anthropic/claude-fable-5",
        ),
        (
            "Fable-5 (Claude Code) xHigh",
            None,
            "anthropic/claude-fable-5",
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
fn lookup_claude_sonnet_5_and_haiku_45() {
    let records = required_aliases::load_embedded().unwrap();
    let idx = AliasIndex::build(&records);
    let cases: &[(&str, Option<&str>, &str)] = &[
        (
            "anthropic/claude-sonnet-5-20260630",
            Some("anthropic"),
            "anthropic/claude-sonnet-5",
        ),
        (
            "Claude Sonnet 5 (Adaptive Reasoning, Max Effort)",
            Some("anthropic"),
            "anthropic/claude-sonnet-5",
        ),
        (
            "claude-sonnet-5-non-reasoning",
            Some("anthropic"),
            "anthropic/claude-sonnet-5",
        ),
        (
            "anthropic/claude-4.5-haiku-20251001",
            Some("anthropic"),
            "anthropic/claude-haiku-4.5",
        ),
        (
            "Claude 4.5 Haiku (Reasoning)",
            Some("anthropic"),
            "anthropic/claude-haiku-4.5",
        ),
        (
            "claude-haiku-4-5-20251001-thinking-16k",
            Some("anthropic"),
            "anthropic/claude-haiku-4.5",
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
        ("gpt-5.4-nano-high", Some("openai")),
        ("gpt-5.5-instant", Some("openai")),
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
