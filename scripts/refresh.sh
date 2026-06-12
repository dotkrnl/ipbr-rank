#!/usr/bin/env bash
# Run the full ipbr-rank pipeline (fetch → score → render) with live data,
# and optionally publish the rendered site to Cloudflare Pages.
#
# Usage:
#   scripts/refresh.sh             # default: fetch all sources, render to out/
#   scripts/refresh.sh --offline   # use cached responses only
#   scripts/refresh.sh --only artificial_analysis,lmarena
#   scripts/refresh.sh --open      # open out/site/index.html when done
#   scripts/refresh.sh --publish   # also deploy out/site to Cloudflare Pages
#
# Reads from .env:
#   AA_API_KEY, OPENROUTER_API_KEY, HF_TOKEN — pipeline source credentials
#   CLOUDFLARE_ACCOUNT_ID                    — required for --publish
#   CLOUDFLARE_PAGES_PROJECT (optional, default "ipbr")
#
# Cloudflare deploy uses the wrangler CLI via npx. Wrangler must already
# be authenticated (`npx wrangler login`) — this script does NOT prompt.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

open_when_done=false
publish=false
forwarded=()
for arg in "$@"; do
  case "$arg" in
    --open)    open_when_done=true ;;
    --publish) publish=true ;;
    *) forwarded+=("$arg") ;;
  esac
done

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

for var in AA_API_KEY OPENROUTER_API_KEY HF_TOKEN; do
  if [[ -z "${!var:-}" ]]; then
    echo "warning: $var is not set — sources that depend on it will degrade" >&2
  fi
done

echo "==> building ipbr-rank-cli (release)"
cargo build --release -p ipbr-rank-cli

# Daily snapshot: the delta indicator compares against yesterday's
# scoreboard, not the last ~10-min refresh. On the first successful
# run of each day, the live scoreboard is saved as a dated snapshot;
# subsequent runs reuse it so every refresh shares the same baseline.
mkdir -p cache
prev_args=()
today="$(date -u +%Y-%m-%d)"
live="cache/prev_scoreboard.toml"
snapshot="cache/daily_snapshot_${today}.toml"

fetched=false
if curl -fsS --max-time 10 \
    "https://ipbr.pages.dev/scoreboard.toml" \
    -o "$live" 2>/dev/null; then
  fetched=true
  echo "fetched live scoreboard ($(wc -c < "$live") bytes)"
else
  rm -f "$live"
  echo "note: live scoreboard fetch failed"
fi

# Create today's snapshot if it doesn't exist yet (first run of the day).
if [[ ! -f "$snapshot" ]] && $fetched; then
  cp "$live" "$snapshot"
  echo "created daily snapshot for $today"
fi

# Find the most recent daily snapshot that is NOT today's.
yesterday_snapshot="$(ls -t cache/daily_snapshot_*.toml 2>/dev/null \
  | grep -v "$today" | head -1)"
if [[ -n "$yesterday_snapshot" ]]; then
  prev_args=(--prev "$yesterday_snapshot")
  echo "using daily snapshot: $yesterday_snapshot"
else
  echo "note: no prior-day snapshot found; deltas will be omitted this run"
fi

# Prune snapshots older than 7 days.
find cache -name 'daily_snapshot_*.toml' -mtime +7 -delete 2>/dev/null || true

echo "==> running pipeline"
./target/release/ipbr-rank \
  --cache cache \
  --out out \
  all \
  ${prev_args[@]+"${prev_args[@]}"} \
  ${forwarded[@]+"${forwarded[@]}"}

echo
echo "done."
echo "  scoreboard:  out/scoreboard.toml"
echo "  site:        out/site/index.html"

if $publish; then
  if [[ -z "${CLOUDFLARE_ACCOUNT_ID:-}" ]]; then
    echo "error: --publish requires CLOUDFLARE_ACCOUNT_ID in .env" >&2
    exit 2
  fi
  project="${CLOUDFLARE_PAGES_PROJECT:-ipbr}"
  echo "==> publishing to Cloudflare Pages (project=$project)"
  npx --yes wrangler pages deploy out/site \
    --project-name="$project" \
    --branch=main \
    --commit-dirty=true
fi

if $open_when_done; then
  case "$(uname -s)" in
    Darwin) open out/site/index.html ;;
    Linux)  xdg-open out/site/index.html >/dev/null 2>&1 || true ;;
    *)      echo "(open the site manually: out/site/index.html)" ;;
  esac
fi
