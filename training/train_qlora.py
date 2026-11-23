#!/usr/bin/env python3
"""QLoRA fine-tune for the lanthorn player model.

Trains a 4-bit base model with a LoRA adapter on the chat-formatted dataset
produced by build_dataset.py (train.jsonl / valid.jsonl). Assistant-only loss
when the base model's chat template supports it.

Example (16 GB card):

    .venv/bin/python training/train_qlora.py \
      --data /work/shared-docs/lanthorn-sft/v1-20261011 \
      --base-model Qwen/Qwen3-8B \
      --out runs/qlora-qwen3-8b-v1 --epochs 1

Requires the training venv (torch, transformers, peft, trl, datasets,
bitsandbytes, accelerate) — see requirements-qlora.txt.
"""

import argparse
import json
import time
from pathlib import Path

import torch
from datasets import load_dataset
from peft import LoraConfig
from transformers import AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig
from trl import SFTConfig, SFTTrainer


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--data", required=True, type=Path)
    parser.add_argument("--base-model", default="Qwen/Qwen3-8B")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--epochs", type=float, default=1.0)
    parser.add_argument("--lr", type=float, default=2e-4)
    parser.add_argument("--lora-r", type=int, default=16)
    parser.add_argument("--max-seq-len", type=int, default=1536)
    parser.add_argument("--batch-size", type=int, default=1)
    parser.add_argument("--grad-accum", type=int, default=16)
    parser.add_argument("--max-examples", type=int, default=0,
                        help="Debug: cap training examples (0 = all)")
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    started = time.time()

    tokenizer = AutoTokenizer.from_pretrained(args.base_model)
    model = AutoModelForCausalLM.from_pretrained(
        args.base_model,
        quantization_config=BitsAndBytesConfig(
            load_in_4bit=True,
            bnb_4bit_quant_type="nf4",
            bnb_4bit_compute_dtype=torch.bfloat16,
        ),
        device_map={"": 0},  # whole model on GPU 0; "auto" splits to CPU when the desktop holds VRAM
    )

    dataset = load_dataset(
        "json",
        data_files={
            "train": str(args.data / "train.jsonl"),
            "valid": str(args.data / "valid.jsonl"),
        },
    )
    if args.max_examples:
        dataset["train"] = dataset["train"].select(range(args.max_examples))
    # Validation is a smoke signal, not a leaderboard: cap the slice so an
    # epoch-end eval pass costs minutes, not hours.
    eval_cap = 500
    if len(dataset["valid"]) > eval_cap:
        dataset["valid"] = dataset["valid"].select(range(eval_cap))

    peft_config = LoraConfig(
        r=args.lora_r,
        lora_alpha=args.lora_r * 2,
        lora_dropout=0.05,
        target_modules="all-linear",
        task_type="CAUSAL_LM",
    )
    sft_kwargs = dict(
        output_dir=str(args.out),
        num_train_epochs=args.epochs,
        learning_rate=args.lr,
        per_device_train_batch_size=args.batch_size,
        gradient_accumulation_steps=args.grad_accum,
        max_length=args.max_seq_len,
        logging_steps=25,
        save_strategy="epoch",
        eval_strategy="epoch",
        bf16=True,
        gradient_checkpointing=True,
        optim="paged_adamw_8bit",
        report_to=[],
        dataset_text_field=None,
        seed=20261011,
    )
    # Assistant-only loss needs a chat template with {% generation %} support;
    # enable when available, fall back to full-sequence loss otherwise.
    try:
        test = tokenizer.apply_chat_template(
            [{"role": "user", "content": "x"}, {"role": "assistant", "content": "y"}],
            tokenize=False,
        )
        if "{% generation %}" in tokenizer.chat_template:
            sft_kwargs["assistant_only_loss"] = True
    except Exception:
        pass

    trainer = SFTTrainer(
        model=model,
        args=SFTConfig(**sft_kwargs),
        train_dataset=dataset["train"],
        eval_dataset=dataset["valid"],
        processing_class=tokenizer,
        peft_config=peft_config,
    )
    trainer.train()
    trainer.save_model(args.out / "adapter")
    tokenizer.save_pretrained(args.out / "adapter")

    summary = {
        "base_model": args.base_model,
        "examples": len(dataset["train"]),
        "epochs": args.epochs,
        "lora_r": args.lora_r,
        "lr": args.lr,
        "max_seq_len": args.max_seq_len,
        "assistant_only_loss": sft_kwargs.get("assistant_only_loss", False),
        "wall_minutes": round((time.time() - started) / 60, 1),
        "train_loss": trainer.state.log_history[-1].get("loss") if trainer.state.log_history else None,
    }
    (args.out / "train-summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
