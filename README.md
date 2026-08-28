# slop

[![Slop analysis](https://github.com/hellosoftwareuk/ai-slop-linter/actions/workflows/slop-analysis.yml/badge.svg)](https://github.com/hellosoftwareuk/ai-slop-linter/actions/workflows/slop-analysis.yml)

`slop` is a fast, native CLI that scans TypeScript, TSX, Rust, Terraform, and
Terragrunt syntax trees for maintainability hotspots. Point it at a repository
or any subfolder:

```sh
slop .
slop src/features/billing
slop . --format json
slop . --fail-above 35
slop . --top 50
slop src --fix
```

Scans are read-only unless `--fix` is present. The fixer currently supports
TypeScript and deliberately has no unsafe mode or configuration surface. It
applies twenty low-risk rewrites:

- `prefer-const` only when Oxc's semantic binding graph proves one initialized
  `let` binding has no writes
- `object-property-shorthand` for ordinary identifier pairs such as
  `{ customer: customer }`
- `redundant-boolean-conditional` for exact `condition ? true : false` and
  reversed literal branches, retaining one condition evaluation
- `duplicate-type-member` for exact repeated union or intersection members
- `collapsible-if` for strict, else-free `if` blocks containing only one nested
  braced `if`
- `prefer-dot-property` for static ASCII identifier access such as
  `customer["name"]`, including optional chains
- `prefer-static-object-key` for safe computed keys such as
  `{ ["customer"]: customer }`, excluding `__proto__`
- `empty-else` for structurally empty, comment-free else blocks
- `redundant-terminal-return` for an argument-free final function statement
- `redundant-terminal-continue` for an unlabeled final statement in a braced
  loop body
- `redundant-boolean-return` for exact true/false return branches, retaining one
  condition evaluation
- `unnecessary-empty-statement` for standalone semicolons that are direct
  statement-list items rather than control-flow bodies
- `redundant-type-identity` for `never` union members and `unknown`
  intersection members when another member remains
- `duplicate-type-assertion` for adjacent `as` or angle-bracket assertions
  whose type source is exactly equal
- `duplicate-non-null-assertion` for adjacent repeated `!` assertions
- `jsx-boolean-shorthand` for JSX attributes explicitly set to literal `true`
- `collapsible-else-if` for comment-free else blocks containing only one `if`
- `invert-empty-if` for direct statement-list branches with an empty first
  block and a non-empty braced else block
- `empty-finally` for empty, comment-free finalizers when a catch remains
- `redundant-local-key-remap` for similar property/value names such as
  `{ action_id: actionId }`, but only when a closed-world semantic proof can
  atomically rename the definition and every use to `{ actionId }`

All twenty also appear during an ordinary scan as findings with `fixable: true`
in JSON and `fixable with --fix` in text. `--fix` stages byte-range edits in
memory, rejects stale ranges, never applies overlapping ranges in the same pass,
reparses every result, writes each changed file atomically, rolls earlier files
back if a write fails, and then performs a fresh scan. The score and
`--fail-above` decision therefore describe the post-fix tree. A second `--fix`
run is expected to make zero changes.

Comments inside a candidate span, parser errors, TypeScript declaration files,
mixed declarations, uninitialized bindings, writes, symlinks, and ambiguous
shapes are not rewritten; direct `eval` disables `prefer-const`. The fixer
also retains numeric-literal bracket access, computed `__proto__`, labelled loop
control, documented terminal statements, non-literal JSX values, dissimilar
assertions, identity-only types, and dangling-else-sensitive branches. It
preserves untouched bytes, including BOMs and line endings. Architectural
findings, unsafe assertions, complex flow, input mutation, async behavior,
clones, and naming findings remain review-only because changing them requires
human judgment.

Key-remap fixes are deliberately stricter than textual matching. The Oxc
semantic graph must prove a function-local `const` object has no type contract,
reassignment, untracked alias, dynamic/reflection use, spread, collision, or
escape. It follows nested objects, direct `const` aliases, containment, static
dot/bracket reads, and a single untyped local-helper call. Passing or returning
the object, serializing it, sending it to an API, using a typed owner or helper
parameter, or encountering any unresolved path blocks the whole atomic change.
Blocked candidates appear as zero-point `suspicious-key-remap` review signals:
they make the safety decision visible without changing the slop score.

The CLI currently evaluates 62 finding types for TypeScript and 37 for Rust.
The local code checks shared by both languages are:

- `long-function`, `complex-function`, and `deep-nesting`
- `parameter-bundle`, `large-file`, `vague-names`, and
  `generic-function-name`
- `wrapper-cluster`
- `boolean-soup`, `else-if-chain`, and `branch-fanout`
- `exit-point-cluster`, `branch-dense-function`, and `nested-callbacks`
- `async-without-await`, `mutation-cluster`, and `boolean-parameter-cluster`
- `tangled-chain`, `input-mutation`, and `boolean-call-soup`
- `error-laundering` and `assertionless-test`

TypeScript additionally checks `nested-ternary`, `any-cluster`, and
`assertion-cluster`, plus `empty-catch`. Rust additionally checks clustered
`unwrap`, `expect`, and panic-style exits with `panic-path-cluster`. The local
flow-readability rules use conservative boundaries and emit one measured
finding per function and rule.

`generic-function-name` splits camelCase, PascalCase, and snake_case names into
words and recognizes wholly generic verb/noun pairs such as `processData`,
`handleRequest`, `doWork`, and `executeTask`. Very vague nouns are reported on
smaller non-trivial bodies; boundary terms such as request, response, event,
and message require substantially more flow. Thin wrappers, tests, compact
framework adapters, and names containing domain vocabulary such as
`processInvoiceData` remain quiet. The rule is review-only.

`input-mutation` reports the first caller-owned mutation in TypeScript. Because
Rust makes mutation explicit in the type system, it only reports functions that
mutate at least two `&mut` inputs across three or more sites.

Terraform (`.tf`, `.tfvars`) and Terragrunt (`.hcl`) add 15 HCL-specific checks:

- `oversized-hcl-block`, `deep-hcl-nesting`, and `complex-hcl-expression`
- `large-hcl-collection`, `dynamic-block-cluster`, and `local-value-cluster`
- `untyped-variable-cluster` and `undocumented-interface-cluster`
- `floating-module-source`, `broad-ignore-changes`, and
  `explicit-dependency-cluster`
- `terragrunt-dependency-cluster`, `terragrunt-hook-cluster`,
  `terragrunt-config-read-cluster`, and `terragrunt-include-cluster`

Local Terraform module sources and static Terragrunt `dependency`,
`dependencies`, `include`, and `read_terragrunt_config` paths feed the same
repository and folder graphs as TypeScript imports and Rust modules. Large HCL
blocks also participate in normalized cross-file clone detection.

After the parallel file scan, a zero-configuration repository graph adds these
architecture checks:

- `dependency-cycle`, `module-fanout`, and `coupling-hub`
- `barrel-maze` and `unstable-dependency`
- `workspace-boundary-bypass` for TypeScript packages with existing `exports`
  maps
- `structural-clone` for large normalized functions repeated across non-test files

The same graph is aggregated into folders for structural checks:

- `crowded-folder`, `wide-folder`, and `deep-folder-chain`
- `wrapper-directory` and `folder-dependency-cycle`
- `folder-coupling-hub`, `misplaced-module`, and `catch-all-folder`

There is no Slop configuration file. Absolute minimums prevent findings on
small repositories, while repository percentiles adapt fan-out, width, and
coupling thresholds to the codebase. Existing `package.json` metadata is read
when it can prove a workspace boundary.

The walker respects `.gitignore` and processes files in parallel. `node_modules`,
build output, coverage output, fixture directories, `.terraform`,
`.terragrunt-cache`, Terraform lock files, and TypeScript declaration files are
skipped by default. Pointing directly at a skipped directory still scans it.
Parsing is performed directly with Oxc for TypeScript, rust-analyzer's lossless
syntax parser for Rust, and Tree-sitter HCL for Terraform and Terragrunt; the
scan does not start Node, `tsc`, `rustc`, Cargo, Terraform, or Terragrunt.

## Architecture

The analyzers intentionally do not pretend TypeScript and Rust have the same
AST. Each parser has a thin adapter that emits language-neutral facts and
events. One shared engine turns those facts into findings and one scorer turns
findings into the repository score. A second linear pass constructs module and
folder graphs:

```text
Oxc TypeScript AST ─┐
                    ├─> facts/events ─> shared rules ─> shared score
rust-analyzer CST ──┤
Tree-sitter HCL CST ┘

resolved imports/use edges ─> module graph ─> architecture findings
                                      └─────> folder graph ─> structure findings

large named function spans ─┐
large top-level HCL blocks ─┴─> normalized tokens ─> cross-file clone groups
```

That boundary shares the product logic without hiding language semantics. The
Rust parser always returns a tree, so valid regions of an incomplete file still
produce findings while syntax errors are counted separately. If a file uses a
pre-2024 keyword as an identifier, the parser automatically selects the edition
that preserves the most syntax.

The HCL adapter treats blocks, expressions, module interfaces, lifecycle rules,
and Terragrunt orchestration as configuration concepts rather than pretending
they are functions. Tree-sitter also preserves partial-file analysis when HCL
is temporarily invalid.

Rust macro calls, `macro_rules!` transcribers, and macro 2.0 bodies are inspected
recursively when their token trees contain Rust items, statements, expressions,
closures, or control flow. Arbitrary non-Rust macro DSLs do not become file parse
errors; their coverage is reported through `macro_inputs_unresolved`. The score
is intentionally based on human-authored source rather than compiler-generated
expansion output.

Every finding includes a rule-specific `remediation_prompt` in JSON and a
copyable `LLM prompt` in text output. Parser diagnostics and fatal CLI errors
also include prompts that preserve the failing path and recommend fixing the
underlying source or invocation rather than excluding it.

In five-run checks on the development machine, a mixed tree of 1,541 files and
509,354 non-empty lines scanned in a median of 273 ms with zero file parse
errors: about 1.9 million non-empty lines per second. Use the `elapsed_ms` JSON
field to benchmark your own machines and repositories.

## Build

```sh
cargo build --release
cargo install --path .
```

The executable is written to `target/release/slop` (`slop.exe` on Windows).

## Score

The score is a bounded debt-density index from 0 to 100. Findings receive
transparent debt points, the points are normalized per thousand non-empty lines,
and the density is mapped onto the bounded score. Scan roots smaller than 500
lines use a 500-line floor so one finding does not overwhelm a tiny sample:

```text
score = 100 × (1 − e^(−points_per_kloc / 75))
```

This version-zero score is intended for comparing folders and tracking a codebase
over time. Its constants should eventually be calibrated against timed human
maintenance tasks.
