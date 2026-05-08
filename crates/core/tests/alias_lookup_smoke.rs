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
