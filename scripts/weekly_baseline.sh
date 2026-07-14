#!/usr/bin/env bash
# Resolve the baseline the site's rank-change chips compare against: the
# published scoreboard as it stood at the start of *last* week. Weeks open on
# Sunday, so on Tue 2026-07-14 the baseline is the snapshot frozen on Sun
# 2026-07-05. Every refresh within a week therefore shares one baseline, and it
# only moves when a new week opens.
#
# Snapshots live in cache/week_<sunday>.toml, keyed by the week they open, and
# are frozen from the live scoreboard on the first run of that week.
#
# Prints the baseline path on stdout — empty when none is available, in which
# case the caller omits --prev and the site renders no rank changes. All notes
# go to stderr so stdout stays a clean path.
#
# Usage: scripts/weekly_baseline.sh [CACHE_DIR]   (default: cache)

set -euo pipefail

cache_dir="${1:-cache}"
live_url="https://ipbr.pages.dev/scoreboard.toml"

mkdir -p "$cache_dir"

# GNU date spells epoch input `-d @EPOCH`, BSD date spells it `-r EPOCH`; CI is
# the former and macOS the latter. Everything is UTC, so a day is exactly 86400
# seconds and no DST correction is needed.
epoch_to_date() {
  date -u -d "@$1" +%F 2>/dev/null || date -u -r "$1" +%F
}

now="$(date -u +%s)"
dow="$(date -u +%w)" # 0 = Sunday
this_week="$(epoch_to_date "$((now - dow * 86400))")"
last_week="$(epoch_to_date "$((now - (dow + 7) * 86400))")"

current="$cache_dir/week_${this_week}.toml"
baseline="$cache_dir/week_${last_week}.toml"

live="$(mktemp)"
trap 'rm -f "$live"' EXIT

if curl -fsS --max-time 10 "$live_url" -o "$live"; then
  echo "fetched live scoreboard ($(wc -c <"$live" | tr -d ' ') bytes)" >&2

  # First run of the week freezes this week's opening snapshot. It sits unused
  # until next Sunday, when it becomes the baseline.
  if [[ ! -f "$current" ]]; then
    cp "$live" "$current"
    echo "froze week-opening snapshot for $this_week" >&2
  fi

  # No snapshot from last week — either this scheme is new or the cache was
  # lost. Seed the baseline from today so rank changes start accruing from now
  # (they read as zero until the board moves) instead of vanishing for a week.
  if [[ ! -f "$baseline" ]]; then
    cp "$live" "$baseline"
    echo "seeded the missing $last_week baseline from today's scoreboard" >&2
  fi
else
  echo "note: live scoreboard fetch failed" >&2
fi

# Prune by week, never by mtime: the baseline is up to 15 days old, so an mtime
# sweep would delete the very file the rank changes are measured against.
for snapshot in "$cache_dir"/week_*.toml; do
  [[ -e "$snapshot" ]] || continue
  week="$(basename "$snapshot" .toml)"
  week="${week#week_}"
  if [[ "$week" < "$last_week" ]]; then
    rm -f "$snapshot"
  fi
done

if [[ -f "$baseline" ]]; then
  echo "$baseline"
else
  echo "note: no baseline snapshot; rank changes are omitted this run" >&2
fi
