# StockCraft

Local-first inventory and light warehouse management (*Lagerhaltung / Warenwirtschaft*) in pure Rust. Part of the `craft-biz` Back-Office suite.

Replaces the inventory/stock core of **inFlow, Sortly, Zoho Inventory, Katana (parts), and weclapp Lager** — without per-seat SaaS rent or cloud lock-in.

## Architecture (3-Tier ArtCraft Pattern)

- **L1 `stockcraft-core`**: SQLite WAL store, SKUs & bin locations, immutable movement ledger (`GoodsIn`, `GoodsOut`, `Transfer`, `InventurDiff`, `Storno` with SQLite `BEFORE UPDATE/DELETE` triggers), moving-average valuation in integer cents, *Meldebestand* low-stock alerts, *Inventur* (stocktake) sessions, and `craft-biz` event bus (`stock.low`, `stock.cogs_booked`, `stock.inventory_adjusted`).
- **L2 `stockcraft-mcp` (`stockcraft` binary)**: Dual CLI (`--json`) + MCP `2024-11-05` stdio server over one `Command` dispatch.
- **L3 `stockcraft-app`**: Native `egui 0.37` desktop workstation (`--db`, `--dark`, `--smoke`).

## Quick Start

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stockcraft-mcp -- seed
cargo run -p stockcraft-mcp -- status --json
cargo run -p stockcraft-mcp -- mcp
cargo run -p stockcraft-app
```

## License

MIT OR Apache-2.0
