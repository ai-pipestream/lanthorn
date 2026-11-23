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


class LocalModel:
    """Direct in-process generation (transformers + optional LoRA adapter).

    Loaded lazily so the server paths stay stdlib-only. Use this for the
    post-training comparison: the adapter directory from train_qlora.py drops
    straight in, no GGUF conversion needed.
    """

    def __init__(self, base: str, adapter: str | None):
        import torch  # noqa: F401
        from transformers import AutoModelForCausalLM, AutoTokenizer

        self.tokenizer = AutoTokenizer.from_pretrained(adapter or base)
        self.model = AutoModelForCausalLM.from_pretrained(
            base, torch_dtype=torch.bfloat16, device_map={"": 0}
        )
        if adapter:
            from peft import PeftModel

            self.model = PeftModel.from_pretrained(self.model, adapter)
        self.model.eval()

    def chat(self, transcript: str, max_new: int = 32) -> str:
        import torch

        messages = [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": transcript},
        ]
        # Qwen3-style templates deliberate by default; command picking
        # doesn't want it (mirrors think:false on the ollama path).
        enc = self.tokenizer.apply_chat_template(
            messages, add_generation_prompt=True, return_tensors="pt",
            return_dict=True, enable_thinking=False,
        ).to(self.model.device)
        with torch.no_grad():
            out = self.model.generate(
                **enc,
                max_new_tokens=max_new,
                do_sample=False,
                pad_token_id=self.tokenizer.eos_token_id,
            )
        return self.tokenizer.decode(out[0][enc["input_ids"].shape[1]:], skip_special_tokens=True)


def ask_model(endpoint: str, model: str, transcript: str, timeout: float, api: str) -> str:
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": transcript},
    ]
    if api == "ollama":
        # Native API: think:false is honored here (the OpenAI-compat endpoint
        # ignores it, and thinking models burn the whole budget on hidden
        # reasoning and return empty content).
        url = f"{endpoint.rstrip('/')}/api/chat"
        payload = json.dumps(
            {
                "model": model,
                "messages": messages,
                "stream": False,
                "think": False,
                "options": {"temperature": 0.2, "num_predict": 32},
            }
        ).encode()
        out_field = ("message", "content")
    else:
        url = f"{endpoint.rstrip('/')}/chat/completions"
        payload = json.dumps(
            {
                "model": model,
                "messages": messages,
                "temperature": 0.2,
                "max_tokens": 32,
            }
        ).encode()
        out_field = None
    req = urllib.request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = json.loads(resp.read())
    if api == "ollama":
        return body["message"]["content"]
    return body["choices"][0]["message"]["content"]


THINK_BLOCK = re.compile(r"<think>.*?</think>", re.DOTALL)


def sanitize_command(text: str) -> str:
    """Reduce a model reply to one plausible game command (broker rules)."""
    text = THINK_BLOCK.sub("", text)  # safety net if a template thinks anyway
    if text.strip().startswith("<think>"):
        return ""  # truncated think block; no usable command
    line = text.strip().splitlines()[0].strip() if text.strip() else ""
    line = line.strip('"\'')
    line = re.sub(r"^[>\s]+", "", line)
    return line if COMMAND_GUARD.match(line) else ""


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--engine-bin", required=True, type=Path)
    parser.add_argument("--story", required=True, type=Path)
    parser.add_argument("--endpoint", default="http://localhost:11434",
                        help="Server base URL (no /v1 suffix needed for --api ollama)")
    parser.add_argument("--model", required=True)
    parser.add_argument("--api", choices=["openai", "ollama", "local"], default="ollama",
                        help="ollama uses the native /api/chat with think disabled (default); "
                             "openai for vllm/llama.cpp servers; local loads a HF model in-process")
    parser.add_argument("--local-base", help="HF base model for --api local (e.g. Qwen/Qwen3-8B)")
    parser.add_argument("--local-adapter", help="Optional LoRA adapter directory for --api local")
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

    local = None
    if args.api == "local":
        if not args.local_base:
            parser.error("--api local requires --local-base")
        local = LocalModel(args.local_base, args.local_adapter)

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
                    reply = None
                else:
                    if args.api == "local":
                        reply = local.chat(context)
                    else:
                        reply = ask_model(args.endpoint, args.model, context, args.model_timeout, args.api)
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
                        "model_reply": reply,
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
