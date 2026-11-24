#!/usr/bin/env bash
# Accept or reject the working-tree changes against BASE (default HEAD).
#
# Behaviour and hygiene come from the same checks CI runs; the learned
# judge then decides whether the change actually reads better.
#
#   MODEL=path/to/model.pt ml/gate.sh [BASE]     (MODEL defaults to ml/model.pt)
set -euo pipefail

base=${1:-HEAD}
cd "$(git rev-parse --show-toplevel)"
model=$(realpath "${MODEL:-ml/model.pt}")

step() { printf '\n== %s\n' "$1"; }

step "rustfmt"
cargo fmt --all -- --check
step "tests"
cargo test --locked --quiet
step "clippy"
cargo clippy --locked --all-targets --quiet -- -D warnings
step "slop: no new debt, zero-slop gate"
cargo build --locked --release --quiet
./target/release/slop diff --base "$base"
./target/release/slop . --fail-above 0 >/dev/null
step "learned judge"
(cd ml && python refactor_loop.py judge --model "$model" --base "$base" --strip-comments) \
  2>&1 | grep -vE "Warning|Loading weights"
