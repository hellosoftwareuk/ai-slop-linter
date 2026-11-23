"""Fine-tune a code encoder as a pairwise code-quality ranker.

The model maps a code window to one scalar, its quality. Training uses a
Bradley-Terry objective on refactor commits: the window after a refactor
should score higher than the window before it,

    loss = -log sigmoid(q(after) - q(before)).

The score is therefore relative. Higher means "looks more like code people
refactor towards"; lower means "looks more like code people refactor away
from". Only the top layers are fine-tuned so training fits on a CPU.
"""

from __future__ import annotations

import argparse
import json
import math
import random
import time
from collections import defaultdict

import torch
from torch import nn
from transformers import AutoModel, AutoTokenizer, get_linear_schedule_with_warmup


class QualityModel(nn.Module):
    def __init__(self, encoder_name: str, trainable_layers: int):
        super().__init__()
        self.encoder = AutoModel.from_pretrained(encoder_name)
        self.head = nn.Linear(self.encoder.config.hidden_size, 1)
        for parameter in self.encoder.parameters():
            parameter.requires_grad = False
        layers = self.encoder.encoder.layer
        for layer in layers[len(layers) - trainable_layers :]:
            for parameter in layer.parameters():
                parameter.requires_grad = True

    def forward(self, input_ids, attention_mask):
        hidden = self.encoder(input_ids=input_ids, attention_mask=attention_mask).last_hidden_state
        mask = attention_mask.unsqueeze(-1).to(hidden.dtype)
        pooled = (hidden * mask).sum(1) / mask.sum(1).clamp(min=1.0)
        return self.head(pooled).squeeze(-1)


def load(path: str, kind: str | None = None) -> list[dict]:
    rows = [json.loads(line) for line in open(path)]
    return [row for row in rows if kind is None or row["kind"] == kind]


def score_texts(model, tokenizer, texts, max_length, batch_size=32) -> list[float]:
    model.eval()
    scores: list[float] = []
    # Length-sorted batches minimise padding; results are restored in order.
    order = sorted(range(len(texts)), key=lambda i: len(texts[i]))
    by_index: dict[int, float] = {}
    with torch.no_grad():
        for start in range(0, len(order), batch_size):
            chunk = order[start : start + batch_size]
            batch = tokenizer(
                [texts[i] for i in chunk],
                truncation=True,
                max_length=max_length,
                padding=True,
                return_tensors="pt",
            )
            for i, value in zip(chunk, model(batch["input_ids"], batch["attention_mask"]).tolist()):
                by_index[i] = value
    scores = [by_index[i] for i in range(len(texts))]
    return scores


def evaluate(model, tokenizer, rows, max_length) -> dict:
    before = score_texts(model, tokenizer, [row["before"] for row in rows], max_length)
    after = score_texts(model, tokenizer, [row["after"] for row in rows], max_length)
    groups: dict[str, list[float]] = defaultdict(list)
    for row, b, a in zip(rows, before, after):
        groups[f"{row['kind']}/{row['lang']}"].append(1.0 if a > b else 0.0)
        groups[f"{row['kind']}/all"].append(1.0 if a > b else 0.0)
    return {key: (round(sum(v) / len(v), 4), len(v)) for key, v in sorted(groups.items())}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", default="data")
    parser.add_argument("--encoder", default="microsoft/unixcoder-base")
    parser.add_argument("--trainable-layers", type=int, default=4)
    parser.add_argument("--max-length", type=int, default=384)
    parser.add_argument("--batch-pairs", type=int, default=8)
    parser.add_argument("--epochs", type=int, default=2)
    parser.add_argument("--lr", type=float, default=3e-5)
    parser.add_argument("--head-lr", type=float, default=1e-3)
    parser.add_argument("--limit", type=int, default=0, help="debug: cap training pairs")
    parser.add_argument("--out", default="model.pt")
    parser.add_argument("--seed", type=int, default=13)
    args = parser.parse_args()

    random.seed(args.seed)
    torch.manual_seed(args.seed)
    torch.set_num_threads(torch.get_num_threads())

    tokenizer = AutoTokenizer.from_pretrained(args.encoder)
    model = QualityModel(args.encoder, args.trainable_layers)
    train = load(f"{args.data}/train.jsonl", "refactor")
    val = load(f"{args.data}/val.jsonl")
    if args.limit:
        train = train[: args.limit]

    head_params = list(model.head.parameters())
    encoder_params = [p for p in model.encoder.parameters() if p.requires_grad]
    optimizer = torch.optim.AdamW(
        [{"params": encoder_params, "lr": args.lr}, {"params": head_params, "lr": args.head_lr}],
        weight_decay=0.01,
    )
    steps = args.epochs * math.ceil(len(train) / args.batch_pairs)
    scheduler = get_linear_schedule_with_warmup(optimizer, int(0.06 * steps), steps)

    print(f"train pairs={len(train)} steps={steps}", flush=True)
    best = -1.0
    step = 0
    started = time.time()
    for epoch in range(args.epochs):
        random.shuffle(train)
        model.train()
        running = []
        for start in range(0, len(train), args.batch_pairs):
            batch_rows = train[start : start + args.batch_pairs]
            texts = [row["before"] for row in batch_rows] + [row["after"] for row in batch_rows]
            batch = tokenizer(texts, truncation=True, max_length=args.max_length, padding=True, return_tensors="pt")
            scores = model(batch["input_ids"], batch["attention_mask"])
            before, after = scores[: len(batch_rows)], scores[len(batch_rows) :]
            loss = -nn.functional.logsigmoid(after - before).mean()
            optimizer.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()
            scheduler.step()
            step += 1
            running.append(((after > before).float().mean().item(), loss.item()))
            if step % 25 == 0:
                acc = sum(r[0] for r in running) / len(running)
                avg = sum(r[1] for r in running) / len(running)
                print(f"epoch {epoch} step {step}/{steps} loss {avg:.4f} train-acc {acc:.3f} "
                      f"{(time.time() - started) / step:.2f}s/step", flush=True)
                running = []
        metrics = evaluate(model, tokenizer, val, args.max_length)
        print(f"epoch {epoch} val {json.dumps(metrics)}", flush=True)
        if metrics["refactor/all"][0] > best:
            best = metrics["refactor/all"][0]
            torch.save(
                {"state": model.state_dict(), "args": vars(args), "val": metrics},
                args.out,
            )
            print(f"saved {args.out} (val refactor acc {best})", flush=True)


if __name__ == "__main__":
    main()
