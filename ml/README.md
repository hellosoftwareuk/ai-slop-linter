# Learned code-quality ranker (prototype)

An experiment: can a transformer learn what humans consider poorly structured
or hard-to-read code, beyond what `slop`'s explicit rules measure?

This directory is a self-contained Python prototype. Nothing here is wired
into the `slop` CLI yet; that decision depends on the results below.

## Idea

Nobody publishes a large dataset labelled "this code is badly designed". But
developers label code implicitly every time they commit a refactor: the
version before a commit titled *"Refactor X"*, *"Simplify Y"*, or *"Extract
helper for Z"* is one they judged worse than the version after it.

The model learns from those judgments. It maps a window of code to one
scalar, its *quality*, and is trained with a pairwise (Bradley-Terry)
objective so the after-refactor window scores higher than the
before-refactor window:

```text
loss = -log σ(q(after) − q(before))
```

So the score is relative, not an absolute grade. Low means "resembles code
people refactor away from"; high means "resembles code people refactor
towards".

## Pipeline

| Step | Script | What it does |
| --- | --- | --- |
| Mine | `mine_pairs.py` | Streams [CommitPack](https://huggingface.co/datasets/bigcode/commitpack) JSONL and keeps single-file commits whose messages describe a quality change. Cuts aligned before/after windows around the changed hunks (plus 6 lines of shared context) so both sides cover the same logical region. Also emits ordinary fix/feature commits as a *control* set. |
| Prepare | `prepare.py` | Deduplicates, splits 80/10/10 **by repository** (no repository appears in more than one split), and scores both full files with `slop`'s file-local rules as a baseline. |
| Train | `train.py` | Fine-tunes the top 4 layers of [`microsoft/unixcoder-base`](https://huggingface.co/microsoft/unixcoder-base) (125M parameters) plus a linear head. Frozen-layer activations are cached once, which makes CPU training about 3× faster. |
| Evaluate | `evaluate.py` | Pairwise accuracy on held-out repositories with bootstrap confidence intervals, compared against three baselines. Also scores arbitrary files with sliding windows. |

## Reproduce

```sh
python -m venv .venv && . .venv/bin/activate
pip install --index-url https://download.pytorch.org/whl/cpu torch
pip install -r ml/requirements.txt
cargo build --release

mkdir -p ml/pairs
base=https://huggingface.co/datasets/bigcode
for lang in typescript rust javascript; do
  curl -sL $base/commitpackft/resolve/main/data/$lang/data.jsonl \
    | python ml/mine_pairs.py --lang $lang --max-control 400 > ml/pairs/ft-$lang.jsonl
done
for shard in typescript-0001 typescript-0002 rust-0001 rust-0002; do   # more shards = more data
  lang=${shard%-*}
  curl -sL $base/commitpack/resolve/main/data/$lang/$shard.jsonl \
    | python ml/mine_pairs.py --lang $lang --max-control 600 > ml/pairs/full-$shard.jsonl
done

cd ml
python prepare.py pairs data --slop ../target/release/slop
python train.py --epochs 4 --out model.pt   # resumable; keeps the best validation epoch
python evaluate.py --data data --model model.pt
python evaluate.py --model model.pt --score ../tests/fixtures/sloppy.ts ../tests/fixtures/clean.ts
```

The run described below used CommitPackFT for TypeScript, Rust, and JavaScript
plus CommitPack shards `typescript-0001`…`0006` and `rust-0001`…`0004`.

## Results

Data: 4,116 unique refactor pairs from about 2,800 repositories (3,226 train,
463 validation, 427 test) plus control fix/feature pairs for the held-out
repositories. Refactors make the window shorter only 52.6% of the time, so
length alone is not a giveaway.

The model is the epoch-1 checkpoint (best validation accuracy, 61.8%). Training
accuracy reached 86% early in epoch 2 while validation stalled, so the
model is data-limited, not compute-limited.

### Held-out test repositories

Pairwise accuracy is the fraction of refactor commits where the method rates
"after" above "before" (95% bootstrap interval). The control column is how
often "after" is preferred on ordinary fix/feature commits; near 50% means the
method is not merely detecting "newer" or "longer" code.

| Method | Refactor, all | TypeScript | Rust | JavaScript | Control |
| --- | --- | --- | --- | --- | --- |
| `slop` file-local debt | 50.2% [47.4, 53.0] | 49.0% | 51.4% | 51.2% | 41.2% |
| Shorter wins | 50.5% [45.9, 54.9] | 52.9% | 50.5% | 46.8% | 19.5% |
| Surface features (14 stats, logistic) | 60.5% [56.3, 65.2] | 57.1% | 66.2% | 60.8% | 46.7% |
| **Transformer** | **62.5% [57.8, 67.2]** | **63.9%** | 57.7% | **64.8%** | 47.7% |
| Transformer + surface (blend fitted on val) | 62.8% [58.1, 67.4] | 62.3% | 59.5% | 66.4% | 47.1% |

Reading the table:

* **The signal is real.** Refactor direction is learnable from a single window
  of code, and the transformer gets it right about 5 times in 8 on
  repositories it never saw, while staying neutral on control commits.
* **It is only modestly better than surface statistics.** The 2-point gap
  over indentation, line-length, and identifier statistics is inside the
  confidence interval. It is clearly better on TypeScript (+6.8) and worse on
  Rust (−8.5, from only 111 test pairs).
* **It measures something different from `slop`.** On the 142 test refactors
  where `slop`'s debt changes, the transformer agrees with `slop`'s direction
  48.6% of the time, roughly independent. Most refactors are below `slop`'s
  thresholds (it scores both sides equally two-thirds of the time).
* **The ceiling is low by construction.** Commit messages are noisy labels:
  some "refactors" are style churn or mix in behaviour changes, so no method
  should approach 100% on this benchmark.

### Sanity check on this repository

`python evaluate.py --model model.pt --score FILE` slides a 50-line window
over each file (stride 25) and reports the mean and the worst window:

| File | Mean | Worst window |
| --- | --- | --- |
| `tests/fixtures/clean.ts` | +8.6 | +8.6 |
| `tests/fixtures/sloppy.ts` | −8.2 | −12.6 at line 51 (a 24-step pointless alias chain) |
| `tests/fixtures/clean.rs` | +3.0 | +3.0 |
| `tests/fixtures/sloppy.rs` | −11.3 | −13.6 at line 51 |
| `src/cli.rs` | +4.4 | +3.5 |
| `src/scoring.rs` | +3.2 | −0.3 |
| `src/analyzer/core.rs` | −0.5 | −2.6 at line 301 (repeated `if let Some(frame) = self.frames.last_mut()` blocks) |

Scores are relative (no calibrated zero point), but the ordering matches
human judgment on every fixture.

## Limits and next steps

* **More data is the main lever.** This run used 6 of 77 TypeScript and 4 of
  30 Rust CommitPack shards. Mining takes about a minute per shard; 5–10× more
  pairs is cheap and directly targets the overfitting seen in epoch 2.
* **Labels could be cleaner.** Filtering pairs whose diff is mostly
  formatting, or having an LLM verify a sample of commit messages, would raise
  the achievable ceiling.
* **Windows, not repositories.** The model sees ≤384 tokens, so it judges local
  readability and design. Cross-module architecture (cycles, coupling,
  layering) stays with `slop`'s repository graph rules.
* **Calibration before integration.** A `slop` finding would need percentiles
  over a reference corpus (e.g. "bottom 5% of windows") rather than raw scores,
  plus ONNX export and a Rust runtime behind an opt-in cargo feature so the
  default build stays fast.
