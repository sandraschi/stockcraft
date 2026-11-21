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


## Documentation preservation and hooks

On 2026-11-20 a README "standardization" sweep across the fleet deleted project descriptions,
status sections and user warnings in nine repos. Full rules:
`mcp-central-docs/standards/CONTENT_PRESERVATION_STANDARD.md`.

- Applying any standard or template restructures; it never deletes. Move content (within the file
  or to `docs/` with a link), do not drop it. Never remove what the project is, what it is based on,
  its status (what works, what does not, what is not built) or warnings to users.
- Remove content only when Sandra explicitly asks for that removal, in this repo.
- Never add claims you have not verified (release downloads, versions, test counts, features).
- Only standards with `status: active` may be applied across repos. A change across several repos is
  done on one repo first and approved by Sandra; its commits name the tool and carry a `Sweep-Id:` line.
- `.githooks/commit-msg` blocks a commit that deletes a Markdown file, removes 20% of its lines
  (15 or more) or removes 3 or more headings. A removal Sandra asked for passes with
  `Docs-Removal-Approved: <who and why>` in the message. Enable once per clone:
  `git config core.hooksPath .githooks`. Never use `--no-verify`; if a hook blocks you, stop and ask.
