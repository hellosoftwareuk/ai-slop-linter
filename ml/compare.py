"""Paired comparison of rankers on the same held-out refactor pairs.

    python compare.py --data data2 --models v2=model2.pt v1=model.pt

Independent confidence intervals overlap more than the methods actually
differ, because every method is judged on the same pairs. This bootstraps
the difference in pairwise accuracy pair by pair, which is the right test
for "is A better than B".
"""

from __future__ import annotations

import argparse
import json
import random

import numpy as np

from evaluate import fit_surface_baseline, load_model
from train import score_texts


def outcomes(before, after) -> np.ndarray:
    before, after = np.asarray(before), np.asarray(after)
    return np.where(after > before, 1.0, np.where(after == before, 0.5, 0.0))


def paired_difference(a: np.ndarray, b: np.ndarray, rounds: int = 5000):
    rng = random.Random(0)
    n = len(a)
    diffs = a - b
    samples = sorted(float(np.mean([diffs[rng.randrange(n)] for _ in range(n)])) for _ in range(rounds))
    return float(diffs.mean()), samples[int(0.025 * rounds)], samples[int(0.975 * rounds)]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", default="data2")
    parser.add_argument("--models", nargs="+", required=True, help="name=path pairs")
    args = parser.parse_args()

    train = [json.loads(line) for line in open(f"{args.data}/train.jsonl")]
    rows = [json.loads(line) for line in open(f"{args.data}/test.jsonl")]
    rows = [row for row in rows if row["kind"] == "refactor"]

    methods: dict[str, np.ndarray] = {}
    for spec in args.models:
        name, path = spec.split("=", 1)
        model, tokenizer, max_length = load_model(path)
        methods[name] = outcomes(
            score_texts(model, tokenizer, [r["before"] for r in rows], max_length),
            score_texts(model, tokenizer, [r["after"] for r in rows], max_length),
        )
    surface = fit_surface_baseline(train)
    methods["surface"] = outcomes([surface(r["before"]) for r in rows], [surface(r["after"]) for r in rows])
    methods["shorter"] = outcomes([-len(r["before"]) for r in rows], [-len(r["after"]) for r in rows])
    methods["slop"] = outcomes([-r["slop_before"] for r in rows], [-r["slop_after"] for r in rows])

    print(f"{len(rows)} held-out refactor pairs")
    for name, values in methods.items():
        print(f"  {name:<10} {values.mean():.3f}")
    first = args.models[0].split("=", 1)[0]
    for other in methods:
        if other == first:
            continue
        mean, low, high = paired_difference(methods[first], methods[other])
        print(f"  {first} - {other:<10} {mean * 100:+.1f} points  [{low * 100:+.1f}, {high * 100:+.1f}]")


if __name__ == "__main__":
    main()
