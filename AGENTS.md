# AGENTS.md - stockcraft

Local-first inventory & light warehouse management craft app (*Lagerhaltung*, *Wareneingang*, *Warenausgang*, *Kommissionierung*, *Inventur*). Read first:
`mcp-central-docs/standards/RUST_ARTCRAFT_STANDARD.md`,
`mcp-central-docs/projects/craft-fleet-expansion/BACKOFFICE_CRAFT_SUITE.md`.

## Layout

- `crates/stockcraft-core` (L1): SQLite WAL inventory store, immutable `movements` ledger, moving-average valuation in integer cents, *Inventur* stocktakes, `pick_lists`, `craft-biz` bus inbox/outbox, `Command` dispatch, and `tools()` catalog.
- `crates/stockcraft-mcp` (L2): `stockcraft` binary. CLI verbs map onto the same `dispatch` path as MCP `2024-11-05` stdio tools.
- `crates/stockcraft-app` (L3, `egui 0.37` via `craft-biz-ui`): `stockcraft-app` desktop window (`Shell` + `SuiteApp`). Mutations go through `dispatch`; reads use `Store`. Supports `--smoke`, `--screenshot PATH --view NAME`, `--lang de|en`, `--dark`, `--demo`.
- Shared kernel: `../craft-biz` (`Money`, `Bus`, `mcp`, `craft-biz-ui`).

## Inventory invariants (never weaken)

- Stock movements are immutable (`BEFORE UPDATE` and `BEFORE DELETE` triggers on `movements`). Corrections use `storno`.
- Double-storno is rejected (`stornoed = 1` flag on original movement).
- Stock issues cannot drive `qty_on_hand` below zero (`InsufficientStock`).
- Valuation uses integer cents (`i64`) and moving-average cost on `receive`; no floats for currency.

## Docs (keep current in the same commit)

- `README.md`: what it does, quick start, MCP config, build.
- `HELP.md`: task-oriented how-to; update when a verb or workflow changes.
- `docs/PRD.md`: purpose, boundaries, honest gaps, next steps.
- `docs/TOOLS.md`: generated. After any tool change run `just docs`; `tools_doc_is_current` fails otherwise.
- `CHANGELOG.md`: one entry per change set.

## Rules

- Zero-panic: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` denied outside tests.
- MCP mode: stdout is JSON-RPC only; diagnostics to stderr.
- Never use the word "sovereign" in code, UI, or docs (use local, offline, airgapped).
- No em dash characters in code, docs, or commit messages.
- Demo data only via `stock_seed` / `seed`.

## Gates

1. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
2. `cargo test --workspace`
3. `cargo fmt --all --check`
4. `stockcraft-app --smoke` and MCP `initialize` -> `tools/list` probe
