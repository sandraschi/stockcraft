# Changelog

## 0.2.0 (2026-11-15) - Tier 5
- Added batch/lot & MHD expiry tracking in FEFO order (`stock_lot_receive`, `stock_lots`, `stock_expiring_lots`) + **Lots & MHD** view in `stockcraft-app`.
- Added *Niederstwertprinzip* write-downs (`stock_writedown`) emitting `stock.inventory_adjusted` to `buchcraft`.
- Added reorder planning (`stock_reorder_plan`) with optional `stock.reorder_requested` bus emission.
- Added CSV import (`stock_import_csv` / `stockcraft import-csv`) and `File > Import CSV...`.
- Added `SuiteApp::info()`, `craft_biz_ui::coverage` test verifying all 25 MCP tools are covered in the UI, and extensive `HELP.md` user guide.

## 0.1.0 (2026-11-15) - Tiers 1-4
- Initial L1/L2/L3 implementation (`stockcraft-core`, `stockcraft-mcp`, `stockcraft-app`).
- `craft-biz-ui` `SuiteApp` + `Shell` desktop workstation (Craft Menu Bar Standard, `Ctrl+K` palette, Signals dock, DE/EN, dark/light, `--smoke`, `--screenshot`).
- Pick lists, CSV export, `craft-biz` bus inbox drain, `tools --markdown` and `docs/TOOLS.md` sync test.
