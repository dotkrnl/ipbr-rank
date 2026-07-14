#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCES_DOC="$REPO_ROOT/docs/sources.md"
SCHEMA_DOC="$REPO_ROOT/docs/output-schema.md"
SOURCES_DIR="$REPO_ROOT/crates/sources/src"
SCOREBOARD_RS="$REPO_ROOT/crates/core/src/scoreboard.rs"
CLI_MAIN_RS="$REPO_ROOT/crates/cli/src/main.rs"

if [ ! -f "$SOURCES_DOC" ]; then
    echo "ERROR: docs/sources.md not found" >&2
    exit 1
fi

if [ ! -f "$SCHEMA_DOC" ]; then
    echo "ERROR: docs/output-schema.md not found" >&2
    exit 1
fi

# Extract all source IDs, including modules that expose multiple sources under
# named constants, literal `id()` implementations, or evaluation-page configs.
source_ids=$(
    {
        grep -hE 'const [A-Z0-9_]*SOURCE_ID[A-Z0-9_]*:.*"[^"]+"' "$SOURCES_DIR"/*.rs 2>/dev/null \
            | sed 's/.*"\([^"]*\)".*/\1/'
        sed -n '/fn id(/,/^[[:space:]]*}/{s/^[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p;}' "$SOURCES_DIR"/*.rs
        sed -n 's/^[[:space:]]*source_id: "\([^"]*\)",[[:space:]]*$/\1/p' \
            "$SOURCES_DIR/artificial_analysis/evaluations.rs"
    } | sort -u
)

# Extract H2 headings from docs/sources.md.
doc_sections=$(grep '^## ' "$SOURCES_DOC" \
    | sed 's/^## //' \
    | sort)

errors=0

while IFS= read -r id; do
    [ -z "$id" ] && continue
    if ! echo "$doc_sections" | grep -qx "$id"; then
        echo "ERROR: source '$id' is registered but has no ## $id section in docs/sources.md" >&2
        errors=$((errors + 1))
    fi
done <<< "$source_ids"

while IFS= read -r section; do
    [ -z "$section" ] && continue
    case "$section" in
        "Removed sources"|"Evidence precedence") continue ;;
    esac
    if ! echo "$source_ids" | grep -qx "$section"; then
        echo "WARNING: docs/sources.md has ## $section but no matching SOURCE_ID in sources crate" >&2
    fi
done <<< "$doc_sections"

# Keep the public schema reference synchronized with the values emitted by the
# binary. These strings previously drifted during methodology changes while the
# source-list check continued to pass.
schema_version=$(sed -n 's/^pub const SCHEMA_VERSION: &str = "\([^"]*\)";.*/\1/p' "$SCOREBOARD_RS")
methodology=$(sed -n 's/.*methodology: "\([^"]*\)"\.to_string().*/\1/p' "$CLI_MAIN_RS" | head -1)

if [ -z "$schema_version" ]; then
    echo "ERROR: could not read SCHEMA_VERSION from crates/core/src/scoreboard.rs" >&2
    errors=$((errors + 1))
elif ! grep -Fq "schema_version = \"$schema_version\"" "$SCHEMA_DOC"; then
    echo "ERROR: docs/output-schema.md does not document schema_version = \"$schema_version\"" >&2
    errors=$((errors + 1))
fi

if [ -z "$methodology" ]; then
    echo "ERROR: could not read methodology from crates/cli/src/main.rs" >&2
    errors=$((errors + 1))
elif ! grep -Fq "methodology = \"$methodology\"" "$SCHEMA_DOC"; then
    echo "ERROR: docs/output-schema.md does not document methodology = \"$methodology\"" >&2
    errors=$((errors + 1))
fi

count=$(echo "$source_ids" | grep -c . || true)

if [ "$errors" -gt 0 ]; then
    echo "FAIL: $errors source(s) missing documentation" >&2
    exit 1
fi

echo "OK: all $count registered sources have documentation"
