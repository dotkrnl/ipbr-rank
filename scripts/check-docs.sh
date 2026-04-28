#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCES_DOC="$REPO_ROOT/docs/sources.md"
SOURCES_DIR="$REPO_ROOT/crates/sources/src"

if [ ! -f "$SOURCES_DOC" ]; then
    echo "ERROR: docs/sources.md not found" >&2
    exit 1
fi

# Extract SOURCE_ID values from source implementation files.
source_ids=$(grep 'const SOURCE_ID' "$SOURCES_DIR"/*.rs 2>/dev/null \
    | sed 's/.*"\(.*\)".*/\1/' \
    | sort)

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
    if ! echo "$source_ids" | grep -qx "$section"; then
        echo "WARNING: docs/sources.md has ## $section but no matching SOURCE_ID in sources crate" >&2
    fi
done <<< "$doc_sections"

count=$(echo "$source_ids" | grep -c . || true)

if [ "$errors" -gt 0 ]; then
    echo "FAIL: $errors source(s) missing documentation" >&2
    exit 1
fi

echo "OK: all $count registered sources have documentation"
