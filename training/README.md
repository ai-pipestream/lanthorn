# training/

Fine-tuning pipeline for a lanthorn player model, fed by the full-play corpus
collected in the companion experiment workspace
(`/work/experiments/lanthorn-recall-training`).

## build_dataset.py

Builds chat-formatted SFT JSONL (train / valid / heldout) from the fleet's
`play/<run>/events.jsonl` transcripts.

```sh
python3 training/build_dataset.py \
  --data-dir /work/experiments/lanthorn-recall-training \
  --out /path/to/dataset
```

Behavior:

- **Kept runs only** by default — a run needs a `keep` verdict in
  `reviewed/k3-judgments.jsonl` (`--include-unkept` relaxes this).
- **Splits come from the registry** (`batches/splits.json`): test games only
  ever land in `heldout.jsonl`; development games become the validation file.
  Never train on `heldout.jsonl`.
- **Stall loops dropped**: a command identical to the one immediately before
  it is excluded as a target (it still appears in later contexts — the player
  saw those turns).
- **No answer leakage**: an example's context is the transcript exactly as it
  stood before the target command; the response to that command is never in
  the input.
- Deterministic for fixed inputs and `--seed`.

Each example is `{"messages": [system, user, assistant], "meta": {...}}` with
provenance (game_id, run_id, session/branch, event_id) in `meta` so examples
can be traced back to exact transcript spans. `stats.json` records per-split
counts, drop reasons, and the config used.

## eval.py

Plays a game with a served model and records every turn — the baseline and
post-training comparison harness.

```sh
python3 training/eval.py \
  --engine-bin target.noindex/debug/zvm-cli \
  --story /path/to/game.z5 \
  --endpoint http://localhost:11434/v1 --model qwen3:8b \
  --max-turns 40 --out runs/photopia-baseline
```

- Works against any OpenAI-compatible server (ollama, vllm, llama.cpp).
- Uses the same system prompt as the dataset builder (`prompting.py`) — a
  tuned model is always evaluated under its training instruction.
- Commands pass the same hygiene rules as the collection broker (one short
  line, no path characters); unparseable replies degrade to `look`.
- `[MORE]` paging is answered with a keypress before the model is asked again.
- Writes `events.jsonl`, `stdout.raw`, and `summary.json` (turns, outcome)
  into `--out`, mirroring the fleet's evidence layout.

Always evaluate on `heldout.jsonl` games only — never on training games.

## train_qlora.py

QLoRA fine-tune on the built dataset. Needs the training venv
(`requirements-qlora.txt`; ~6 GB of wheels, CUDA build of torch).

```sh
cd training
uv venv --python 3.12 .venv
uv pip install --python .venv/bin/python -r requirements-qlora.txt
PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True \
  .venv/bin/python train_qlora.py \
  --data /work/shared-docs/lanthorn-sft/v2-20261011-capped800 \
  --base-model Qwen/Qwen3-8B --out runs/qlora-qwen3-8b-v2 \
  --batch-size 2 --grad-accum 8 --max-seq-len 1280 --epochs 1
```

Notes from the first run on a 4080 SUPER (16 GB, shared with the desktop):

- `device_map="auto"` splits to CPU when the desktop holds VRAM — the script
  pins the whole model to GPU 0 instead.
- Validation runs on a 500-example slice so an epoch-end eval costs minutes.
- Measured pace is ~100 s per optimizer step at these settings: 35.6k examples
  / 16 per step ≈ 2.2k steps ≈ ~2.5 days. Trim `--data` (per-game cap in
  build_dataset.py) or `--max-seq-len` for a shorter run.

## Suggested first run (QLoRA on a 16 GB card)

- Base: Qwen3-8B or Llama-3.1-8B-Instruct, 4-bit, LoRA r=16–32
- ~50k train examples, 1–2 epochs, seq len 2048 (p95 context fits)
- Evaluate by playing a held-out game through `zvm-cli --plain` and comparing
  progress against the base model — never on training games.

See `docs/plans/` in the experiment workspace for the corpus statistics that
motivate these choices.
