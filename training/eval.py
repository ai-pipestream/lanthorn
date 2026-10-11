#!/usr/bin/env python3
"""Play a held-out game with a served model, for before/after fine-tune evals.

Drives a lanthorn dumb-terminal interpreter (`zvm-cli --plain`, and the gvm /
scott siblings) over stdin/stdout while an OpenAI-compatible server (ollama,
vllm, llama.cpp's server) supplies the player commands. Every turn is recorded
to events.jsonl and the raw engine bytes to stdout.raw, mirroring the
collection fleet's evidence layout so eval runs stay auditable.

Example:

    python3 training/eval.py \
      --engine-bin target.noindex/debug/zvm-cli \
      --story /path/to/photopia.z8 \
      --endpoint http://localhost:11434/v1 --model qwen3:8b \
      --max-turns 40 --out runs/photopia-baseline

Stdlib-only by design; the server does the heavy lifting.
"""

import argparse
import json
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from prompting import SYSTEM_PROMPT

MORE_PROMPT = re.compile(r"\[(?:more|More|MORE)\]\s*$")
COMMAND_GUARD = re.compile(r"^[^/\\]{1,200}$")
ENDING_HINTS = ("*** you have", "would you like to restart", "the end")


def read_available(proc, timeout: float) -> str:
    """Drain engine stdout until it goes quiet for `timeout` seconds."""
    import os
    import selectors

    sel = selectors.DefaultSelector()
    sel.register(proc.stdout, selectors.EVENT_READ)
    chunks = []
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        ready = sel.select(remaining)
        if not ready:
            break
        chunk = os.read(proc.stdout.fileno(), 65536).decode("utf-8", "replace")
        if not chunk:
            break  # engine closed stdout (exited)
        chunks.append(chunk)
        deadline = time.monotonic() + timeout  # reset on activity
    return "".join(chunks)


def ask_model(endpoint: str, model: str, transcript: str, timeout: float) -> str:
    payload = json.dumps(
        {
            "model": model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": transcript},
            ],
            "temperature": 0.2,
            "max_tokens": 32,
        }
    ).encode()
    req = urllib.request.Request(
        f"{endpoint.rstrip('/')}/chat/completions",
        data=payload,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = json.loads(resp.read())
    return body["choices"][0]["message"]["content"]


def sanitize_command(text: str) -> str:
    """Reduce a model reply to one plausible game command (broker rules)."""
    line = text.strip().splitlines()[0].strip() if text.strip() else ""
    line = line.strip('"\'')
    line = re.sub(r"^[>\s]+", "", line)
    return line if COMMAND_GUARD.match(line) else ""


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--engine-bin", required=True, type=Path)
    parser.add_argument("--story", required=True, type=Path)
    parser.add_argument("--endpoint", default="http://localhost:11434/v1")
    parser.add_argument("--model", required=True)
    parser.add_argument("--max-turns", type=int, default=40)
    parser.add_argument("--window", type=int, default=8,
                        help="Transcript pairs shown to the model (match training)")
    parser.add_argument("--read-timeout", type=float, default=2.0,
                        help="Seconds of engine silence that mean 'turn complete'")
    parser.add_argument("--model-timeout", type=float, default=120.0)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    engine_bin = args.engine_bin.resolve()
    story = args.story.resolve()

    args.out.mkdir(parents=True, exist_ok=True)
    raw_path = args.out / "stdout.raw"
    events_path = args.out / "events.jsonl"

    engine_args = [str(engine_bin), "--plain", "--data-dir", str(args.out / "saves")]
    if "zvm" in engine_bin.name:
        engine_args += ["--sound", "off"]
    engine_args.append(str(story))

    proc = subprocess.Popen(
        engine_args,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        cwd=args.out,
        bufsize=0,
    )

    segments: list = []
    events = []
    outcome = "max-turns"
    with raw_path.open("wb") as raw:
        try:
            opening = read_available(proc, args.read_timeout)
            raw.write(opening.encode())
            if opening.strip():
                segments.append(opening.strip())
            for turn in range(1, args.max_turns + 1):
                if proc.poll() is not None:
                    outcome = "engine-exited"
                    break
                context = "\n\n".join(segments[-2 * args.window :]).strip()
                if MORE_PROMPT.search(context):
                    command = " "
                else:
                    reply = ask_model(args.endpoint, args.model, context, args.model_timeout)
                    command = sanitize_command(reply) or "look"
                proc.stdin.write((command + "\n").encode())
                proc.stdin.flush()
                response = read_available(proc, args.read_timeout)
                raw.write(response.encode())
                events.append(
                    {
                        "event_index": turn,
                        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S+00:00", time.gmtime()),
                        "command": command,
                        "response": response,
                    }
                )
                segments.append(f"> {command}")
                if response.strip():
                    segments.append(response.strip())
                tail = response.lower()
                if any(hint in tail for hint in ENDING_HINTS):
                    outcome = "ending-detected"
                    break
                print(f"[{turn}] {command!r}", flush=True)
        except KeyboardInterrupt:
            outcome = "interrupted"
        finally:
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()

    with events_path.open("w") as fh:
        for ev in events:
            fh.write(json.dumps(ev, ensure_ascii=False) + "\n")
    summary = {
        "model": args.model,
        "endpoint": args.endpoint,
        "story": str(story),
        "turns": len(events),
        "outcome": outcome,
        "window": args.window,
    }
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
