"""Use the quality ranker as a refactoring referee.

    python refactor_loop.py hotspots --model model.pt ../src --top 15
    python refactor_loop.py judge    --model model.pt --base HEAD

``hotspots`` scores every 50-line window of the TypeScript and Rust files
under a path and lists the worst, non-overlapping windows first.

``judge`` compares the working tree with a Git revision. Each cluster of
changed lines becomes an aligned before/after window (the same shape the
model was trained on) and is scored on both sides. A positive delta means
the model reads the new code as the kind people refactor towards. The
verdict is "keep" only when the net delta is positive and no single region
gets meaningfully worse; everything else is "revert". The judge does not
check behaviour: run the test suite and `slop diff` alongside it.
"""

from __future__ import annotations

import argparse
import difflib
import subprocess
from pathlib import Path

import numpy as np

from codetext import without_comments
from evaluate import load_model, windows
from train import score_texts

SOURCE_SUFFIXES = {".rs", ".ts", ".tsx"}
CONTEXT_LINES = 8
# Changes separated by more unchanged lines than this are judged separately.
CLUSTER_GAP = 2 * CONTEXT_LINES
# A region that drops by more than this is a regression even if others gain.
REGRESSION_TOLERANCE = 1.0


def hotspots(model, tokenizer, max_length, root: Path, top: int) -> None:
    candidates = []
    for path in sorted(root.rglob("*")):
        if path.suffix not in SOURCE_SUFFIXES or not path.is_file():
            continue
        # Windows are cut from the real file so line numbers stay accurate;
        # each is scored without comments, as the model was trained.
        chunks = windows(path.read_text())
        scores = score_texts(model, tokenizer, [without_comments(text) for _, text in chunks],
                             max_length)
        for score, (line, text) in zip(scores, chunks):
            candidates.append((score, str(path), line, line + len(text.splitlines()) - 1))
    candidates.sort()
    taken: list[tuple[float, str, int, int]] = []
    for candidate in candidates:
        _, path, start, end = candidate
        if any(path == other[1] and start <= other[3] and other[2] <= end for other in taken):
            continue
        taken.append(candidate)
        if len(taken) == top:
            break
    for score, path, start, end in taken:
        print(f"{score:+7.2f}  {path}:{start}-{end}")


def changed_clusters(old: list[str], new: list[str]):
    """Yield (old_window, new_window, new_start_line) per cluster of edits."""
    opcodes = [op for op in difflib.SequenceMatcher(a=old, b=new, autojunk=False).get_opcodes()
               if op[0] != "equal"]
    clusters: list[list[tuple]] = []
    for op in opcodes:
        if clusters and op[1] - clusters[-1][-1][2] <= CLUSTER_GAP:
            clusters[-1].append(op)
        else:
            clusters.append([op])
    for cluster in clusters:
        a0, a1 = cluster[0][1], cluster[-1][2]
        b0, b1 = cluster[0][3], cluster[-1][4]
        before = min(CONTEXT_LINES, a0, b0)
        after = min(CONTEXT_LINES, len(old) - a1, len(new) - b1)
        yield (
            "\n".join(old[a0 - before : a1 + after]),
            "\n".join(new[b0 - before : b1 + after]),
            b0 + 1,
        )


def git_lines(base: str, path: str) -> list[str] | None:
    result = subprocess.run(["git", "show", f"{base}:{path}"], capture_output=True, text=True)
    return result.stdout.splitlines() if result.returncode == 0 else None


def judge(model, tokenizer, max_length, base: str, head: str | None = None,
          strip_comments: bool = False) -> bool:
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True,
                         check=True).stdout.strip()
    revisions = [base] + ([head] if head else [])
    changed = subprocess.run(["git", "diff", "--name-only", *revisions, "--", "."],
                             capture_output=True, text=True, check=True, cwd=top).stdout.split()
    regions = []
    for relative in changed:
        path = Path(top, relative)
        if path.suffix not in SOURCE_SUFFIXES:
            continue
        old = git_lines(base, relative) or []
        if head:
            new = git_lines(head, relative) or []
        else:
            new = path.read_text().splitlines() if path.exists() else []
        if strip_comments:
            old = without_comments("\n".join(old)).splitlines()
            new = without_comments("\n".join(new)).splitlines()
        for old_window, new_window, line in changed_clusters(old, new):
            regions.append((relative, line, old_window, new_window))
    if not regions:
        print("no source changes to judge")
        return False

    # An empty side (a new or deleted file) has no meaningful score; it is
    # scored as a neutral zero so only the side that exists contributes.
    texts = [text for _, _, old, new in regions for text in (old, new) if text.strip()]
    scored = dict(zip(texts, score_texts(model, tokenizer, texts, max_length)))
    deltas = []
    for relative, line, old_window, new_window in regions:
        before = scored.get(old_window, 0.0) if old_window.strip() else 0.0
        after = scored.get(new_window, 0.0) if new_window.strip() else 0.0
        deltas.append(after - before)
        print(f"{after - before:+7.2f}  ({before:+.2f} -> {after:+.2f})  {relative}:{line}")
    net = float(np.sum(deltas))
    worst = float(np.min(deltas))
    keep = net > 0 and worst > -REGRESSION_TOLERANCE
    print(f"net {net:+.2f}, worst region {worst:+.2f} -> {'keep' if keep else 'revert'}")
    return keep


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["hotspots", "judge"])
    parser.add_argument("path", nargs="?", default=".")
    parser.add_argument("--model", required=True)
    parser.add_argument("--top", type=int, default=15)
    parser.add_argument("--base", default="HEAD")
    parser.add_argument("--head", help="judge this revision instead of the working tree")
    parser.add_argument("--strip-comments", action="store_true",
                        help="ignore whole-line comments (the model over-rewards them)")
    args = parser.parse_args()

    model, tokenizer, max_length = load_model(args.model)
    if args.command == "hotspots":
        hotspots(model, tokenizer, max_length, Path(args.path), args.top)
    else:
        raise SystemExit(0 if judge(model, tokenizer, max_length, args.base, args.head, args.strip_comments) else 1)


if __name__ == "__main__":
    main()
