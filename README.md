# StockCraft

![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)
![Rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange?logo=rust)
![MCP stdio](https://img.shields.io/badge/MCP-stdio-purple)
![ArtCraft harness](https://img.shields.io/badge/ArtCraft-harness-green)

**Local-first inventory and light warehouse management (*Lagerhaltung / Warenwirtschaft*) in pure Rust.** Part of the `craft-biz` Sovereign Back-Office Suite.

Replaces the inventory/stock core of **inFlow, Sortly, Zoho Inventory, Katana (parts), and weclapp Lager** without per-seat SaaS rent or cloud lock-in.

## Features

- **SKUs & Bin Locations**: SQLite WAL store tracking SKUs, bins, and stock levels across warehouse locations.
- **Immutable Movement Ledger**: `GoodsIn`, `GoodsOut`, `Transfer`, `InventurDiff`, and `Storno` movements protected by SQLite `BEFORE UPDATE/DELETE` triggers.
- **Moving-Average Valuation**: Integer-cent inventory valuation without floating-point drift.
- ***Meldebestand* & *Inventur***: Low-stock reorder alerts and full stocktake (*Inventur*) sessions.
- **Domino Event Bus**: Emits `stock.low`, `stock.cogs_booked`, and `stock.inventory_adjusted` on the `craft-biz` bus.

## The Craft App Standard (3-Tier ArtCraft Architecture)

StockCraft implements the **Pure-Rust ArtCraft** 3-tier containment harness:

| Layer | Crate | Role |
|---|---|---|
| **L1 Core** | `crates/stockcraft-core` | SQLite WAL store, immutable movement ledger, integer-cent valuation, `craft-biz` event bus, and universal `Command` dispatch with zero-panic lint walls (`unwrap_used` / `expect_used` / `panic` denied). |
| **L2 CLI + MCP** | `crates/stockcraft-mcp` | Single native `stockcraft` binary providing `clap` CLI subcommands (`--json`) and a JSON-RPC `2024-11-05` MCP stdio server (`mcp`) with zero Python/Node runtime dependencies. |
| **L3 Desktop App** | `crates/stockcraft-app` | Native GPU-rendered `eframe`/`egui` 0.37 desktop workstation (`--db`, `--dark`, `--smoke`) invoking L1 directly in-process (zero HTTP/WebView2 overhead). |

**Lineage**: Adapted from the Storytold ArtCraft pattern ([storytold/artcraft](https://github.com/storytold/artcraft)): one workspace, one command bus, headless-first CLI + MCP + native GUI.

## Quick Start

```powershell
cargo run -p stockcraft-mcp -- seed
cargo run -p stockcraft-mcp -- status --json
cargo run -p stockcraft-mcp -- mcp
cargo run -p stockcraft-app
```

## MCP Server Configuration

```json
{
  "mcpServers": {
    "stockcraft": {
      "command": "D:/Dev/repos/stockcraft/target/release/stockcraft.exe",
      "args": ["mcp"]
    }
  }
}
```

## Verification

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## License

MIT OR Apache-2.0
