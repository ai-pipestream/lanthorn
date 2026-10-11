#!/usr/bin/env python3
"""Build supervised fine-tuning (SFT) datasets from lanthorn play transcripts.

Reads the full-play corpus produced by the lanthorn data-collection fleet
(play/<run>/events.jsonl plus review metadata) and writes chat-formatted JSONL
splits suitable for training a model to suggest the next player command.

Design rules inherited from the collection experiment:

- Only independently kept runs train the model by default (K3 keep verdicts).
- Held-out games come from the experiment's split registry
  (batches/splits.json); test games never enter the training or validation
  files, and families that share lineage stay together.
- Consecutive duplicate commands (stall loops) are dropped by default; they
  teach the model to loop.
- Output is deterministic for fixed inputs and seed.

The script is stdlib-only so it runs anywhere the corpus is mounted.
"""

import argparse
import json
import random
import sys
from collections import Counter
from pathlib import Path

from prompting import SYSTEM_PROMPT

LANE_SUFFIXES = ("-kimi", "-cursor", "-junie", "-antigravity")


def base_game_id(run_id: str) -> str:
    """Map a run directory name to the registry's base game id."""
    for suffix in LANE_SUFFIXES:
        if run_id.endswith(suffix):
            return run_id[: -len(suffix)]
    return run_id


def load_keep_verdicts(reviewed_dir: Path) -> set:
    """Run ids with an independent keep verdict."""
    path = reviewed_dir / "k3-judgments.jsonl"
    kept = set()
    if not path.exists():
        return kept
    with path.open() as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            rec = json.loads(line)
            if rec.get("verdict") == "keep":
                kept.add(rec["game_id"])
    return kept


def load_splits(path: Path) -> dict:
    registry = json.loads(path.read_text())
    return registry["assignments"]


