# StockCraft User Guide & Reference (`HELP.md`)

`stockcraft` is the local-first, offline-capable inventory and light warehouse management app (*Lagerhaltung*) in the Back-Office Craft Suite. All stock valuations use integer cents (`i64`) and all movements are recorded in an immutable SQLite WAL ledger.

---

## 1. Quick Start (GUI, CLI, MCP)

### Desktop Workstation (`stockcraft-app`)
```powershell
stockcraft-app --demo          # Open with demo SKUs and movements
stockcraft-app --lang de       # German UI (or --lang en for English)
stockcraft-app --dark          # Dark theme
stockcraft-app --smoke         # Headless health check (prints JSON status and exits)
```
Press **`Ctrl+K`** anywhere in the window to open the **Command Palette**, which auto-generates forms for every `stock_*` MCP tool.
Open **`Help > Project Files & Docs`** to read `README.md`, `HELP.md`, `docs/PRD.md`, and `docs/TOOLS.md` inside the app.

### CLI (`stockcraft`)
```powershell
stockcraft status
stockcraft seed
stockcraft list
```
Use `--db <PATH>` to target a specific database (default: `$STOCKCRAFT_DB` or `%LOCALAPPDATA%\stockcraft\stockcraft.db`) and `--json` for machine-readable output.

### MCP Server (`2024-11-05` stdio)
```powershell
stockcraft mcp
```
`stockcraft` exposes 25 `stock_*` tools over JSON-RPC stdio. See `docs/TOOLS.md` for the full schema catalog.

---

## 2. Core Inventory Workflows

### A. Create or Update SKUs (*Artikelstamm*)
- **GUI**: Click `+ New SKU` in the top bar or `File > New SKU...`.
- **CLI**:
  ```powershell
  stockcraft item-upsert BOLT-M6 "Bolt M6 Stainless" --ean 4001234567890 --unit pcs --location A-01-01 --min-stock 50 --ekr 1600
  ```
- Fields:
  - `sku`: Unique article code.
  - `location`: Warehouse bin (e.g. `A-01-01`).
  - `min_stock`: *Meldebestand* threshold; triggers low-stock alerts and `stock.low` bus events.
  - `ekr_account`: Austrian EKR inventory account (default `1600` *Vorraete*).

### B. Receiving Stock (*Wareneingang* & Moving-Average Costing)
- **GUI**: Click `Receive` in the top bar or `+` on any row in **Inventory**.
- **CLI**:
  ```powershell
  stockcraft receive BOLT-M6 200 --cost-cents 12 --reference PO-2026-01
  ```
- **How Moving-Average Valuation Works**:
  When you receive `qty` units at `unit_cost_cents`, the new average cost is:
  `((old_qty * old_cost) + (qty * unit_cost_cents)) / (old_qty + qty)`
  All arithmetic uses 64-bit integer cents (no floating-point rounding drift).

### C. Issuing Stock (*Warenausgang*) & Bin Transfers (*Umlagerung*)
- **GUI**: Click `Issue` or `-` on an Inventory row, or `Stock > Transfer bin...`.
- **CLI**:
  ```powershell
  stockcraft issue BOLT-M6 25 --reference ORD-104
  stockcraft transfer BOLT-M6 B-02-04 --reference REORG-01
  ```
- Issuing stock checks `qty_on_hand >= qty` (never allows negative inventory) and emits:
  - `stock.cogs_booked` -> `buchcraft` (EKR `5000` *Materialaufwand*)
  - `stock.low` -> `taskcraft` (if `qty_on_hand <= min_stock`)

### D. Corrections via *Storno* (Immutable Ledger)
- Stock movements are protected by SQLite `BEFORE UPDATE` and `BEFORE DELETE` triggers. Rows can **never** be edited or deleted.
- **GUI**: Open **Movements** and click `Storno` on any active movement (asks for confirmation).
- **CLI**:
  ```powershell
  stockcraft storno 2 --reason "Wrong quantity entered"
  ```
- Double-storno of the same movement is blocked.

---

## 3. Lots, Batch Numbers & MHD Expiry (FEFO)

Use **Lots & MHD** when tracking batch numbers (*Chargen*) or best-before dates (*Mindesthaltbarkeitsdatum*, `YYYY-MM-DD`):
- **GUI**: Open **Lots & MHD** -> `+ Receive lot` (or `Stock > Receive lot / MHD...`).
- **CLI**:
  ```powershell
  stockcraft lot-receive PASTE-01 10 --cost-cents 2000 --lot LOT-2026-A --expires 2026-12-15 --reference PO-88
  stockcraft lots --sku PASTE-01
  stockcraft expiring-lots --within-days 30
  ```
