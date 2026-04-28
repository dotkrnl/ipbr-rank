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

echo "==> running pipeline"
./target/release/ipbr-rank \
  --cache cache \
  --out out \
  all \
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
