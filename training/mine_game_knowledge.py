#!/usr/bin/env python3
"""Mine per-game world knowledge from lanthorn play transcripts.

Two products per game, both evidence-only (every claim carries the event ids
that support it):

1. **Map**: rooms and directed exits, reconstructed from movement commands
   and the room names that follow them. Rooms are keyed by their displayed
   name; name variants and aliasing are left visible rather than merged.
2. **Vocabulary**: verbs and verb+noun pairs that produced real (non-parser-
   rejection) responses, ranked by observation count.

Intended consumers: an in-game auto-map, a "verbs this game understands"
onboarding panel, and corpus QA (a game whose map does not connect or whose
vocabulary is empty is a bad run).

Stdlib-only; read-only against the corpus.
"""

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

DIRECTIONS = {
    "n": "north", "s": "south", "e": "east", "w": "west",
    "ne": "northeast", "nw": "northwest", "se": "southeast", "sw": "southwest",
    "u": "up", "d": "down",
    "north": "north", "south": "south", "east": "east", "west": "west",
    "northeast": "northeast", "northwest": "northwest",
    "southeast": "southeast", "southwest": "southwest",
    "up": "up", "down": "down", "in": "in", "out": "out",
    "inside": "in", "outside": "out",
}
MOVEMENT_RE = re.compile(
    r"^(?:go\s+|walk\s+|run\s+|head\s+|move\s+)?("
    + "|".join(sorted(DIRECTIONS, key=len, reverse=True))
    + r")$",
    re.IGNORECASE,
)

# Parser rejections and other responses that carry no world progress. A
# command whose response matches any of these is a vocabulary negative.
FAILURE_MARKERS = (
    "i don't know the word", "i don't understand", "i beg your pardon",
    "you can't see any such thing", "that's not something you can",
    "that's not a verb i recognize", "you can't go that way",
    "you can't go in that direction", "there is no way to go",
    "nothing happens", "what do you want to", "that's hardly portable",
    "you see nothing special", "i only understood you as far as",
    "that seems to be closed", "you can't, at least not now",
    "you are not holding that", "you don't have that",
)

ROOM_LINE_MAX = 70


def room_name(response: str):
    """Best-effort room name: the first short title-like line of a response.

    Covers the two dominant conventions seen in the corpus: an Inform-style
    bare title line ('Living Room') and Scott-Adams-style 'You are in/at ...'
    sentences. Returns None when no line qualifies.
    """
    for line in response.splitlines():
        line = line.strip()
        if not line or line.startswith(">"):
            continue
        m = re.match(r"^You are (?:now )?(in|at|on) (.+?)\.?\s*(?:\(.*)?$", line, re.IGNORECASE)
        if m and len(line) <= ROOM_LINE_MAX:
            return m.group(2).rstrip(".").strip()
        if (
            len(line) <= ROOM_LINE_MAX
            and not line.endswith((".", "?", "!"))
            and not line.startswith(("(", "[", "You", "I ", "What", "How"))
            and re.search(r"[A-Za-z]", line)
            and len(line.split()) <= 8
        ):
            return line
        break  # only the first content line can be a room title
    return None


def mine_run(events_path: Path):
    """Yield (event_id, command, response) for command events in order."""
    with events_path.open() as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if ev.get("event_type") != "command":
                continue
            command = (ev.get("command") or "").strip()
            response = (ev.get("response") or "").strip()
            if command and response:
                yield ev.get("event_id"), command, response


def mine_game(run_dirs):
    """Mine map + vocabulary across every run directory of one game."""
    edges = defaultdict(list)  # (from_room, direction, to_room) -> [event_ids]
    rooms_seen = Counter()
    verbs = Counter()
    verb_noun = Counter()
    failures = Counter()
    command_count = 0

    for run_dir in run_dirs:
        events_path = run_dir / "events.jsonl"
        if not events_path.exists():
            continue
        current_room = None
        for event_id, command, response in mine_run(events_path):
            command_count += 1
            norm = " ".join(command.lower().split())
            response_head = response[:400].lower()
            failed = any(marker in response_head for marker in FAILURE_MARKERS)

            move = MOVEMENT_RE.match(norm)
            new_room = room_name(response)
            if move and not failed:
                direction = DIRECTIONS[move.group(1).lower()]
                if new_room:
                    rooms_seen[new_room] += 1
                    if current_room and current_room != new_room:
                        edges[(current_room, direction, new_room)].append(event_id)
                    current_room = new_room
            elif new_room and not failed and not move:
                # Non-movement command revealing a room title (look, restore).
                current_room = current_room or new_room

            words = norm.split()
            if words:
                if failed:
                    failures[words[0]] += 1
                else:
                    verbs[words[0]] += 1
                    if len(words) > 1:
                        verb_noun[" ".join(words[:2])] += 1

    return {
        "map": {
            "rooms": [
                {"name": name, "observations": count}
                for name, count in rooms_seen.most_common()
            ],
            "edges": [
                {
                    "from": src,
                    "direction": direction,
                    "to": dst,
                    "observations": len(event_ids),
                    "event_ids": event_ids[:3],
                }
                for (src, direction, dst), event_ids in sorted(
                    edges.items(), key=lambda kv: -len(kv[1])
                )
            ],
        },
        "vocabulary": {
            "verbs": [
                {"verb": verb, "successes": count, "failures": failures.get(verb, 0)}
                for verb, count in verbs.most_common()
            ],
            "verb_noun": [
                {"pair": pair, "successes": count}
                for pair, count in verb_noun.most_common(200)
            ],
        },
        "stats": {
            "command_events": command_count,
            "runs": sorted(d.name for d in run_dirs),
        },
    }


def base_game_id(run_id: str) -> str:
    for suffix in ("-kimi", "-cursor", "-junie", "-antigravity"):
        if run_id.endswith(suffix):
            return run_id[: -len(suffix)]
    return run_id


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--data-dir", required=True, type=Path,
                        help="Experiment workspace containing play/")
    parser.add_argument("--out", required=True, type=Path,
                        help="Output directory; one <game>.json per game plus _index.json")
    parser.add_argument("--game", help="Mine a single base game id (default: all)")
    args = parser.parse_args(argv)

    play_dir = args.data_dir / "play"
    runs_by_game = defaultdict(list)
    for run_dir in sorted(p for p in play_dir.iterdir() if p.is_dir()):
        if ".void-" in run_dir.name:
            continue  # voided duplicate runs; never evidence
        runs_by_game[base_game_id(run_dir.name)].append(run_dir)
    if args.game:
        if args.game not in runs_by_game:
            print(f"error: no runs for game {args.game!r}", file=sys.stderr)
            return 2
        runs_by_game = {args.game: runs_by_game[args.game]}

    args.out.mkdir(parents=True, exist_ok=True)
    index = {}
    for game_id, run_dirs in sorted(runs_by_game.items()):
        result = mine_game(run_dirs)
        result["game_id"] = game_id
        (args.out / f"{game_id}.json").write_text(json.dumps(result, indent=2, ensure_ascii=False))
        index[game_id] = {
            "rooms": len(result["map"]["rooms"]),
            "edges": len(result["map"]["edges"]),
            "verbs": len(result["vocabulary"]["verbs"]),
            "command_events": result["stats"]["command_events"],
        }
    (args.out / "_index.json").write_text(json.dumps(index, indent=2))
    print(f"mined {len(index)} games -> {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
