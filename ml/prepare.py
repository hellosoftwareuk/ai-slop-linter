"""Deduplicate mined pairs, split them by repository, and attach slop scores.

Usage: python prepare.py PAIRS_DIR OUT_DIR --slop path/to/slop

Splits are assigned by a stable hash of the repository name, so no repository
contributes to more than one split. Control pairs (ordinary fix/feature
commits) are kept only for repositories in the validation and test splits;
they are an evaluation probe, never training data.

Windows are stored with whole-line comments removed (see codetext.py), and
pairs whose only change was to comments or whitespace are dropped.

Each pair gets the slop debt points of its whole before/after file, which is
the baseline the transformer has to beat.
"""

from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import subprocess
import tempfile
from collections import Counter, defaultdict
from pathlib import Path

from codetext import without_comments

SPLITS = (("train", 0.8), ("val", 0.1), ("test", 0.1))
# A few prolific repositories would otherwise dominate the training signal.
MAX_PAIRS_PER_REPOSITORY = 20


def split_for(repo: str) -> str:
    bucket = int(hashlib.sha1(repo.encode()).hexdigest()[:8], 16) / 0xFFFFFFFF
    cumulative = 0.0
    for name, share in SPLITS:
        cumulative += share
        if bucket <= cumulative:
            return name
    return SPLITS[-1][0]


def extension(row: dict) -> str:
    suffix = Path(row["path"]).suffix.lower()
    if row["lang"] == "javascript":
        # slop has no JavaScript frontend; TSX parsing accepts plain JS and JSX.
        return ".tsx"
    return suffix if suffix in {".ts", ".tsx", ".rs"} else {"typescript": ".ts", "rust": ".rs"}[row["lang"]]


# Repository-level rules compare files with each other. Pairs are unrelated
# files sharing a scratch tree, so those findings would be artefacts (a
# before/after pair is, for instance, a perfect structural clone).
REPOSITORY_CATEGORIES = {"architecture", "structure"}
REPOSITORY_RULES = {"structural-clone"}


def slop_points(rows: list[dict], slop: str) -> None:
    """Annotate rows in place with file-local slop debt for both sides."""
    for side in ("before", "after"):
        # Each side is scanned in its own tree so a pair never meets itself.
        with tempfile.TemporaryDirectory() as tmp:
            for index, row in enumerate(rows):
                # Directories are named "p<index>" so slop's default skips
                # (fixtures, build output...) never hide them.
                target = Path(tmp, f"p{index}", f"file{extension(row)}")
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(row[f"{side}_file"])
            output = subprocess.run(
                [slop, tmp, "--format", "json", "--top", "1000000"],
                check=False,
                capture_output=True,
                text=True,
            )
            report = json.loads(output.stdout)
        points: dict[str, float] = defaultdict(float)
        for finding in report["findings"]:
            if finding["category"] in REPOSITORY_CATEGORIES or finding["rule"] in REPOSITORY_RULES:
                continue
            points[Path(finding["path"]).parts[-2]] += finding["points"]
        for index, row in enumerate(rows):
            row[f"slop_{side}"] = points.get(f"p{index}", 0.0)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("pairs_dir")
    parser.add_argument("out_dir")
    parser.add_argument("--slop", required=True)
    args = parser.parse_args()

    seen: set[tuple[str, str]] = set()
    per_repository: Counter = Counter()
    by_split: dict[str, list[dict]] = defaultdict(list)
    for path in sorted(glob.glob(os.path.join(args.pairs_dir, "*.jsonl"))):
        for line in open(path):
            row = json.loads(line)
            # Windows are judged without comments; a change that only
            # touched comments carries no structural signal.
            row["before"] = without_comments(row["before"])
            row["after"] = without_comments(row["after"])
            if row["before"].split() == row["after"].split():
                continue
            key = (row["before"], row["after"])
            if key in seen:
                continue
            seen.add(key)
            per_repository[row["kind"], row["repo"]] += 1
            if per_repository[row["kind"], row["repo"]] > MAX_PAIRS_PER_REPOSITORY:
                continue
            split = split_for(row["repo"])
            if row["kind"] == "control" and split == "train":
                continue
            by_split[split].append(row)

    os.makedirs(args.out_dir, exist_ok=True)
    for split, rows in by_split.items():
        # Chunk the slop runs so one temporary tree stays small.
        for start in range(0, len(rows), 2000):
            slop_points(rows[start : start + 2000], args.slop)
        with open(os.path.join(args.out_dir, f"{split}.jsonl"), "w") as handle:
            for row in rows:
                row.pop("before_file")
                row.pop("after_file")
                handle.write(json.dumps(row) + "\n")
        print(split, Counter((row["lang"], row["kind"]) for row in rows))


if __name__ == "__main__":
    main()
