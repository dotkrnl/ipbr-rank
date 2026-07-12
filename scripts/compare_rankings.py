#!/usr/bin/env python3
"""Compare two ipbr scoreboard TOMLs and emit a Markdown rank-change report."""

from __future__ import annotations

import argparse
import pathlib
import tomllib


ROLES = (
    ("balanced", "Balanced capability"),
    ("i_raw", "Idea"),
    ("p_raw", "Plan"),
    ("b_raw", "Build"),
    ("r", "Review proxy"),
)

# Match the site's one-decimal display: values within half a displayed point
# share a dense rank (1, 1, 2 rather than 1, 2, 3).
DISPLAY_TIE_EPSILON = 0.05


def load(path: pathlib.Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def score(model: dict, role: str) -> float:
    scores = model["scores"]
    if role == "balanced":
        return sum(float(scores[key]) for key in ("i_raw", "p_raw", "b_raw", "r")) / 4.0
    return float(scores[role])


def is_provisional(model: dict, role: str) -> bool:
    roles = model.get("evidence", {}).get("roles", {})
    if role == "balanced":
        return any(
            bool(roles.get(key, {}).get("provisional", False))
            for key in ("I_raw", "P_raw", "B_raw", "R")
        )
    evidence_key = {"i_raw": "I_raw", "p_raw": "P_raw", "b_raw": "B_raw", "r": "R"}[role]
    return bool(roles.get(evidence_key, {}).get("provisional", False))


def dense_ranks(
    models: list[dict], role: str, *, eligible_only: bool = False
) -> tuple[dict[str, int], dict[str, float]]:
    scores = {model["canonical_id"]: score(model, role) for model in models}
    if eligible_only:
        models = [model for model in models if not is_provisional(model, role)]
    ordered = sorted(models, key=lambda model: (-score(model, role), model["canonical_id"]))
    ranks: dict[str, int] = {}
    previous: float | None = None
    rank = 0
    for model in ordered:
        value = score(model, role)
        if previous is None or abs(value - previous) >= DISPLAY_TIE_EPSILON:
            rank += 1
            previous = value
        canonical_id = model["canonical_id"]
        ranks[canonical_id] = rank
    return ranks, scores


def report(before: dict, after: dict, top: int) -> str:
    before_by_id = {model["canonical_id"]: model for model in before["models"]}
    after_by_id = {model["canonical_id"]: model for model in after["models"]}
    common = set(before_by_id) & set(after_by_id)
    lines = [
        "# Ranking changes",
        "",
        f"Before: `{before.get('generated_at', 'unknown')}` / `{before.get('methodology', 'unknown')}`  ",
        f"After: `{after.get('generated_at', 'unknown')}` / `{after.get('methodology', 'unknown')}`",
        "",
        "Dense ties use the displayed 0.1-point precision. Official ranks exclude provisional models; estimate position shows score order across every model.",
    ]
    for role, label in ROLES:
        before_ranks, before_scores = dense_ranks(
            list(before_by_id.values()), role, eligible_only=True
        )
        after_ranks, after_scores = dense_ranks(
            list(after_by_id.values()), role, eligible_only=True
        )
        estimate_positions, _ = dense_ranks(list(after_by_id.values()), role)
        selected = sorted(
            common,
            key=lambda canonical_id: (
                min(
                    before_ranks.get(canonical_id, 10**9),
                    estimate_positions[canonical_id],
                ),
                estimate_positions[canonical_id],
                canonical_id,
            ),
        )
        selected = [
            canonical_id
            for canonical_id in selected
            if before_ranks.get(canonical_id, 10**9) <= top
            or estimate_positions[canonical_id] <= top
        ]
        lines.extend(
            (
                "",
                f"## {label}",
                "",
                "| Model | Official before | Official after | Estimate position | Change | Score before | Score after | Delta | Status |",
                "|---|---:|---:|---:|---:|---:|---:|---:|---|",
            )
        )
        for canonical_id in selected:
            old_rank = before_ranks.get(canonical_id)
            new_rank = after_ranks.get(canonical_id)
            old_label = str(old_rank) if old_rank is not None else "—"
            new_label = str(new_rank) if new_rank is not None else "—"
            movement = f"{old_rank - new_rank:+d}" if old_rank and new_rank else "—"
            status = "provisional" if is_provisional(after_by_id[canonical_id], role) else "ranked"
            lines.append(
                f"| `{canonical_id}` | {old_label} | {new_label} | "
                f"{estimate_positions[canonical_id]} | {movement} | "
                f"{before_scores[canonical_id]:.2f} | {after_scores[canonical_id]:.2f} | "
                f"{after_scores[canonical_id] - before_scores[canonical_id]:+.2f} | {status} |"
            )
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("before", type=pathlib.Path)
    parser.add_argument("after", type=pathlib.Path)
    parser.add_argument("--top", type=int, default=20)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()
    rendered = report(load(args.before), load(args.after), max(args.top, 1))
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        print(rendered, end="")


if __name__ == "__main__":
    main()
