"""Text normalisation shared by data preparation, training, and judging."""

from __future__ import annotations


def without_comments(code: str) -> str:
    """Drop whole-line `//` comments and line-leading `/* ... */` blocks.

    Refactor commits often add comments, so a model trained on raw code
    learns to reward even useless ones. Scoring comment-free code removes
    that bias. Only lines that are entirely comment are dropped: trailing
    comments stay, because removing them safely needs a tokenizer that
    understands string literals. `#` is not treated as a comment marker
    (it starts Rust attributes), and neither is a bare `*` (it starts
    dereferences and rustfmt's wrapped multiplications).
    """
    kept = []
    in_block = False
    for line in code.splitlines():
        stripped = line.lstrip()
        if in_block:
            in_block = "*/" not in stripped
            continue
        if stripped.startswith("//"):
            continue
        if stripped.startswith("/*"):
            in_block = "*/" not in stripped[2:]
            continue
        kept.append(line)
    return "\n".join(kept)
