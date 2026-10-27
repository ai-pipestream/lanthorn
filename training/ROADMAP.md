# Training pipeline roadmap

## Where we are (2026-10-11)

- `build_dataset.py` → SFT splits (35.6k train / 7.4k valid / 4.6k heldout,
  per-game cap 800, lane + protocol metadata per example)
- `train_qlora.py` → QLoRA v1 running on the 4080S (Qwen3-8B, ~2.5 days at
  the measured pace)
- `eval.py` → photopia baseline recorded (qwen3:8b base: reaches the housing
  cluster by turn 14, then stalls; 53% look-ratio)
- `mine_game_knowledge.py` → maps + vocabularies for all 111 games
  (4,654 rooms / 8,652 edges), corpus QA flags included

## Muse returns (tonight)

Lifting `control/stop-broker` resumes broker-based collection on the Pi
fleet (decision belongs to the coordinator/user). The recovery machinery is
fixed since the last stand-down (600s adoption window, stop-broker gate,
budget re-entry). New runs land in the same corpus; rebuild the dataset with
build_dataset.py and retrain — the pipeline is idempotent.

Priority for new collection: the ~13 games without a validated solve
(anchorhead, curses, varicella, windhall, time-all-things are the marathons),
and more v2-protocol data (v1:v2 is currently 2.2:1).

## The 200+ GB machine (next month)

- **Inference**: serve the merged tuned model at full precision, or a 70B
  class model as the fleet player — eval.py already talks to any
  OpenAI-compatible server.
- **Training**: full fine-tune of 8-14B (no quantization tricks), or QLoRA on
  32B+. Same scripts; only --base-model and batch settings change.
- **Retrieval track** (the original lanthorn target): the candidate-generation
  pipeline (run-candidates.sh) has never run at scale. 200 GB of VRAM makes
  the labeling + embedding-training loop cheap. The transcripts are ready.

## Data quality ledger (fix-as-we-go)

- Lane suffix ≠ assignment cli in 43 runs — resolved via
  evaluation/lane-resolution.json (cli wins), joined by build_dataset.py.
- Protocol version via manifest date proxy — stamped as meta.protocol.
- `origin` field absent from events — probe/recovery exclusion is by
  event_type + dedup instead; revisit if probes prove detectable another way.
- underoos-that-ate-new-york: no rooms mined (investigate run quality).
- Status-bar-only room titles (shade) are invisible to transcript mining by
  construction; the mapper crate reads screen memory if we need those maps.
