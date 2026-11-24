"""Evaluate the quality ranker against cheap baselines on held-out repositories.

Usage:
    python evaluate.py --data data                     # baselines only
    python evaluate.py --data data --model model.pt    # plus the transformer
    python evaluate.py --model model.pt --score FILE...  # score real files

Metric: pairwise accuracy, the fraction of held-out refactor commits where a
method rates the "after" window above the "before" window (ties count 0.5).
Control commits (fixes/features) are reported as the rate at which "after" is
preferred; a pure quality signal has no reason to prefer either side, so a
rate far from 0.5 suggests a method is detecting edit direction or size
rather than quality.
"""

from __future__ import annotations

import argparse
import json
import random
import re
from collections import defaultdict

import numpy as np

from codetext import without_comments

KEYWORDS = re.compile(r"\b(if|else|for|while|match|switch|case|catch|return|loop)\b|&&|\|\||\?")
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
COMMENT = re.compile(r"^\s*(//|/\*|\*|#)")


def surface_features(code: str) -> np.ndarray:
    lines = code.splitlines() or [""]
    nonblank = [line for line in lines if line.strip()] or [""]
    indents = [len(line) - len(line.lstrip()) for line in nonblank]
    lengths = [len(line) for line in nonblank]
    identifiers = IDENTIFIER.findall(code) or [""]
    depth = max_depth = 0
    for char in code:
        if char in "{([":
            depth += 1
            max_depth = max(max_depth, depth)
        elif char in "})]":
            depth = max(0, depth - 1)
    return np.array(
        [
            np.log1p(len(code)),
            np.log1p(len(nonblank)),
            np.mean(lengths),
            np.max(lengths),
            np.mean(indents),
            np.max(indents),
            max_depth,
            len(KEYWORDS.findall(code)) / len(nonblank),
            np.mean([len(name) for name in identifiers]),
            len(set(identifiers)) / len(identifiers),
            sum(1 for name in identifiers if len(name) <= 2) / len(identifiers),
            sum(1 for line in nonblank if COMMENT.match(line)) / len(nonblank),
            sum(1 for line in nonblank if len(line) > 100) / len(nonblank),
            (len(lines) - len(nonblank)) / len(lines),
        ]
    )


def accuracy(before, after) -> list[float]:
    return [1.0 if a > b else 0.5 if a == b else 0.0 for b, a in zip(before, after)]


def bootstrap(values: list[float], rounds: int = 2000) -> tuple[float, float, float]:
    rng = random.Random(0)
    n = len(values)
    means = sorted(sum(rng.choice(values) for _ in range(n)) / n for _ in range(rounds))
    return sum(values) / n, means[int(0.025 * rounds)], means[int(0.975 * rounds)]


def report(name: str, rows: list[dict], before, after) -> None:
    outcomes = accuracy(before, after)
    groups: dict[str, list[float]] = defaultdict(list)
    for row, outcome in zip(rows, outcomes):
        groups[f"{row['kind']}/all"].append(outcome)
        groups[f"{row['kind']}/{row['lang']}"].append(outcome)
    for key in sorted(groups):
        mean, low, high = bootstrap(groups[key])
        print(f"  {name:<22} {key:<22} {mean:.3f}  [{low:.3f}, {high:.3f}]  n={len(groups[key])}")


def fit_surface_baseline(train: list[dict]):
    from sklearn.linear_model import LogisticRegression
    from sklearn.preprocessing import StandardScaler

    diffs = np.array([surface_features(r["after"]) - surface_features(r["before"]) for r in train])
    scaler = StandardScaler(with_mean=False).fit(diffs)
    x = scaler.transform(diffs)
    # Both orientations, no intercept: the model must be antisymmetric.
    classifier = LogisticRegression(fit_intercept=False, max_iter=2000).fit(
        np.vstack([x, -x]), np.r_[np.ones(len(x)), np.zeros(len(x))]
    )
    return lambda code: float(classifier.decision_function(scaler.transform([surface_features(code)]))[0])


