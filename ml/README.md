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
python train.py --epochs 4 --out model.pt
python evaluate.py --data data --model model.pt
python evaluate.py --model model.pt --score ../tests/fixtures/sloppy.ts ../tests/fixtures/clean.ts
```

The run described below used CommitPackFT for TypeScript, Rust, and JavaScript
plus CommitPack shards `typescript-0001`…`0006` and `rust-0001`…`0004`.

## Results

RESULTS_PLACEHOLDER
