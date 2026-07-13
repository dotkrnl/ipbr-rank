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


def display_score_key(value: float) -> str:
    """Use the same one-decimal key shown on the leaderboard."""
    return format(value, ".1f")


def load(path: pathlib.Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def score(model: dict, role: str) -> float:
    scores = model["scores"]
    if role == "balanced":
        return sum(float(scores[key]) for key in ("i_raw", "p_raw", "b_raw", "r")) / 4.0
    return float(scores[role])


def is_provisional(model: dict, role: str) -> bool:
    scores = model.get("scores", {})
    roles = model.get("evidence", {}).get("roles", {})
    if role == "balanced":
        if "balanced_status" in scores:
            return scores["balanced_status"] == "provisional"
        # Compatibility with methodology-v2 snapshots, which used an
        # all-four-role veto and did not serialize Balanced status.
        return any(
            bool(roles.get(key, {}).get("provisional", False))
            for key in ("I_raw", "P_raw", "B_raw", "R")
        )
    evidence_key = {"i_raw": "I_raw", "p_raw": "P_raw", "b_raw": "B_raw", "r": "R"}[role]
    return bool(roles.get(evidence_key, {}).get("provisional", False))


def dense_ranks(models: list[dict], role: str) -> tuple[dict[str, int], dict[str, float]]:
    scores = {model["canonical_id"]: score(model, role) for model in models}
    ordered = sorted(models, key=lambda model: (-score(model, role), model["canonical_id"]))
    ranks: dict[str, int] = {}
    previous: str | None = None
    rank = 0
    for model in ordered:
        value = score(model, role)
        display_key = display_score_key(value)
        if previous != display_key:
            rank += 1
            previous = display_key
        canonical_id = model["canonical_id"]
        ranks[canonical_id] = rank
    return ranks, scores


def report(before: dict, after: dict, top: int) -> str:
    before_by_id = {model["canonical_id"]: model for model in before["models"]}
    after_by_id = {model["canonical_id"]: model for model in after["models"]}
    all_ids = set(before_by_id) | set(after_by_id)
    lines = [
        "# Ranking changes",
        "",
        f"Before: `{before.get('generated_at', 'unknown')}` / `{before.get('methodology', 'unknown')}`  ",
        f"After: `{after.get('generated_at', 'unknown')}` / `{after.get('methodology', 'unknown')}`",
        "",
        "Dense ties use the displayed 0.1-point precision. Added and removed models are shown explicitly; provisional status is reported separately.",
    ]
    for role, label in ROLES:
        before_ranks, before_scores = dense_ranks(list(before_by_id.values()), role)
        after_ranks, after_scores = dense_ranks(list(after_by_id.values()), role)
        selected = sorted(
            all_ids,
            key=lambda canonical_id: (
                min(
                    before_ranks.get(canonical_id, len(before_ranks) + 1),
                    after_ranks.get(canonical_id, len(after_ranks) + 1),
                ),
                after_ranks.get(canonical_id, len(after_ranks) + 1),
                canonical_id,
            ),
        )
        selected = [
            canonical_id
            for canonical_id in selected
            if before_ranks.get(canonical_id, top + 1) <= top
            or after_ranks.get(canonical_id, top + 1) <= top
        ]
        lines.extend(
            (
                "",
                f"## {label}",
                "",
                "| Model | Rank before | Rank after | Rank Δ | Score before | Score after | Score Δ | Status before → after |",
                "|---|---:|---:|---:|---:|---:|---:|---|",
            )
        )
        for canonical_id in selected:
            old_rank = before_ranks.get(canonical_id)
            new_rank = after_ranks.get(canonical_id)
            old_score = before_scores.get(canonical_id)
            new_score = after_scores.get(canonical_id)
            if old_rank is None:
                movement = "new"
            elif new_rank is None:
                movement = "removed"
            else:
                movement = f"{old_rank - new_rank:+d}"
            old_status = "—"
            if canonical_id in before_by_id:
                old_status = (
                    "provisional"
                    if is_provisional(before_by_id[canonical_id], role)
                    else "ranked"
                )
            new_status = "—"
            if canonical_id in after_by_id:
                new_status = (
                    "provisional"
                    if is_provisional(after_by_id[canonical_id], role)
                    else "ranked"
                )
            rank_before = "—" if old_rank is None else str(old_rank)
            rank_after = "—" if new_rank is None else str(new_rank)
            score_before = "—" if old_score is None else f"{old_score:.2f}"
            score_after = "—" if new_score is None else f"{new_score:.2f}"
            score_delta = (
                "—"
                if old_score is None or new_score is None
                else f"{new_score - old_score:+.2f}"
            )
            lines.append(
                f"| `{canonical_id}` | {rank_before} | {rank_after} | {movement} | "
                f"{score_before} | {score_after} | {score_delta} | "
                f"{old_status} → {new_status} |"
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