def build_examples(run_dir: Path, window: int, dedup_consecutive: bool):
    """Build chat examples for one run directory.

    The transcript is replayed faithfully: every event's response text is
    appended to the rolling context, and each command appends a `> command`
    line. A command becomes a training target only when it passes the filters;
    its context is the transcript exactly as it stood *before* that command —
    the game's response to the target command never leaks into the input.
    Commands dropped as targets still appear in the context (a player saw
    those turns too).
    """
    events_path = run_dir / "events.jsonl"
    if not events_path.exists():
        return [], Counter({"no_events_file": 1})

    examples = []
    dropped = Counter()
    segments: list = []  # alternating narration / "> command" lines
    prev_norm = None
    with events_path.open() as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            response = (ev.get("response") or "").strip()
            if ev.get("event_type") != "command":
                if response:
                    segments.append(response)
                continue
            command = (ev.get("command") or "").strip()
            if not command:
                if response:
                    segments.append(response)
                continue
            norm = " ".join(command.lower().split())
            is_target = not (dedup_consecutive and norm == prev_norm)
            prev_norm = norm
            if is_target:
                context = "\n\n".join(segments[-2 * window :]).strip()
                if context:
                    examples.append(
                        {
                            "messages": [
                                {"role": "system", "content": SYSTEM_PROMPT},
                                {"role": "user", "content": context},
                                {"role": "assistant", "content": command},
                            ],
                            "meta": {
                                "event_id": ev.get("event_id"),
                                "session_id": ev.get("session_id"),
                                "branch_id": ev.get("branch_id"),
                            },
                        }
                    )
                else:
                    dropped["empty_context"] += 1
            else:
                dropped["consecutive_duplicate"] += 1
            segments.append(f"> {command}")
            if response:
                segments.append(response)
    return examples, dropped


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--data-dir",
        required=True,
        type=Path,
        help="Experiment workspace containing play/, reviewed/, batches/splits.json",
    )
    parser.add_argument("--out", required=True, type=Path, help="Output directory")
    parser.add_argument(
        "--window",
        type=int,
        default=8,
        help="Number of previous command/response pairs per example (default 8)",
    )
    parser.add_argument("--seed", type=int, default=20261011, help="Shuffle seed")
    parser.add_argument(
        "--include-unkept",
        action="store_true",
        help="Also train on runs without a keep verdict (default: kept runs only)",
    )
    parser.add_argument(
        "--keep-consecutive-duplicates",
        action="store_true",
        help="Keep stall-loop repeats (default: drop them)",
    )
    parser.add_argument(
        "--valid-fraction",
        type=float,
        default=0.05,
        help="Fraction of development-game examples held for validation (default 0.05)",
    )
    parser.add_argument(
        "--max-per-game",
        type=int,
        default=0,
        help="Cap train/valid examples per game (0 = no cap). Concentrates the "
             "dataset away from the longest runs; heldout is never capped.",
    )
    args = parser.parse_args(argv)

    play_dir = args.data_dir / "play"
    splits = load_splits(args.data_dir / "batches" / "splits.json")
    kept = load_keep_verdicts(args.data_dir / "reviewed")
    if not kept and not args.include_unkept:
        print("error: no keep verdicts found; pass --include-unkept to proceed anyway", file=sys.stderr)
        return 2

    outputs = {"train": [], "valid": [], "heldout": []}
    stats = {"runs": Counter(), "dropped": Counter(), "examples_by_split": Counter(),
             "examples_by_game": Counter(), "unregistered_runs": []}

    for run_dir in sorted(p for p in play_dir.iterdir() if p.is_dir()):
        run_id = run_dir.name
        game_id = base_game_id(run_id)
        if not args.include_unkept and run_id not in kept:
            stats["runs"]["skipped_not_kept"] += 1
            continue
        split = splits.get(game_id)
        if split is None:
            stats["runs"]["skipped_unregistered"] += 1
            stats["unregistered_runs"].append(run_id)
            continue
        examples, dropped = build_examples(
            run_dir,
            window=args.window,
            dedup_consecutive=not args.keep_consecutive_duplicates,
        )
        stats["dropped"].update(dropped)
        if not examples:
            stats["runs"]["no_examples"] += 1
            continue
        for ex in examples:
            ex["meta"].update({"game_id": game_id, "run_id": run_id, "split": split})
        target = {"train": "train", "development": "valid", "test": "heldout"}[split]
        if args.max_per_game and split != "test":
            existing = stats["examples_by_game"][game_id]
            budget = args.max_per_game - existing
            if budget <= 0:
                stats["runs"]["capped_skipped"] += 1
                stats["dropped"]["per_game_cap"] += len(examples)
                continue
            if len(examples) > budget:
                stats["dropped"]["per_game_cap"] += len(examples) - budget
                examples = examples[:budget]
        outputs[target].extend(examples)
        stats["runs"][f"used_{split}"] += 1
        stats["examples_by_game"][game_id] += len(examples)

    rng = random.Random(args.seed)
    for key in outputs:
        rng.shuffle(outputs[key])
    # Move a slice of development examples into validation is unnecessary —
    # development games already map to the valid file; --valid-fraction instead
    # carves a stratified slice off train for hyperparameter smoke tests when
    # the registry has no development games.
    if not outputs["valid"] and args.valid_fraction > 0 and outputs["train"]:
        cut = max(1, int(len(outputs["train"]) * args.valid_fraction))
        outputs["valid"] = outputs["train"][:cut]
        outputs["train"] = outputs["train"][cut:]
        stats["valid_carved_from_train"] = cut

    args.out.mkdir(parents=True, exist_ok=True)
    for name, rows in outputs.items():
        with (args.out / f"{name}.jsonl").open("w") as fh:
            for row in rows:
                fh.write(json.dumps(row, ensure_ascii=False) + "\n")
        stats["examples_by_split"][name] = len(rows)

    stats["unregistered_runs"].sort()
    with (args.out / "stats.json").open("w") as fh:
        json.dump(
            {
                "runs": dict(stats["runs"]),
                "dropped": dict(stats["dropped"]),
                "examples_by_split": dict(stats["examples_by_split"]),
                "examples_by_game": dict(
                    sorted(stats["examples_by_game"].items(), key=lambda kv: -kv[1])
                ),
                "unregistered_runs": stats["unregistered_runs"],
                "config": {
                    "window": args.window,
                    "seed": args.seed,
                    "include_unkept": args.include_unkept,
                    "dedup_consecutive": not args.keep_consecutive_duplicates,
                },
            },
            fh,
            indent=2,
        )

    print(
        "wrote train={} valid={} heldout={} to {}".format(
            len(outputs["train"]), len(outputs["valid"]), len(outputs["heldout"]), args.out
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
