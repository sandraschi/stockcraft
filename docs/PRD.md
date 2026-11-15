# StockCraft PRD

## Purpose
`stockcraft` is the local-first, offline-capable inventory and light warehouse management app (*Lagerhaltung*) in the Back-Office Craft Suite. It replaces cloud SaaS tools such as Sortly, Katana Inventory, inFlow, Cin7 Core, and Billbee-Lager with a single local SQLite WAL ledger, CLI/MCP server, and native `egui` workstation.

## Architecture
- **L1 (`stockcraft-core`)**: SQLite WAL schema (`items`, immutable `movements`, `stocktakes`, `stocktake_lines`, `pick_lists`, `pick_list_lines`, `lots`), integer-cent moving-average valuation, *Niederstwertprinzip* write-downs, reorder planning, CSV import/export, `Command` enum + `dispatch`, `craft-biz` bus integration, and MCP tool catalog (`tools()`).
- **L2 (`stockcraft-mcp`)**: `stockcraft` binary offering both human/JSON CLI subcommands and a `2024-11-05` MCP stdio server (`25` tools).
- **L3 (`stockcraft-app`)**: Native `egui 0.37` desktop app built on `craft-biz-ui` (`Shell` + `SuiteApp`) with 6 views, DE/EN i18n, light/dark themes, `Ctrl+K` command palette, `Signals` bus dock, embedded docs (`HELP.md`, `PRD.md`, `TOOLS.md`, `README.md`), and headless `--smoke` / `--screenshot` support.

## Today (v0.2.0 / Tier 5)
- Moving-average valuation in integer cents (no floats).
- Immutable movement ledger with `BEFORE UPDATE` / `BEFORE DELETE` SQLite triggers and linked `storno` reversals.
- Bin transfers (`transfer`), *Meldebestand* (`low_alerts`), reorder planning (`reorder_plan`), CSV export & import (`export_csv`, `import_csv`).
- *Inventur* workflow (`inventur_start`, `inventur_count`, `inventur_commit`).
- Pick lists (`pick_create`, `pick_list`, `pick_commit`) and `craft-biz` bus intake (`inbox_drain` for `invoice.issued` and `bom.exported`) + outbound events (`stock.low`, `stock.cogs_booked`, `stock.inventory_adjusted`, `stock.reorder_requested`).
- Batch/lot & MHD expiry tracking in FEFO order (`lot_receive`, `lots`, `expiring_lots`).
- *Niederstwertprinzip* write-down (`writedown`) emitting `stock.inventory_adjusted` to `buchcraft`.
- Extensive user guide (`HELP.md`) and 100% `craft_biz_ui::coverage` tool verification.

## Honest Gaps & Next Steps
- Multi-warehouse separate cost pools (per-warehouse average cost) are single-ledger today.
- Serialized single-unit tracking (1-to-1 IMEI/serial numbers) and USB-HID barcode scanner wedge mode are candidates for future tiers.