def report_combined(model, tokenizer, max_length, args, surface, rows, before, after) -> None:
    """Blend transformer and surface-feature scores, fitted on validation only.

    If the blend beats both parts on the test split, the transformer carries
    information the surface statistics do not.
    """
    from sklearn.linear_model import LogisticRegression

    from train import score_texts

    val = [json.loads(line) for line in open(f"{args.data}/val.jsonl")]
    val = [row for row in val if row["kind"] == "refactor"]
    val_before = score_texts(model, tokenizer, [r["before"] for r in val], max_length)
    val_after = score_texts(model, tokenizer, [r["after"] for r in val], max_length)

    def deltas(pairs, model_before, model_after):
        return np.array([
            [a - b, surface(row["after"]) - surface(row["before"])]
            for row, b, a in zip(pairs, model_before, model_after)
        ])

    x = deltas(val, val_before, val_after)
    blend = LogisticRegression(fit_intercept=False).fit(
        np.vstack([x, -x]), np.r_[np.ones(len(x)), np.zeros(len(x))]
    )
    weights = blend.coef_[0]
    print(f"  blend weights (fitted on val): transformer {weights[0]:+.3f}, surface {weights[1]:+.3f}")
    combined = deltas(rows, before, after) @ weights
    report("transformer + surface", rows, [0.0] * len(rows), list(combined))


def load_model(path: str):
    import torch
    from transformers import AutoTokenizer

    from train import QualityModel

    checkpoint = torch.load(path, map_location="cpu", weights_only=False)
    args = checkpoint["args"]
    model = QualityModel(args["encoder"], args["trainable_layers"])
    model.load_state_dict(checkpoint["state"])
    model.eval()
    return model, AutoTokenizer.from_pretrained(args["encoder"]), args["max_length"]


def windows(code: str, size: int = 50, stride: int = 25) -> list[tuple[int, str]]:
    lines = code.splitlines()
    starts = range(0, max(1, len(lines) - size // 2), stride)
    return [(start + 1, "\n".join(lines[start : start + size])) for start in starts]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", default="data")
    parser.add_argument("--model")
    parser.add_argument("--split", default="test")
    parser.add_argument("--score", nargs="*", help="score whole files with sliding windows")
    args = parser.parse_args()

    if args.score:
        from train import score_texts

        model, tokenizer, max_length = load_model(args.model)
        for path in args.score:
            chunks = windows(open(path).read())
            scores = score_texts(model, tokenizer, [without_comments(text) for _, text in chunks],
                                 max_length)
            worst = min(zip(scores, chunks))
            print(f"{path}: mean {np.mean(scores):+.3f}  worst {worst[0]:+.3f} at line {worst[1][0]}")
        return

    train = [json.loads(line) for line in open(f"{args.data}/train.jsonl")]
    rows = [json.loads(line) for line in open(f"{args.data}/{args.split}.jsonl")]
    print(f"{args.split}: pairwise accuracy (after rated better), 95% bootstrap CI")

    report("slop (file debt)", rows, [-r["slop_before"] for r in rows], [-r["slop_after"] for r in rows])
    report("shorter wins", rows, [-len(r["before"]) for r in rows], [-len(r["after"]) for r in rows])
    surface = fit_surface_baseline(train)
    report("surface features", rows, [surface(r["before"]) for r in rows], [surface(r["after"]) for r in rows])

    if args.model:
        from train import score_texts

        model, tokenizer, max_length = load_model(args.model)
        before = score_texts(model, tokenizer, [r["before"] for r in rows], max_length)
        after = score_texts(model, tokenizer, [r["after"] for r in rows], max_length)
        report("transformer", rows, before, after)
        report_combined(model, tokenizer, max_length, args, surface, rows, before, after)
        decided = [i for i, r in enumerate(rows) if r["kind"] == "refactor" and r["slop_before"] != r["slop_after"]]
        agree = [(after[i] > before[i]) == (rows[i]["slop_after"] < rows[i]["slop_before"]) for i in decided]
        if agree:
            print(f"  transformer agrees with slop on {np.mean(agree):.3f} of {len(agree)} refactors slop decides")


if __name__ == "__main__":
    main()
