"""Mine before/after code pairs from commit datasets.

Reads CommitPack-style JSONL on stdin (fields: old_contents, new_contents,
subject, message, old_file, new_file, repos) and writes one JSON object per
usable pair to stdout.

Each pair is a pair of aligned windows: the changed hunks plus a few lines of
shared context, cut from the same logical region of the old and new file. Two
kinds of pairs are emitted:

* ``refactor`` - commit messages that describe a behaviour-preserving quality
  change (refactor, simplify, clean up, readability...). These are the
  training signal: the "after" window is presumed to be better.
* ``control`` - ordinary fix/feature commits. They are only used for
  evaluation: a model that measures quality, rather than "this looks like the
  newer version", should be close to indifferent on them.
"""

from __future__ import annotations

import argparse
import difflib
import json
import re
import sys

REFACTOR = re.compile(
    r"\b(refactor\w*|clean(?:ed|s|ing)?[- ]?up|cleanup|simplif\w*|readab\w*|"
    r"tid(?:y|ied|ies)\s+(?:up\s+)?(?:code|logic|function|method)|restructur\w*|"
    r"reorgani[sz]\w*|decoupl\w*|dedup\w*|de-?duplicat\w*|DRY\b|"
    r"extract(?:ed|s)?\s+(?:a\s+|the\s+)?(?:method|function|helper|component|class|module|trait|fn)|"
    r"split\s+(?:up\s+)?(?:\w+\s+)?(?:function|method|component|class|module|file)|"
    r"improve\s+(?:code\s+)?(?:structure|readability|clarity)|code\s+quality|"
    r"more\s+idiomatic|idiomatic)",
    re.IGNORECASE,
)
# Commits that mention these alongside a refactor keyword usually mix a
# behaviour change into the diff, or undo a previous quality change.
MIXED = re.compile(r"\b(revert\w*|wip|merge|bump|version|release|format(?:ting)?|prettier|rustfmt|lint\w*|typo)\b", re.IGNORECASE)
CONTROL = re.compile(r"^\s*(fix|add|implement|support|handle|update|allow|use)\w*\b", re.IGNORECASE)

CONTEXT_LINES = 6
MIN_CHANGED = 3
MAX_CHANGED_SIDE = 120
MAX_WINDOW_LINES = 160
MAX_FILE_BYTES = 80_000


def classify(subject: str, message: str) -> str | None:
    text = f"{subject}\n{message}"
    if MIXED.search(subject):
        return None
    if REFACTOR.search(text):
        return "refactor"
    if CONTROL.search(subject) and not REFACTOR.search(text):
        return "control"
    return None


def aligned_windows(old: list[str], new: list[str]):
    """Return (old_window, new_window, changed_old, changed_new) or None.

    The window spans every non-equal opcode plus CONTEXT_LINES of shared
    context on each side, so both versions cover the same logical region.
    """
    matcher = difflib.SequenceMatcher(a=old, b=new, autojunk=False)
    changes = [op for op in matcher.get_opcodes() if op[0] != "equal"]
    if not changes:
        return None
    a0 = min(op[1] for op in changes)
    a1 = max(op[2] for op in changes)
    b0 = min(op[3] for op in changes)
    b1 = max(op[4] for op in changes)
    changed_old = sum(op[2] - op[1] for op in changes)
    changed_new = sum(op[4] - op[3] for op in changes)
    if changed_old + changed_new < MIN_CHANGED:
        return None
    if changed_old > MAX_CHANGED_SIDE or changed_new > MAX_CHANGED_SIDE:
        return None
    # Equal regions before a0/b0 and after a1/b1 line up one-to-one, so the
    # same amount of context can be taken from both files.
    before = min(CONTEXT_LINES, a0, b0)
    after = min(CONTEXT_LINES, len(old) - a1, len(new) - b1)
    old_window = old[a0 - before : a1 + after]
    new_window = new[b0 - before : b1 + after]
    if max(len(old_window), len(new_window)) > MAX_WINDOW_LINES:
        return None
    return old_window, new_window, changed_old, changed_new


def substantive(lines: list[str]) -> list[str]:
    return [line.strip() for line in lines if line.strip()]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lang", required=True)
    parser.add_argument("--max-control", type=int, default=2000)
    args = parser.parse_args()

    controls = 0
    for raw in sys.stdin:
        try:
            row = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if row.get("old_file") != row.get("new_file"):
            continue
        old, new = row.get("old_contents") or "", row.get("new_contents") or ""
        if not old or not new or len(old) > MAX_FILE_BYTES or len(new) > MAX_FILE_BYTES:
            continue
        kind = classify(row.get("subject", ""), row.get("message", ""))
        if kind is None or (kind == "control" and controls >= args.max_control):
            continue
        windows = aligned_windows(old.splitlines(), new.splitlines())
        if windows is None:
            continue
        old_window, new_window, changed_old, changed_new = windows
        # Whitespace-only or reordering-free edits carry no quality signal.
        if substantive(old_window) == substantive(new_window):
            continue
        if kind == "control":
            controls += 1
        json.dump(
            {
                "kind": kind,
                "lang": args.lang,
                "repo": (row.get("repos") or "").split(",")[0],
                "commit": row.get("commit"),
                "path": row.get("new_file"),
                "subject": row.get("subject", "")[:200],
                "before": "\n".join(old_window),
                "after": "\n".join(new_window),
                "changed_before": changed_old,
                "changed_after": changed_new,
                # Whole files let file-level tools (such as slop) score both
                # versions without parsing a window cut mid-construct.
                "before_file": old,
                "after_file": new,
            },
            sys.stdout,
        )
        sys.stdout.write("\n")


if __name__ == "__main__":
    main()
