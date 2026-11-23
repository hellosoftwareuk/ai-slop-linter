"""Fine-tune a code encoder as a pairwise code-quality ranker.

The model maps a code window to one scalar, its quality. Training uses a
Bradley-Terry objective on refactor commits: the window after a refactor
should score higher than the window before it,

    loss = -log sigmoid(q(after) - q(before)).

The score is therefore relative. Higher means "looks more like code people
refactor towards"; lower means "looks more like code people refactor away
from".

Only the top layers are fine-tuned, so training fits on a CPU: the frozen
layers' output is computed once per sequence and cached, and each epoch only
runs the trainable layers.
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
        self.frozen_layers = len(layers) - trainable_layers
        for layer in layers[self.frozen_layers :]:
            for parameter in layer.parameters():
                parameter.requires_grad = True

    def forward(self, input_ids, attention_mask):
        hidden = self.encoder(input_ids=input_ids, attention_mask=attention_mask).last_hidden_state
        return self.pool(hidden, attention_mask)

    def pool(self, hidden, attention_mask):
        mask = attention_mask.unsqueeze(-1).to(hidden.dtype)
        pooled = (hidden * mask).sum(1) / mask.sum(1).clamp(min=1.0)
        return self.head(pooled).squeeze(-1)

    @torch.no_grad()
    def frozen_states(self, input_ids, attention_mask):
        """Output of the last frozen layer; constant during training."""
        output = self.encoder(input_ids=input_ids, attention_mask=attention_mask, output_hidden_states=True)
        return output.hidden_states[self.frozen_layers]

    def top(self, hidden, attention_mask):
        """Score from cached frozen states by running only the trainable layers."""
        mask, _ = self.encoder._create_attention_masks(attention_mask, None, hidden, None, None)
        for layer in self.encoder.encoder.layer[self.frozen_layers :]:
            hidden = layer(hidden, mask)
        return self.pool(hidden, attention_mask)


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


def frozen_cache(model, tokenizer, texts, max_length, batch_size=32) -> list[torch.Tensor]:
    """Frozen-layer states per text, unpadded and stored as float16."""
    model.eval()
    order = sorted(range(len(texts)), key=lambda i: len(texts[i]))
    cache: list[torch.Tensor | None] = [None] * len(texts)
    for start in range(0, len(order), batch_size):
        chunk = order[start : start + batch_size]
        batch = tokenizer([texts[i] for i in chunk], truncation=True, max_length=max_length,
                          padding=True, return_tensors="pt")
        states = model.frozen_states(batch["input_ids"], batch["attention_mask"])
        for row, i in enumerate(chunk):
            length = int(batch["attention_mask"][row].sum())
            cache[i] = states[row, :length].to(torch.float16).clone()
    return cache


def collate(states: list[torch.Tensor]) -> tuple[torch.Tensor, torch.Tensor]:
    width = max(state.shape[0] for state in states)
    hidden = torch.zeros(len(states), width, states[0].shape[1])
    mask = torch.zeros(len(states), width, dtype=torch.long)
    for row, state in enumerate(states):
        hidden[row, : state.shape[0]] = state.float()
        mask[row, : state.shape[0]] = 1
    return hidden, mask


def score_cached(model, states: list[torch.Tensor], batch_size=64) -> list[float]:
    model.eval()
    order = sorted(range(len(states)), key=lambda i: states[i].shape[0])
    scores = [0.0] * len(states)
    with torch.no_grad():
        for start in range(0, len(order), batch_size):
            chunk = order[start : start + batch_size]
            for i, value in zip(chunk, model.top(*collate([states[i] for i in chunk])).tolist()):
                scores[i] = value
    return scores


def evaluate(model, rows, before_cache, after_cache) -> dict:
    before = score_cached(model, before_cache)
    after = score_cached(model, after_cache)
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
    started = time.time()
    caches = {
        (name, side): frozen_cache(model, tokenizer, [row[side] for row in rows], args.max_length)
        for name, rows in (("train", train), ("val", val))
        for side in ("before", "after")
    }
    print(f"cached frozen states in {time.time() - started:.0f}s", flush=True)
    pairs = list(zip(caches["train", "before"], caches["train", "after"]))
    best = -1.0
    step = 0
    started = time.time()
    for epoch in range(args.epochs):
        random.shuffle(pairs)
        model.train()
        running = []
        for start in range(0, len(pairs), args.batch_pairs):
            batch_pairs = pairs[start : start + args.batch_pairs]
            states = [before for before, _ in batch_pairs] + [after for _, after in batch_pairs]
            scores = model.top(*collate(states))
            before, after = scores[: len(batch_pairs)], scores[len(batch_pairs) :]
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
        metrics = evaluate(model, val, caches["val", "before"], caches["val", "after"])
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
