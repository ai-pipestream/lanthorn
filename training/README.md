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

## Suggested first run (QLoRA on a 16 GB card)

- Base: Qwen3-8B or Llama-3.1-8B-Instruct, 4-bit, LoRA r=16–32
- ~50k train examples, 1–2 epochs, seq len 2048 (p95 context fits)
- Evaluate by playing a held-out game through `zvm-cli --plain` and comparing
  progress against the base model — never on training games.

See `docs/plans/` in the experiment workspace for the corpus statistics that
motivate these choices.