- Lots are sorted in **FEFO** order (First Expired, First Out). Lots expiring within 30 days are highlighted with a warning pill and a badge in the navigation rail.

---

## 4. Pick Lists & `craft-biz` Bus Integration (*Kommissionierung*)

`stockcraft` connects to the `craft-biz` local event bus:
- **Inbound events consumed** (`stock_inbox_drain`):
  - `invoice.issued` (from `billcraft`) -> automatically creates an open pick list for the invoiced SKUs.
  - `bom.exported` (from `modcraft`) -> automatically creates an open pick list for the bill of materials.
- **Outbound events emitted**:
  - `stock.low` -> `taskcraft`
  - `stock.cogs_booked` -> `buchcraft`
  - `stock.inventory_adjusted` -> `buchcraft` (from *Inventur* or *Niederstwertprinzip* write-downs)
  - `stock.reorder_requested` -> `taskcraft` (from `reorder-plan --emit-bus`)
- **GUI**: Open **Pick lists** (or the **Signals** dock) -> `Drain bus inbox`, review availability per bin, and click `Commit pick` to issue all lines atomically.
- **CLI**:
  ```powershell
  stockcraft inbox-drain
  stockcraft pick-list
  stockcraft pick-commit 1
  ```

---

## 5. Physical Stocktake (*Inventur*)

1. **Start**: Open **Inventur** -> `Start Inventur` (or `stockcraft inventur-start --note "Year-end 2026"`).
2. **Count**: Enter the counted physical quantity for each SKU and click `Save` (or `stockcraft inventur-count <ID> <SKU> <QTY>`).
3. **Commit**: Click `Commit Inventur` (or `stockcraft inventur-commit <ID>`).
   - Uncounted lines default to `expected_qty`.
   - Any variance posts an `inventur_diff` movement and emits `stock.inventory_adjusted` to `buchcraft`.

---

## 6. *Niederstwertprinzip* Write-Downs & Reorder Planning

### *Niederstwertprinzip* (UGB / HGB Lower-of-Cost-or-Market)
- If stock value drops below moving-average cost (damage, obsolescence, near-expiry):
  - **GUI**: `Stock > Write-down (Niederstwert)...` (or `Ctrl+K` -> `stock_writedown`).
  - **CLI**: `stockcraft writedown PASTE-01 1500 --reason "Near MHD"`
- Write-ups above current cost are rejected; write-downs post an immutable `writedown` movement and emit `stock.inventory_adjusted` to `buchcraft`.

### Reorder Planning (*Bestellvorschlag*)
- **GUI**: Open **Low stock** -> `Reorder plan` (or `Stock > Reorder plan`).
- **CLI**: `stockcraft reorder-plan` (add `--emit-bus` to send `stock.reorder_requested` tasks to `taskcraft`).

---

## 7. CSV Import & Export

- **Export**: `File > Export > Inventory CSV` or `stockcraft export-csv`.
- **Import**: `File > Import CSV...` or `stockcraft import-csv <FILE.csv>`.
  Required columns: `sku,name`. Optional columns: `ean,unit,location,qty_on_hand,min_stock,unit_cost_cents,ekr_account`.

---

## 8. Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+K` | Open Command Palette (schema forms for all 25 tools) |
| `Ctrl+Plus` / `Ctrl+Wheel` | Zoom UI in |
| `Ctrl+Minus` | Zoom UI out |
| `Ctrl+0` | Reset zoom |
| `F1` | Open Help & Project Docs |

---

## 9. USB-HID Barcode Scanner Wedge (`stock_scan`)

Any USB-HID barcode scanner (which types the EAN/SKU followed by `Enter`) works directly in **Inventory > Barcode / EAN**:
- Select **Lookup**, **+1 Receive**, or **-1 Issue**, place focus in the scan box, and scan.
- **CLI**:
  ```powershell
  stockcraft scan 4001234567890 --action issue --qty 1 --reference POS-SCAN
  ```

---

## 10. Troubleshooting

- **`insufficient stock for SKU`**: You cannot issue more units than `qty_on_hand`. Receive stock first or check open pick lists.
- **`movements are immutable; use storno`**: Direct SQL `UPDATE`/`DELETE` on `movements` is blocked by triggers. Always use `storno`.
- **`docs/TOOLS.md is stale`**: Run `just docs` (or `stockcraft tools --markdown`) after changing any MCP tool schema.
