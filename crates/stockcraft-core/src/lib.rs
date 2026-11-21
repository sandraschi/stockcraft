//! `stockcraft-core` (L1): SQLite WAL inventory store, immutable movement ledger,
//! moving-average valuation in integer cents, *Inventur* (stocktake), and `craft-biz` bus.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::Utc;
use craft_biz::bus::Bus;
use craft_biz::mcp::tool;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use thiserror::Error;

pub const APP: &str = "stockcraft";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum StockError {
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("invalid argument: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("insufficient stock for {sku}: have {have}, requested {need}")]
    InsufficientStock { sku: String, have: i64, need: i64 },
    #[error("bus error: {0}")]
    Bus(String),
}

pub type Result<T> = std::result::Result<T, StockError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub sku: String,
    pub name: String,
    pub ean: String,
    pub unit: String,
    pub location: String,
    pub min_stock: i64,
    pub qty_on_hand: i64,
    pub unit_cost_cents: i64,
    pub ekr_account: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Movement {
    pub id: i64,
    pub created_at: String,
    pub sku: String,
    pub kind: String,
    pub qty_delta: i64,
    pub unit_cost_cents: i64,
    pub location: String,
    pub reference: String,
    pub storno_of: Option<i64>,
    pub stornoed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventurLine {
    pub sku: String,
    pub name: String,
    pub location: String,
    pub expected_qty: i64,
    pub counted_qty: Option<i64>,
    pub unit_cost_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickList {
    pub id: i64,
    pub source: String,
    pub reference: String,
    pub status: String,
    pub created_at: String,
    pub committed_at: Option<String>,
    pub line_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickLine {
    pub pick_list_id: i64,
    pub sku: String,
    pub qty: i64,
    pub location: String,
    pub qty_on_hand: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickLineInput {
    pub sku: String,
    pub qty: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lot {
    pub id: i64,
    pub sku: String,
    pub lot_no: String,
    pub expires_at: String,
    pub qty_on_hand: i64,
    pub unit_cost_cents: i64,
    pub received_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorderItem {
    pub sku: String,
    pub name: String,
    pub location: String,
    pub qty_on_hand: i64,
    pub min_stock: i64,
    pub suggested_qty: i64,
    pub unit_cost_cents: i64,
    pub estimated_cost_cents: i64,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| StockError::Invalid(format!("create db dir: {e}")))?;
            }
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let s = Self { conn };
        s.migrate()?;
        Ok(s)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let s = Self { conn };
        s.migrate()?;
        Ok(s)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS items (
                sku TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                ean TEXT NOT NULL DEFAULT '',
                unit TEXT NOT NULL DEFAULT 'pcs',
                location TEXT NOT NULL DEFAULT 'A-01-01',
                min_stock INTEGER NOT NULL DEFAULT 0,
                qty_on_hand INTEGER NOT NULL DEFAULT 0,
                unit_cost_cents INTEGER NOT NULL DEFAULT 0,
                ekr_account TEXT NOT NULL DEFAULT '1600'
            );

            CREATE TABLE IF NOT EXISTS movements (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at TEXT NOT NULL,
                sku TEXT NOT NULL REFERENCES items(sku),
                kind TEXT NOT NULL,
                qty_delta INTEGER NOT NULL,
                unit_cost_cents INTEGER NOT NULL,
                location TEXT NOT NULL,
                reference TEXT NOT NULL DEFAULT '',
                storno_of INTEGER REFERENCES movements(id)
            );

            CREATE TRIGGER IF NOT EXISTS trg_movements_no_update
            BEFORE UPDATE ON movements
            BEGIN
                SELECT RAISE(ABORT, 'movements are immutable; use storno');
            END;

            CREATE TRIGGER IF NOT EXISTS trg_movements_no_delete
            BEFORE DELETE ON movements
            BEGIN
                SELECT RAISE(ABORT, 'movements are immutable; use storno');
            END;

            CREATE TABLE IF NOT EXISTS stocktakes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                started_at TEXT NOT NULL,
                committed_at TEXT,
                note TEXT NOT NULL DEFAULT ''
            );

            CREATE TABLE IF NOT EXISTS stocktake_lines (
                stocktake_id INTEGER NOT NULL REFERENCES stocktakes(id),
                sku TEXT NOT NULL REFERENCES items(sku),
                expected_qty INTEGER NOT NULL,
                counted_qty INTEGER,
                unit_cost_cents INTEGER NOT NULL,
                PRIMARY KEY (stocktake_id, sku)
            );

            CREATE TABLE IF NOT EXISTS pick_lists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                reference TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                created_at TEXT NOT NULL,
                committed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS pick_list_lines (
                pick_list_id INTEGER NOT NULL REFERENCES pick_lists(id),
                sku TEXT NOT NULL,
                qty INTEGER NOT NULL CHECK(qty > 0),
                PRIMARY KEY (pick_list_id, sku)
            );

            CREATE TABLE IF NOT EXISTS lots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sku TEXT NOT NULL REFERENCES items(sku),
                lot_no TEXT NOT NULL,
                expires_at TEXT NOT NULL DEFAULT '',
                qty_on_hand INTEGER NOT NULL DEFAULT 0,
                unit_cost_cents INTEGER NOT NULL DEFAULT 0,
                received_at TEXT NOT NULL,
                UNIQUE(sku, lot_no)
            );
            ",
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_item(
        &self,
        sku: &str,
        name: &str,
        ean: &str,
        unit: &str,
        location: &str,
        min_stock: i64,
        ekr_account: &str,
    ) -> Result<Item> {
        let sku = sku.trim();
        let name = name.trim();
        if sku.is_empty() || name.is_empty() {
            return Err(StockError::Invalid("sku and name must not be empty".into()));
        }
        let unit = if unit.trim().is_empty() {
            "pcs"
        } else {
            unit.trim()
        };
        let location = if location.trim().is_empty() {
            "A-01-01"
        } else {
            location.trim()
        };
        let ekr = if ekr_account.trim().is_empty() {
            "1600"
        } else {
            ekr_account.trim()
        };
        if min_stock < 0 {
            return Err(StockError::Invalid("min_stock must be >= 0".into()));
        }

        self.conn.execute(
            "INSERT INTO items (sku, name, ean, unit, location, min_stock, ekr_account)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(sku) DO UPDATE SET
               name = excluded.name,
               ean = excluded.ean,
               unit = excluded.unit,
               location = excluded.location,
               min_stock = excluded.min_stock,
               ekr_account = excluded.ekr_account",
            params![sku, name, ean.trim(), unit, location, min_stock, ekr],
        )?;
        self.get_item(sku)
    }

    pub fn get_item(&self, sku: &str) -> Result<Item> {
        self.conn
            .query_row(
                "SELECT sku, name, ean, unit, location, min_stock, qty_on_hand, unit_cost_cents, ekr_account
                 FROM items WHERE sku = ?1",
                params![sku.trim()],
                |r| {
                    Ok(Item {
                        sku: r.get(0)?,
                        name: r.get(1)?,
                        ean: r.get(2)?,
                        unit: r.get(3)?,
                        location: r.get(4)?,
                        min_stock: r.get(5)?,
                        qty_on_hand: r.get(6)?,
                        unit_cost_cents: r.get(7)?,
                        ekr_account: r.get(8)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StockError::NotFound(format!("sku '{sku}'")))
    }

    pub fn find_by_code(&self, code: &str) -> Result<Item> {
        let trimmed = code.trim();
        if trimmed.is_empty() {
            return Err(StockError::Invalid(
                "barcode/sku code must not be empty".into(),
            ));
        }
        self.conn
            .query_row(
                "SELECT sku, name, ean, unit, location, min_stock, qty_on_hand, unit_cost_cents, ekr_account
                 FROM items WHERE sku = ?1 OR (ean != '' AND ean = ?1) LIMIT 1",
                params![trimmed],
                |r| {
                    Ok(Item {
                        sku: r.get(0)?,
                        name: r.get(1)?,
                        ean: r.get(2)?,
                        unit: r.get(3)?,
                        location: r.get(4)?,
                        min_stock: r.get(5)?,
                        qty_on_hand: r.get(6)?,
                        unit_cost_cents: r.get(7)?,
                        ekr_account: r.get(8)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StockError::NotFound(format!("code '{trimmed}'")))
    }

    pub fn list_items(&self) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT sku, name, ean, unit, location, min_stock, qty_on_hand, unit_cost_cents, ekr_account
             FROM items ORDER BY sku ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Item {
                sku: r.get(0)?,
                name: r.get(1)?,
                ean: r.get(2)?,
                unit: r.get(3)?,
                location: r.get(4)?,
                min_stock: r.get(5)?,
                qty_on_hand: r.get(6)?,
                unit_cost_cents: r.get(7)?,
                ekr_account: r.get(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Wareneingang: adds quantity and updates moving-average unit cost in integer cents.
    pub fn receive(
        &self,
        sku: &str,
        qty: i64,
        unit_cost_cents: i64,
        reference: &str,
    ) -> Result<Movement> {
        if qty <= 0 {
            return Err(StockError::Invalid("receive qty must be > 0".into()));
        }
        if unit_cost_cents < 0 {
            return Err(StockError::Invalid("unit_cost_cents must be >= 0".into()));
        }
        let item = self.get_item(sku)?;
        let old_val = item.qty_on_hand * item.unit_cost_cents;
        let add_val = qty * unit_cost_cents;
        let new_qty = item.qty_on_hand + qty;
        let new_avg = if new_qty > 0 {
            (old_val + add_val) / new_qty
        } else {
            unit_cost_cents
        };

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE items SET qty_on_hand = ?1, unit_cost_cents = ?2 WHERE sku = ?3",
            params![new_qty, new_avg, item.sku],
        )?;
        self.conn.execute(
            "INSERT INTO movements (created_at, sku, kind, qty_delta, unit_cost_cents, location, reference)
             VALUES (?1, ?2, 'GoodsIn', ?3, ?4, ?5, ?6)",
            params![now, item.sku, qty, unit_cost_cents, item.location, reference.trim()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_movement(id)
    }

    /// Warenausgang: deducts quantity at current moving-average cost.
    pub fn issue(&self, sku: &str, qty: i64, reference: &str) -> Result<Movement> {
        if qty <= 0 {
            return Err(StockError::Invalid("issue qty must be > 0".into()));
        }
        let item = self.get_item(sku)?;
        if item.qty_on_hand < qty {
            return Err(StockError::InsufficientStock {
                sku: item.sku,
                have: item.qty_on_hand,
                need: qty,
            });
        }
        let new_qty = item.qty_on_hand - qty;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE items SET qty_on_hand = ?1 WHERE sku = ?2",
            params![new_qty, item.sku],
        )?;
        self.conn.execute(
            "INSERT INTO movements (created_at, sku, kind, qty_delta, unit_cost_cents, location, reference)
             VALUES (?1, ?2, 'GoodsOut', ?3, ?4, ?5, ?6)",
            params![now, item.sku, -qty, item.unit_cost_cents, item.location, reference.trim()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_movement(id)
    }

    /// Umlagerung: moves item to a new bin location and logs a 0-delta Transfer event.
    pub fn transfer(&self, sku: &str, to_location: &str, reference: &str) -> Result<Movement> {
        let to_loc = to_location.trim();
        if to_loc.is_empty() {
            return Err(StockError::Invalid("to_location must not be empty".into()));
        }
        let item = self.get_item(sku)?;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE items SET location = ?1 WHERE sku = ?2",
            params![to_loc, item.sku],
        )?;
        let note = if reference.trim().is_empty() {
            format!("{} -> {to_loc}", item.location)
        } else {
            format!("{} -> {to_loc} ({})", item.location, reference.trim())
        };
        self.conn.execute(
            "INSERT INTO movements (created_at, sku, kind, qty_delta, unit_cost_cents, location, reference)
             VALUES (?1, ?2, 'Transfer', 0, ?3, ?4, ?5)",
            params![now, item.sku, item.unit_cost_cents, to_loc, note],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_movement(id)
    }

    /// Reverses a movement via a linked Storno row (original row stays untouched).
    pub fn storno(&self, movement_id: i64, reason: &str) -> Result<Movement> {
        let orig = self.get_movement(movement_id)?;
        if orig.storno_of.is_some() {
            return Err(StockError::Invalid("cannot storno a storno entry".into()));
        }
        if orig.stornoed {
            return Err(StockError::Invalid(format!(
                "movement #{movement_id} already stornoed"
            )));
        }
        let item = self.get_item(&orig.sku)?;
        let rev_delta = -orig.qty_delta;
        let new_qty = item.qty_on_hand + rev_delta;
        if new_qty < 0 {
            return Err(StockError::InsufficientStock {
                sku: item.sku,
                have: item.qty_on_hand,
                need: -rev_delta,
            });
        }
        let now = Utc::now().to_rfc3339();
        let ref_text = if reason.trim().is_empty() {
            format!("Storno #{movement_id}")
        } else {
            format!("Storno #{movement_id}: {}", reason.trim())
        };
        self.conn.execute(
            "UPDATE items SET qty_on_hand = ?1 WHERE sku = ?2",
            params![new_qty, item.sku],
        )?;
        self.conn.execute(
            "INSERT INTO movements (created_at, sku, kind, qty_delta, unit_cost_cents, location, reference, storno_of)
             VALUES (?1, ?2, 'Storno', ?3, ?4, ?5, ?6, ?7)",
            params![now, item.sku, rev_delta, orig.unit_cost_cents, orig.location, ref_text, movement_id],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_movement(id)
    }

    pub fn get_movement(&self, id: i64) -> Result<Movement> {
        self.conn
            .query_row(
                "SELECT m.id, m.created_at, m.sku, m.kind, m.qty_delta, m.unit_cost_cents, m.location, m.reference, m.storno_of,
                        EXISTS(SELECT 1 FROM movements s WHERE s.storno_of = m.id) AS stornoed
                 FROM movements m WHERE m.id = ?1",
                params![id],
                |r| {
                    Ok(Movement {
                        id: r.get(0)?,
                        created_at: r.get(1)?,
                        sku: r.get(2)?,
                        kind: r.get(3)?,
                        qty_delta: r.get(4)?,
                        unit_cost_cents: r.get(5)?,
                        location: r.get(6)?,
                        reference: r.get(7)?,
                        storno_of: r.get(8)?,
                        stornoed: r.get::<_, i64>(9)? != 0,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StockError::NotFound(format!("movement #{id}")))
    }

    pub fn list_movements(&self, limit: i64) -> Result<Vec<Movement>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.created_at, m.sku, m.kind, m.qty_delta, m.unit_cost_cents, m.location, m.reference, m.storno_of,
                    EXISTS(SELECT 1 FROM movements s WHERE s.storno_of = m.id) AS stornoed
             FROM movements m ORDER BY m.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.max(1)], |r| {
            Ok(Movement {
                id: r.get(0)?,
                created_at: r.get(1)?,
                sku: r.get(2)?,
                kind: r.get(3)?,
                qty_delta: r.get(4)?,
                unit_cost_cents: r.get(5)?,
                location: r.get(6)?,
                reference: r.get(7)?,
                storno_of: r.get(8)?,
                stornoed: r.get::<_, i64>(9)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn low_alerts(&self) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT sku, name, ean, unit, location, min_stock, qty_on_hand, unit_cost_cents, ekr_account
             FROM items WHERE qty_on_hand <= min_stock AND min_stock > 0 ORDER BY (min_stock - qty_on_hand) DESC, sku ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Item {
                sku: r.get(0)?,
                name: r.get(1)?,
                ean: r.get(2)?,
                unit: r.get(3)?,
                location: r.get(4)?,
                min_stock: r.get(5)?,
                qty_on_hand: r.get(6)?,
                unit_cost_cents: r.get(7)?,
                ekr_account: r.get(8)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn total_valuation_cents(&self) -> Result<i64> {
        let total: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(qty_on_hand * unit_cost_cents), 0) FROM items",
            [],
            |r| r.get(0),
        )?;
        Ok(total)
    }

    pub fn inventur_start(&self, note: &str) -> Result<i64> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO stocktakes (started_at, note) VALUES (?1, ?2)",
            params![now, note.trim()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute(
            "INSERT INTO stocktake_lines (stocktake_id, sku, expected_qty, unit_cost_cents)
             SELECT ?1, sku, qty_on_hand, unit_cost_cents FROM items",
            params![id],
        )?;
        Ok(id)
    }

    pub fn inventur_count(&self, stocktake_id: i64, sku: &str, counted_qty: i64) -> Result<()> {
        if counted_qty < 0 {
            return Err(StockError::Invalid("counted_qty must be >= 0".into()));
        }
        let changed = self.conn.execute(
            "UPDATE stocktake_lines SET counted_qty = ?1 WHERE stocktake_id = ?2 AND sku = ?3",
            params![counted_qty, stocktake_id, sku.trim()],
        )?;
        if changed == 0 {
            return Err(StockError::NotFound(format!(
                "stocktake #{stocktake_id} sku '{sku}'"
            )));
        }
        Ok(())
    }

    pub fn inventur_lines(&self, stocktake_id: i64) -> Result<Vec<InventurLine>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.sku, i.name, i.location, l.expected_qty, l.counted_qty, l.unit_cost_cents
             FROM stocktake_lines l
             JOIN items i ON i.sku = l.sku
             WHERE l.stocktake_id = ?1
             ORDER BY l.sku ASC",
        )?;
        let rows = stmt.query_map(params![stocktake_id], |r| {
            Ok(InventurLine {
                sku: r.get(0)?,
                name: r.get(1)?,
                location: r.get(2)?,
                expected_qty: r.get(3)?,
                counted_qty: r.get(4)?,
                unit_cost_cents: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn inventur_commit(&self, stocktake_id: i64) -> Result<(i64, i64)> {
        let committed: Option<String> = self
            .conn
            .query_row(
                "SELECT committed_at FROM stocktakes WHERE id = ?1",
                params![stocktake_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StockError::NotFound(format!("stocktake #{stocktake_id}")))?;
        if committed.is_some() {
            return Err(StockError::Invalid(format!(
                "stocktake #{stocktake_id} already committed"
            )));
        }

        let lines = self.inventur_lines(stocktake_id)?;
        let now = Utc::now().to_rfc3339();
        let mut adjusted = 0_i64;
        let mut variance_cents = 0_i64;

        for l in lines {
            let Some(counted) = l.counted_qty else {
                continue;
            };
            let delta = counted - l.expected_qty;
            if delta == 0 {
                continue;
            }
            adjusted += 1;
            variance_cents += delta * l.unit_cost_cents;
            self.conn.execute(
                "UPDATE items SET qty_on_hand = ?1 WHERE sku = ?2",
                params![counted, l.sku],
            )?;
            self.conn.execute(
                "INSERT INTO movements (created_at, sku, kind, qty_delta, unit_cost_cents, location, reference)
                 VALUES (?1, ?2, 'InventurDiff', ?3, ?4, ?5, ?6)",
                params![now, l.sku, delta, l.unit_cost_cents, l.location, format!("Inventur #{stocktake_id}")],
            )?;
        }

        self.conn.execute(
            "UPDATE stocktakes SET committed_at = ?1 WHERE id = ?2",
            params![now, stocktake_id],
        )?;
        Ok((adjusted, variance_cents))
    }

    pub fn active_inventur_id(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM stocktakes WHERE committed_at IS NULL ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn seed_demo(&self) -> Result<usize> {
        if !self.list_items()?.is_empty() {
            return Ok(0);
        }
        self.upsert_item(
            "RAM-DDR5-32",
            "DDR5-5600 32GB ECC Module",
            "9001234000011",
            "pcs",
            "A-01-01",
            8,
            "1600",
        )?;
        self.upsert_item(
            "NVME-2TB-PRO",
            "NVMe PCIe 4.0 2TB SSD",
            "9001234000028",
            "pcs",
            "A-01-02",
            6,
            "1600",
        )?;
        self.upsert_item(
            "PSU-1000W",
            "1000W 80+ Platinum PSU",
            "9001234000035",
            "pcs",
            "B-02-01",
            4,
            "1600",
        )?;
        self.upsert_item(
            "CAT6A-3M",
            "Cat6A Patch Cable 3m",
            "9001234000042",
            "pcs",
            "C-01-04",
            20,
            "1600",
        )?;

        self.receive("RAM-DDR5-32", 14, 11_500, "PO-2026-101")?;
        self.receive("NVME-2TB-PRO", 5, 13_900, "PO-2026-102")?;
        self.receive("PSU-1000W", 6, 16_500, "PO-2026-103")?;
        self.receive("CAT6A-3M", 18, 650, "PO-2026-104")?;
        self.issue("RAM-DDR5-32", 4, "BUILD-WS-04")?;
        Ok(4)
    }

    pub fn create_pick_list(
        &self,
        source: &str,
        reference: &str,
        lines: &[PickLineInput],
    ) -> Result<PickList> {
        if lines.is_empty() {
            return Err(StockError::Invalid(
                "pick list requires at least one line".into(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO pick_lists (source, reference, status, created_at) VALUES (?1, ?2, 'open', ?3)",
            params![source, reference, now],
        )?;
        let id = self.conn.last_insert_rowid();
        for line in lines {
            if line.qty <= 0 {
                return Err(StockError::Invalid(format!(
                    "pick qty for {} must be > 0",
                    line.sku
                )));
            }
            self.conn.execute(
                "INSERT INTO pick_list_lines (pick_list_id, sku, qty) VALUES (?1, ?2, ?3)
                 ON CONFLICT(pick_list_id, sku) DO UPDATE SET qty = qty + excluded.qty",
                params![id, line.sku, line.qty],
            )?;
        }
        self.get_pick_list(id)
    }

    pub fn get_pick_list(&self, id: i64) -> Result<PickList> {
        self.conn
            .query_row(
                "SELECT p.id, p.source, p.reference, p.status, p.created_at, p.committed_at,
                        (SELECT COUNT(*) FROM pick_list_lines l WHERE l.pick_list_id = p.id)
                 FROM pick_lists p WHERE p.id = ?1",
                params![id],
                |r| {
                    Ok(PickList {
                        id: r.get(0)?,
                        source: r.get(1)?,
                        reference: r.get(2)?,
                        status: r.get(3)?,
                        created_at: r.get(4)?,
                        committed_at: r.get(5)?,
                        line_count: r.get(6)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    StockError::Invalid(format!("pick list {id} not found"))
                }
                other => StockError::Sql(other),
            })
    }

    pub fn list_pick_lists(&self) -> Result<Vec<PickList>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.source, p.reference, p.status, p.created_at, p.committed_at,
                    (SELECT COUNT(*) FROM pick_list_lines l WHERE l.pick_list_id = p.id)
             FROM pick_lists p ORDER BY p.id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PickList {
                id: r.get(0)?,
                source: r.get(1)?,
                reference: r.get(2)?,
                status: r.get(3)?,
                created_at: r.get(4)?,
                committed_at: r.get(5)?,
                line_count: r.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_pick_lines(&self, pick_list_id: i64) -> Result<Vec<PickLine>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.pick_list_id, l.sku, l.qty, COALESCE(i.location, ''), COALESCE(i.qty_on_hand, 0)
             FROM pick_list_lines l
             LEFT JOIN items i ON i.sku = l.sku
             WHERE l.pick_list_id = ?1
             ORDER BY l.sku",
        )?;
        let rows = stmt.query_map(params![pick_list_id], |r| {
            Ok(PickLine {
                pick_list_id: r.get(0)?,
                sku: r.get(1)?,
                qty: r.get(2)?,
                location: r.get(3)?,
                qty_on_hand: r.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn commit_pick_list(&self, pick_list_id: i64) -> Result<PickList> {
        let pl = self.get_pick_list(pick_list_id)?;
        if pl.status != "open" {
            return Err(StockError::Invalid(format!(
                "pick list {pick_list_id} is already {}",
                pl.status
            )));
        }
        let lines = self.get_pick_lines(pick_list_id)?;
        for line in &lines {
            let item = self.get_item(&line.sku)?;
            if item.qty_on_hand < line.qty {
                return Err(StockError::InsufficientStock {
                    sku: line.sku.clone(),
                    have: item.qty_on_hand,
                    need: line.qty,
                });
            }
        }
        let reference = format!("PICK-{pick_list_id}:{}", pl.reference);
        for line in &lines {
            self.issue(&line.sku, line.qty, &reference)?;
        }
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE pick_lists SET status = 'committed', committed_at = ?1 WHERE id = ?2",
            params![now, pick_list_id],
        )?;
        self.get_pick_list(pick_list_id)
    }

    pub fn export_csv(&self) -> Result<String> {
        let items = self.list_items()?;
        let mut out = String::from("sku,name,ean,unit,location,qty_on_hand,min_stock,unit_cost_cents,valuation_cents,ekr_account\n");
        for it in items {
            let val = it.qty_on_hand * it.unit_cost_cents;
            let name = it.name.replace('"', "\"\"");
            out.push_str(&format!(
                "{},\"{}\",{},{},{},{},{},{},{},{}\n",
                it.sku,
                name,
                it.ean,
                it.unit,
                it.location,
                it.qty_on_hand,
                it.min_stock,
                it.unit_cost_cents,
                val,
                it.ekr_account
            ));
        }
        Ok(out)
    }

    pub fn receive_lot(
        &self,
        sku: &str,
        qty: i64,
        unit_cost_cents: i64,
        lot_no: &str,
        expires_at: &str,
        reference: &str,
    ) -> Result<(Movement, Lot)> {
        let lot_trim = lot_no.trim();
        if lot_trim.is_empty() {
            return Err(StockError::Invalid("lot_no must not be empty".into()));
        }
        let ref_str = if reference.trim().is_empty() {
            format!("LOT:{lot_trim}")
        } else {
            format!("{reference} [LOT:{lot_trim}]")
        };
        let mv = self.receive(sku, qty, unit_cost_cents, &ref_str)?;
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO lots (sku, lot_no, expires_at, qty_on_hand, unit_cost_cents, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(sku, lot_no) DO UPDATE SET
               qty_on_hand = qty_on_hand + excluded.qty_on_hand,
               expires_at = CASE WHEN excluded.expires_at != '' THEN excluded.expires_at ELSE lots.expires_at END,
               unit_cost_cents = excluded.unit_cost_cents",
            params![sku, lot_trim, expires_at.trim(), qty, unit_cost_cents, now],
        )?;
        let lot = self.conn.query_row(
            "SELECT id, sku, lot_no, expires_at, qty_on_hand, unit_cost_cents, received_at
             FROM lots WHERE sku = ?1 AND lot_no = ?2",
            params![sku, lot_trim],
            |r| {
                Ok(Lot {
                    id: r.get(0)?,
                    sku: r.get(1)?,
                    lot_no: r.get(2)?,
                    expires_at: r.get(3)?,
                    qty_on_hand: r.get(4)?,
                    unit_cost_cents: r.get(5)?,
                    received_at: r.get(6)?,
                })
            },
        )?;
        Ok((mv, lot))
    }

    pub fn list_lots(&self, sku_filter: Option<&str>) -> Result<Vec<Lot>> {
        let mut out = Vec::new();
        if let Some(sku) = sku_filter.filter(|s| !s.trim().is_empty()) {
            let mut stmt = self.conn.prepare(
                "SELECT id, sku, lot_no, expires_at, qty_on_hand, unit_cost_cents, received_at
                 FROM lots WHERE sku = ?1
                 ORDER BY CASE WHEN expires_at = '' THEN 1 ELSE 0 END, expires_at ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![sku], |r| {
                Ok(Lot {
                    id: r.get(0)?,
                    sku: r.get(1)?,
                    lot_no: r.get(2)?,
                    expires_at: r.get(3)?,
                    qty_on_hand: r.get(4)?,
                    unit_cost_cents: r.get(5)?,
                    received_at: r.get(6)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, sku, lot_no, expires_at, qty_on_hand, unit_cost_cents, received_at
                 FROM lots
                 ORDER BY CASE WHEN expires_at = '' THEN 1 ELSE 0 END, expires_at ASC, id ASC",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(Lot {
                    id: r.get(0)?,
                    sku: r.get(1)?,
                    lot_no: r.get(2)?,
                    expires_at: r.get(3)?,
                    qty_on_hand: r.get(4)?,
                    unit_cost_cents: r.get(5)?,
                    received_at: r.get(6)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn expiring_lots(&self, within_days: i64) -> Result<Vec<Lot>> {
        let days = within_days.max(0);
        let cutoff = (Utc::now() + chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();
        let mut stmt = self.conn.prepare(
            "SELECT id, sku, lot_no, expires_at, qty_on_hand, unit_cost_cents, received_at
             FROM lots
             WHERE qty_on_hand > 0 AND expires_at != '' AND expires_at <= ?1
             ORDER BY expires_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![cutoff], |r| {
            Ok(Lot {
                id: r.get(0)?,
                sku: r.get(1)?,
                lot_no: r.get(2)?,
                expires_at: r.get(3)?,
                qty_on_hand: r.get(4)?,
                unit_cost_cents: r.get(5)?,
                received_at: r.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn writedown(&self, sku: &str, new_cost_cents: i64, reason: &str) -> Result<Movement> {
        if new_cost_cents < 0 {
            return Err(StockError::Invalid("new_cost_cents must be >= 0".into()));
        }
        let item = self.get_item(sku)?;
        if new_cost_cents >= item.unit_cost_cents {
            return Err(StockError::Invalid(format!(
                "Niederstwertprinzip: new cost ({new_cost_cents}) must be strictly less than current cost ({})",
                item.unit_cost_cents
            )));
        }
        let delta_per_unit = new_cost_cents - item.unit_cost_cents;
        let now = Utc::now().to_rfc3339();
        let ref_note = format!(
            "{} ({} -> {} ct)",
            if reason.trim().is_empty() {
                "Niederstwertprinzip"
            } else {
                reason.trim()
            },
            item.unit_cost_cents,
            new_cost_cents
        );
        self.conn.execute(
            "UPDATE items SET unit_cost_cents = ?1 WHERE sku = ?2",
            params![new_cost_cents, sku],
        )?;
        self.conn.execute(
            "INSERT INTO movements (created_at, sku, kind, qty_delta, unit_cost_cents, location, reference)
             VALUES (?1, ?2, 'writedown', 0, ?3, ?4, ?5)",
            params![now, sku, delta_per_unit, item.location, ref_note],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get_movement(id)
    }

    pub fn reorder_plan(&self) -> Result<Vec<ReorderItem>> {
        let low = self.low_alerts()?;
        Ok(low
            .into_iter()
            .map(|it| {
                let target = (it.min_stock * 2).max(it.min_stock + 1);
                let suggested_qty = (target - it.qty_on_hand).max(1);
                let estimated_cost_cents = suggested_qty * it.unit_cost_cents;
                ReorderItem {
                    sku: it.sku,
                    name: it.name,
                    location: it.location,
                    qty_on_hand: it.qty_on_hand,
                    min_stock: it.min_stock,
                    suggested_qty,
                    unit_cost_cents: it.unit_cost_cents,
                    estimated_cost_cents,
                }
            })
            .collect())
    }

    pub fn import_csv(&self, csv_text: &str) -> Result<usize> {
        let mut lines = csv_text.lines().filter(|l| !l.trim().is_empty());
        let Some(header_line) = lines.next() else {
            return Err(StockError::Invalid("CSV is empty".into()));
        };
        let headers: Vec<String> = parse_csv_row(header_line)
            .into_iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .collect();
        let idx = |name: &str| headers.iter().position(|h| h == name);
        let sku_i =
            idx("sku").ok_or_else(|| StockError::Invalid("CSV missing 'sku' column".into()))?;
        let name_i =
            idx("name").ok_or_else(|| StockError::Invalid("CSV missing 'name' column".into()))?;
        let ean_i = idx("ean");
        let unit_i = idx("unit");
        let loc_i = idx("location");
        let min_i = idx("min_stock");
        let ekr_i = idx("ekr_account");
        let qty_i = idx("qty_on_hand");
        let cost_i = idx("unit_cost_cents");

        let mut count = 0usize;
        for row_str in lines {
            let cols = parse_csv_row(row_str);
            let get = |i: Option<usize>| -> String {
                i.and_then(|k| cols.get(k))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            };
            let sku = get(Some(sku_i));
            let name = get(Some(name_i));
            if sku.is_empty() || name.is_empty() {
                continue;
            }
            let ean = get(ean_i);
            let unit = {
                let u = get(unit_i);
                if u.is_empty() {
                    "pcs".into()
                } else {
                    u
                }
            };
            let location = {
                let l = get(loc_i);
                if l.is_empty() {
                    "A-01-01".into()
                } else {
                    l
                }
            };
            let min_stock = get(min_i).parse::<i64>().unwrap_or(0);
            let ekr = {
                let e = get(ekr_i);
                if e.is_empty() {
                    "1600".into()
                } else {
                    e
                }
            };
            let item = self.upsert_item(&sku, &name, &ean, &unit, &location, min_stock, &ekr)?;
            let qty = get(qty_i).parse::<i64>().unwrap_or(0);
            let cost = get(cost_i).parse::<i64>().unwrap_or(0);
            if item.qty_on_hand == 0 && qty > 0 {
                self.receive(&sku, qty, cost, "CSV-IMPORT")?;
            }
            count += 1;
        }
        Ok(count)
    }
}

fn parse_csv_row(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    cur.push('"');
                    let _ = chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                out.push(cur.clone());
                cur.clear();
            }
            other => cur.push(other),
        }
    }
    out.push(cur);
    out
}

pub struct Ctx<'a> {
    pub store: &'a Store,
    pub bus: Option<&'a Bus>,
}

#[derive(Debug, Clone)]
pub enum Command {
    Status,
    SeedDemo,
    ListItems,
    ItemUpsert {
        sku: String,
        name: String,
        ean: String,
        unit: String,
        location: String,
        min_stock: i64,
        ekr_account: String,
    },
    Receive {
        sku: String,
        qty: i64,
        unit_cost_cents: i64,
        reference: String,
    },
    Issue {
        sku: String,
        qty: i64,
        reference: String,
    },
    Transfer {
        sku: String,
        to_location: String,
        reference: String,
    },
    Storno {
        movement_id: i64,
        reason: String,
    },
    Movements {
        limit: i64,
    },
    LowAlerts,
    Valuation,
    InventurStart {
        note: String,
    },
    InventurCount {
        stocktake_id: i64,
        sku: String,
        counted_qty: i64,
    },
    InventurCommit {
        stocktake_id: i64,
    },
    PickCreate {
        source: String,
        reference: String,
        lines: Vec<PickLineInput>,
    },
    PickList,
    PickCommit {
        pick_list_id: i64,
    },
    ExportCsv,
    ImportCsv {
        csv: String,
    },
    InboxDrain,
    LotReceive {
        sku: String,
        qty: i64,
        unit_cost_cents: i64,
        lot_no: String,
        expires_at: String,
        reference: String,
    },
    Lots {
        sku: Option<String>,
    },
    ExpiringLots {
        within_days: i64,
    },
    Writedown {
        sku: String,
        new_cost_cents: i64,
        reason: String,
    },
    ReorderPlan {
        emit_bus: bool,
    },
    Scan {
        code: String,
        action: String,
        qty: i64,
        unit_cost_cents: Option<i64>,
        reference: String,
    },
}

fn req_str(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| StockError::Invalid(format!("missing string field '{key}'")))
}

fn opt_str(args: &Value, key: &str, default: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn req_i64(args: &Value, key: &str) -> Result<i64> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| StockError::Invalid(format!("missing integer field '{key}'")))
}

impl Command {
    pub fn from_tool(tool_name: &str, args: &Value) -> Result<Self> {
        match tool_name {
            "stock_status" => Ok(Self::Status),
            "stock_seed" => Ok(Self::SeedDemo),
            "stock_list" => Ok(Self::ListItems),
            "stock_item_upsert" => Ok(Self::ItemUpsert {
                sku: req_str(args, "sku")?,
                name: req_str(args, "name")?,
                ean: opt_str(args, "ean", ""),
                unit: opt_str(args, "unit", "pcs"),
                location: opt_str(args, "location", "A-01-01"),
                min_stock: args.get("min_stock").and_then(Value::as_i64).unwrap_or(0),
                ekr_account: opt_str(args, "ekr_account", "1600"),
            }),
            "stock_receive" => Ok(Self::Receive {
                sku: req_str(args, "sku")?,
                qty: req_i64(args, "qty")?,
                unit_cost_cents: req_i64(args, "unit_cost_cents")?,
                reference: opt_str(args, "reference", ""),
            }),
            "stock_issue" => Ok(Self::Issue {
                sku: req_str(args, "sku")?,
                qty: req_i64(args, "qty")?,
                reference: opt_str(args, "reference", ""),
            }),
            "stock_transfer" => Ok(Self::Transfer {
                sku: req_str(args, "sku")?,
                to_location: req_str(args, "to_location")?,
                reference: opt_str(args, "reference", ""),
            }),
            "stock_storno" => Ok(Self::Storno {
                movement_id: req_i64(args, "movement_id")?,
                reason: opt_str(args, "reason", ""),
            }),
            "stock_movements" => Ok(Self::Movements {
                limit: args.get("limit").and_then(Value::as_i64).unwrap_or(25),
            }),
            "stock_low_alerts" => Ok(Self::LowAlerts),
            "stock_valuation" => Ok(Self::Valuation),
            "stock_inventur_start" => Ok(Self::InventurStart {
                note: opt_str(args, "note", "Inventur"),
            }),
            "stock_inventur_count" => Ok(Self::InventurCount {
                stocktake_id: req_i64(args, "stocktake_id")?,
                sku: req_str(args, "sku")?,
                counted_qty: req_i64(args, "counted_qty")?,
            }),
            "stock_inventur_commit" => Ok(Self::InventurCommit {
                stocktake_id: req_i64(args, "stocktake_id")?,
            }),
            "stock_pick_create" => {
                let raw_lines = args
                    .get("lines")
                    .and_then(Value::as_array)
                    .ok_or_else(|| StockError::Invalid("missing array field 'lines'".into()))?;
                let mut lines = Vec::with_capacity(raw_lines.len());
                for row in raw_lines {
                    lines.push(PickLineInput {
                        sku: req_str(row, "sku")?,
                        qty: req_i64(row, "qty")?,
                    });
                }
                Ok(Self::PickCreate {
                    source: opt_str(args, "source", "manual"),
                    reference: req_str(args, "reference")?,
                    lines,
                })
            }
            "stock_pick_list" => Ok(Self::PickList),
            "stock_pick_commit" => Ok(Self::PickCommit {
                pick_list_id: req_i64(args, "pick_list_id")?,
            }),
            "stock_export_csv" => Ok(Self::ExportCsv),
            "stock_import_csv" => Ok(Self::ImportCsv {
                csv: req_str(args, "csv")?,
            }),
            "stock_inbox_drain" => Ok(Self::InboxDrain),
            "stock_lot_receive" => Ok(Self::LotReceive {
                sku: req_str(args, "sku")?,
                qty: req_i64(args, "qty")?,
                unit_cost_cents: req_i64(args, "unit_cost_cents")?,
                lot_no: req_str(args, "lot_no")?,
                expires_at: opt_str(args, "expires_at", ""),
                reference: opt_str(args, "reference", ""),
            }),
            "stock_lots" => Ok(Self::Lots {
                sku: args
                    .get("sku")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string),
            }),
            "stock_expiring_lots" => Ok(Self::ExpiringLots {
                within_days: args
                    .get("within_days")
                    .and_then(Value::as_i64)
                    .unwrap_or(30),
            }),
            "stock_writedown" => Ok(Self::Writedown {
                sku: req_str(args, "sku")?,
                new_cost_cents: req_i64(args, "new_cost_cents")?,
                reason: opt_str(args, "reason", "Niederstwertprinzip"),
            }),
            "stock_reorder_plan" => Ok(Self::ReorderPlan {
                emit_bus: args
                    .get("emit_bus")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
            "stock_scan" => Ok(Self::Scan {
                code: req_str(args, "code")?,
                action: opt_str(args, "action", "lookup"),
                qty: args.get("qty").and_then(Value::as_i64).unwrap_or(1),
                unit_cost_cents: args.get("unit_cost_cents").and_then(Value::as_i64),
                reference: opt_str(args, "reference", "SCAN"),
            }),
            other => Err(StockError::Invalid(format!("unknown tool '{other}'"))),
        }
    }
}

pub fn dispatch(ctx: &Ctx<'_>, cmd: Command) -> Result<Value> {
    match cmd {
        Command::Status => {
            let items = ctx.store.list_items()?;
            let low = ctx.store.low_alerts()?;
            let val_cents = ctx.store.total_valuation_cents()?;
            Ok(json!({
                "app": APP,
                "version": VERSION,
                "items_count": items.len(),
                "low_stock_count": low.len(),
                "total_valuation_cents": val_cents,
                "active_inventur_id": ctx.store.active_inventur_id()?,
            }))
        }
        Command::SeedDemo => {
            let count = ctx.store.seed_demo()?;
            Ok(json!({ "seeded_items": count }))
        }
        Command::ListItems => Ok(json!({ "items": ctx.store.list_items()? })),
        Command::ItemUpsert {
            sku,
            name,
            ean,
            unit,
            location,
            min_stock,
            ekr_account,
        } => {
            let item = ctx.store.upsert_item(
                &sku,
                &name,
                &ean,
                &unit,
                &location,
                min_stock,
                &ekr_account,
            )?;
            Ok(json!({ "item": item }))
        }
        Command::Receive {
            sku,
            qty,
            unit_cost_cents,
            reference,
        } => {
            let mv = ctx.store.receive(&sku, qty, unit_cost_cents, &reference)?;
            Ok(json!({ "movement": mv }))
        }
        Command::Issue {
            sku,
            qty,
            reference,
        } => {
            let mv = ctx.store.issue(&sku, qty, &reference)?;
            let item = ctx.store.get_item(&sku)?;
            if let Some(bus) = ctx.bus {
                if item.min_stock > 0 && item.qty_on_hand <= item.min_stock {
                    let _ = bus.emit(
                        APP,
                        "taskcraft",
                        "stock.low",
                        json!({ "sku": item.sku, "qty": item.qty_on_hand, "min_stock": item.min_stock }),
                    );
                }
                let _ = bus.emit(
                    APP,
                    "buchcraft",
                    "stock.cogs_booked",
                    json!({ "sku": item.sku, "qty": qty, "cents": qty * mv.unit_cost_cents, "ekr": "5000" }),
                );
            }
            Ok(json!({ "movement": mv, "item": item }))
        }
        Command::Transfer {
            sku,
            to_location,
            reference,
        } => {
            let mv = ctx.store.transfer(&sku, &to_location, &reference)?;
            Ok(json!({ "movement": mv }))
        }
        Command::Storno {
            movement_id,
            reason,
        } => {
            let mv = ctx.store.storno(movement_id, &reason)?;
            Ok(json!({ "movement": mv }))
        }
        Command::Movements { limit } => {
            Ok(json!({ "movements": ctx.store.list_movements(limit)? }))
        }
        Command::LowAlerts => Ok(json!({ "low_stock": ctx.store.low_alerts()? })),
        Command::Valuation => Ok(json!({
            "total_valuation_cents": ctx.store.total_valuation_cents()?,
            "items": ctx.store.list_items()?,
        })),
        Command::InventurStart { note } => {
            let id = ctx.store.inventur_start(&note)?;
            Ok(json!({ "stocktake_id": id }))
        }
        Command::InventurCount {
            stocktake_id,
            sku,
            counted_qty,
        } => {
            ctx.store.inventur_count(stocktake_id, &sku, counted_qty)?;
            Ok(
                json!({ "ok": true, "stocktake_id": stocktake_id, "sku": sku, "counted_qty": counted_qty }),
            )
        }
        Command::InventurCommit { stocktake_id } => {
            let (adjusted, variance_cents) = ctx.store.inventur_commit(stocktake_id)?;
            if let Some(bus) = ctx.bus {
                let _ = bus.emit(
                    APP,
                    "buchcraft",
                    "stock.inventory_adjusted",
                    json!({ "stocktake_id": stocktake_id, "adjusted_skus": adjusted, "variance_cents": variance_cents }),
                );
            }
            Ok(
                json!({ "stocktake_id": stocktake_id, "adjusted_skus": adjusted, "variance_cents": variance_cents }),
            )
        }
        Command::PickCreate {
            source,
            reference,
            lines,
        } => {
            let pl = ctx.store.create_pick_list(&source, &reference, &lines)?;
            Ok(json!({ "pick_list": pl, "lines": ctx.store.get_pick_lines(pl.id)? }))
        }
        Command::PickList => Ok(json!({ "pick_lists": ctx.store.list_pick_lists()? })),
        Command::PickCommit { pick_list_id } => {
            let pl = ctx.store.commit_pick_list(pick_list_id)?;
            let lines = ctx.store.get_pick_lines(pick_list_id)?;
            if let Some(bus) = ctx.bus {
                for line in &lines {
                    if let Ok(item) = ctx.store.get_item(&line.sku) {
                        if item.min_stock > 0 && item.qty_on_hand <= item.min_stock {
                            let _ = bus.emit(
                                APP,
                                "taskcraft",
                                "stock.low",
                                json!({ "sku": item.sku, "qty": item.qty_on_hand, "min_stock": item.min_stock }),
                            );
                        }
                        let _ = bus.emit(
                            APP,
                            "buchcraft",
                            "stock.cogs_booked",
                            json!({ "sku": item.sku, "qty": line.qty, "cents": line.qty * item.unit_cost_cents, "ekr": "5000" }),
                        );
                    }
                }
            }
            Ok(json!({ "pick_list": pl, "lines": lines }))
        }
        Command::ExportCsv => Ok(json!({ "csv": ctx.store.export_csv()? })),
        Command::ImportCsv { csv } => {
            let imported = ctx.store.import_csv(&csv)?;
            Ok(json!({ "imported_items": imported }))
        }
        Command::InboxDrain => inbox_drain(ctx),
        Command::LotReceive {
            sku,
            qty,
            unit_cost_cents,
            lot_no,
            expires_at,
            reference,
        } => {
            let (mv, lot) = ctx.store.receive_lot(
                &sku,
                qty,
                unit_cost_cents,
                &lot_no,
                &expires_at,
                &reference,
            )?;
            Ok(json!({ "movement": mv, "lot": lot }))
        }
        Command::Lots { sku } => Ok(json!({ "lots": ctx.store.list_lots(sku.as_deref())? })),
        Command::ExpiringLots { within_days } => Ok(
            json!({ "expiring_lots": ctx.store.expiring_lots(within_days)?, "within_days": within_days }),
        ),
        Command::Writedown {
            sku,
            new_cost_cents,
            reason,
        } => {
            let item_before = ctx.store.get_item(&sku)?;
            let mv = ctx.store.writedown(&sku, new_cost_cents, &reason)?;
            let variance_cents =
                item_before.qty_on_hand * (new_cost_cents - item_before.unit_cost_cents);
            if let Some(bus) = ctx.bus {
                let _ = bus.emit(
                    APP,
                    "buchcraft",
                    "stock.inventory_adjusted",
                    json!({
                        "sku": sku,
                        "kind": "writedown",
                        "variance_cents": variance_cents,
                        "reason": reason,
                    }),
                );
            }
            Ok(json!({ "movement": mv, "variance_cents": variance_cents }))
        }
        Command::ReorderPlan { emit_bus } => {
            let plan = ctx.store.reorder_plan()?;
            if emit_bus {
                if let Some(bus) = ctx.bus {
                    for row in &plan {
                        let _ = bus.emit(
                            APP,
                            "taskcraft",
                            "stock.reorder_requested",
                            json!({
                                "sku": row.sku,
                                "suggested_qty": row.suggested_qty,
                                "estimated_cost_cents": row.estimated_cost_cents,
                            }),
                        );
                    }
                }
            }
            let total_estimated_cents: i64 = plan.iter().map(|r| r.estimated_cost_cents).sum();
            Ok(json!({
                "reorder_items": plan,
                "total_estimated_cents": total_estimated_cents,
            }))
        }
        Command::Scan {
            code,
            action,
            qty,
            unit_cost_cents,
            reference,
        } => {
            let item = ctx.store.find_by_code(&code)?;
            match action.trim().to_ascii_lowercase().as_str() {
                "lookup" => Ok(json!({ "item": item })),
                "receive" => {
                    let cost = unit_cost_cents.unwrap_or(item.unit_cost_cents);
                    let mv = ctx.store.receive(&item.sku, qty, cost, &reference)?;
                    let updated = ctx.store.get_item(&item.sku)?;
                    Ok(json!({ "movement": mv, "item": updated }))
                }
                "issue" => dispatch(
                    ctx,
                    Command::Issue {
                        sku: item.sku,
                        qty,
                        reference,
                    },
                ),
                other => Err(StockError::Invalid(format!(
                    "scan action '{other}' must be lookup, receive, or issue"
                ))),
            }
        }
    }
}

fn parse_bus_lines(payload: &Value) -> std::result::Result<Vec<PickLineInput>, String> {
    let arr = payload
        .get("lines")
        .or_else(|| payload.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| "event payload missing 'lines' or 'items' array".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for row in arr {
        let sku = row
            .get("sku")
            .and_then(Value::as_str)
            .ok_or_else(|| "line missing 'sku'".to_string())?;
        let qty = row
            .get("qty")
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("line for {sku} missing integer 'qty'"))?;
        if qty <= 0 {
            return Err(format!("line for {sku} must have qty > 0"));
        }
        out.push(PickLineInput {
            sku: sku.to_string(),
            qty,
        });
    }
    if out.is_empty() {
        return Err("event has zero pick lines".into());
    }
    Ok(out)
}

pub fn inbox_drain(ctx: &Ctx<'_>) -> Result<Value> {
    let Some(bus) = ctx.bus else {
        return Ok(json!({ "drained": 0, "rejected": 0, "pick_lists": [] }));
    };
    let pending = bus
        .pending(APP)
        .map_err(|e| StockError::Invalid(format!("bus pending: {e}")))?;
    let mut drained = 0usize;
    let mut rejected = 0usize;
    let mut created = Vec::new();
    for ev in pending {
        match ev.kind.as_str() {
            "invoice.issued" | "bom.exported" => match parse_bus_lines(&ev.payload) {
                Ok(lines) => {
                    let reference = ev
                        .payload
                        .get("reference")
                        .or_else(|| ev.payload.get("invoice_no"))
                        .or_else(|| ev.payload.get("bom_id"))
                        .and_then(Value::as_str)
                        .unwrap_or(&ev.id);
                    match ctx.store.create_pick_list(&ev.source, reference, &lines) {
                        Ok(pl) => {
                            let _ = bus.ack(APP, &ev.id);
                            drained += 1;
                            created.push(pl);
                        }
                        Err(e) => {
                            let _ = bus.reject(APP, &ev.id, &e.to_string());
                            rejected += 1;
                        }
                    }
                }
                Err(reason) => {
                    let _ = bus.reject(APP, &ev.id, &reason);
                    rejected += 1;
                }
            },
            other => {
                let _ = bus.reject(APP, &ev.id, &format!("unsupported event kind '{other}'"));
                rejected += 1;
            }
        }
    }
    Ok(json!({
        "drained": drained,
        "rejected": rejected,
        "pick_lists": created,
    }))
}

pub fn tools() -> Vec<Value> {
    vec![
        tool(
            "stock_status",
            "Inventory counts, valuation in cents, and low-stock count.",
            json!({}),
            &[],
        ),
        tool(
            "stock_seed",
            "Seed demo inventory items if database is empty.",
            json!({}),
            &[],
        ),
        tool(
            "stock_list",
            "List all SKUs with quantities, bins, and moving-average cost.",
            json!({}),
            &[],
        ),
        tool(
            "stock_item_upsert",
            "Create or update an inventory SKU.",
            json!({
                "sku": { "type": "string" },
                "name": { "type": "string" },
                "ean": { "type": "string" },
                "unit": { "type": "string" },
                "location": { "type": "string" },
                "min_stock": { "type": "integer" },
                "ekr_account": { "type": "string" }
            }),
            &["sku", "name"],
        ),
        tool(
            "stock_receive",
            "Wareneingang: receive stock and update moving-average unit cost.",
            json!({
                "sku": { "type": "string" },
                "qty": { "type": "integer" },
                "unit_cost_cents": { "type": "integer" },
                "reference": { "type": "string" }
            }),
            &["sku", "qty", "unit_cost_cents"],
        ),
        tool(
            "stock_issue",
            "Warenausgang: issue stock at current moving-average cost and emit bus events.",
            json!({
                "sku": { "type": "string" },
                "qty": { "type": "integer" },
                "reference": { "type": "string" }
            }),
            &["sku", "qty"],
        ),
        tool(
            "stock_transfer",
            "Umlagerung: move an item to a new bin location.",
            json!({
                "sku": { "type": "string" },
                "to_location": { "type": "string" },
                "reference": { "type": "string" }
            }),
            &["sku", "to_location"],
        ),
        tool(
            "stock_storno",
            "Reverse a movement with a linked Storno entry.",
            json!({
                "movement_id": { "type": "integer" },
                "reason": { "type": "string" }
            }),
            &["movement_id"],
        ),
        tool(
            "stock_movements",
            "List recent immutable stock movements.",
            json!({ "limit": { "type": "integer" } }),
            &[],
        ),
        tool(
            "stock_low_alerts",
            "List items at or below Meldebestand (min_stock).",
            json!({}),
            &[],
        ),
        tool(
            "stock_valuation",
            "Total stock valuation in integer cents.",
            json!({}),
            &[],
        ),
        tool(
            "stock_inventur_start",
            "Start a new Inventur (stocktake) session.",
            json!({ "note": { "type": "string" } }),
            &[],
        ),
        tool(
            "stock_inventur_count",
            "Record counted quantity for a SKU in an active stocktake.",
            json!({
                "stocktake_id": { "type": "integer" },
                "sku": { "type": "string" },
                "counted_qty": { "type": "integer" }
            }),
            &["stocktake_id", "sku", "counted_qty"],
        ),
        tool(
            "stock_inventur_commit",
            "Commit an Inventur session and post variance movements.",
            json!({ "stocktake_id": { "type": "integer" } }),
            &["stocktake_id"],
        ),
        tool(
            "stock_pick_create",
            "Create a pick list (Kommissionierung) for one or more SKUs.",
            json!({
                "source": { "type": "string" },
                "reference": { "type": "string" },
                "lines": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "sku": { "type": "string" },
                            "qty": { "type": "integer" }
                        },
                        "required": ["sku", "qty"]
                    }
                }
            }),
            &["reference", "lines"],
        ),
        tool(
            "stock_pick_list",
            "List open and committed pick lists.",
            json!({}),
            &[],
        ),
        tool(
            "stock_pick_commit",
            "Commit a pick list and issue all picked SKUs from stock.",
            json!({ "pick_list_id": { "type": "integer" } }),
            &["pick_list_id"],
        ),
        tool(
            "stock_export_csv",
            "Export current stock levels and valuation as CSV.",
            json!({}),
            &[],
        ),
        tool(
            "stock_import_csv",
            "Import SKUs (and optional initial stock) from CSV text.",
            json!({ "csv": { "type": "string" } }),
            &["csv"],
        ),
        tool(
            "stock_inbox_drain",
            "Drain pending craft-biz bus events (invoice.issued, bom.exported) into pick lists.",
            json!({}),
            &[],
        ),
        tool(
            "stock_lot_receive",
            "Receive stock into a tracked batch/lot with optional MHD expiry date (YYYY-MM-DD).",
            json!({
                "sku": { "type": "string" },
                "qty": { "type": "integer" },
                "unit_cost_cents": { "type": "integer" },
                "lot_no": { "type": "string" },
                "expires_at": { "type": "string" },
                "reference": { "type": "string" }
            }),
            &["sku", "qty", "unit_cost_cents", "lot_no"],
        ),
        tool(
            "stock_lots",
            "List tracked batches/lots in FEFO order (optional SKU filter).",
            json!({ "sku": { "type": "string" } }),
            &[],
        ),
        tool(
            "stock_expiring_lots",
            "List active lots expiring within N days (MHD alert).",
            json!({ "within_days": { "type": "integer" } }),
            &[],
        ),
        tool(
            "stock_writedown",
            "Niederstwertprinzip: write down unit cost to a lower market value and emit adjustment.",
            json!({
                "sku": { "type": "string" },
                "new_cost_cents": { "type": "integer" },
                "reason": { "type": "string" }
            }),
            &["sku", "new_cost_cents"],
        ),
        tool(
            "stock_reorder_plan",
            "Generate reorder suggestions for all SKUs at or below Meldebestand.",
            json!({ "emit_bus": { "type": "boolean" } }),
            &[],
        ),
        tool(
            "stock_scan",
            "USB-HID barcode/EAN or SKU scan: lookup, receive, or issue by code.",
            json!({
                "code": { "type": "string" },
                "action": { "type": "string", "enum": ["lookup", "receive", "issue"] },
                "qty": { "type": "integer" },
                "unit_cost_cents": { "type": "integer" },
                "reference": { "type": "string" }
            }),
            &["code"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_average_and_issue_valuation() -> Result<()> {
        let store = Store::open_in_memory()?;
        store.upsert_item("SKU-1", "Bolt M6", "", "pcs", "A-01", 5, "1600")?;
        store.receive("SKU-1", 10, 100, "PO-1")?;
        store.receive("SKU-1", 10, 200, "PO-2")?;
        let item = store.get_item("SKU-1")?;
        assert_eq!(item.qty_on_hand, 20);
        assert_eq!(item.unit_cost_cents, 150);

        let mv = store.issue("SKU-1", 5, "ORD-1")?;
        assert_eq!(mv.qty_delta, -5);
        assert_eq!(mv.unit_cost_cents, 150);
        assert_eq!(store.total_valuation_cents()?, 15 * 150);
        Ok(())
    }

    #[test]
    fn storno_reverses_movement_and_blocks_double_storno() -> Result<()> {
        let store = Store::open_in_memory()?;
        store.upsert_item("SKU-2", "Cable", "", "pcs", "B-01", 2, "1600")?;
        let mv = store.receive("SKU-2", 10, 500, "PO-1")?;
        let st = store.storno(mv.id, "wrong qty")?;
        assert_eq!(st.qty_delta, -10);
        assert_eq!(store.get_item("SKU-2")?.qty_on_hand, 0);
        assert!(store.storno(mv.id, "again").is_err());
        Ok(())
    }

    #[test]
    fn insufficient_stock_is_rejected() -> Result<()> {
        let store = Store::open_in_memory()?;
        store.upsert_item("SKU-3", "Switch", "", "pcs", "C-01", 1, "1600")?;
        store.receive("SKU-3", 2, 1000, "PO-1")?;
        assert!(matches!(
            store.issue("SKU-3", 3, "ORD-1"),
            Err(StockError::InsufficientStock {
                have: 2,
                need: 3,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn low_alerts_and_inventur_commit() -> Result<()> {
        let store = Store::open_in_memory()?;
        store.upsert_item("SKU-4", "Sensor", "", "pcs", "D-01", 10, "1600")?;
        store.receive("SKU-4", 8, 2500, "PO-1")?;
        assert_eq!(store.low_alerts()?.len(), 1);

        let st_id = store.inventur_start("Year-end")?;
        store.inventur_count(st_id, "SKU-4", 6)?;
        let (adj, var_cents) = store.inventur_commit(st_id)?;
        assert_eq!(adj, 1);
        assert_eq!(var_cents, -5000);
        assert_eq!(store.get_item("SKU-4")?.qty_on_hand, 6);
        Ok(())
    }

    #[test]
    fn command_dispatch_roundtrip() -> Result<()> {
        let store = Store::open_in_memory()?;
        let ctx = Ctx {
            store: &store,
            bus: None,
        };
        let seeded = dispatch(&ctx, Command::SeedDemo)?;
        assert_eq!(seeded["seeded_items"], 4);
        let status = dispatch(&ctx, Command::Status)?;
        assert_eq!(status["items_count"], 4);
        assert_eq!(status["low_stock_count"], 2);
        Ok(())
    }

    #[test]
    fn pick_list_commit_and_csv_export() -> Result<()> {
        let store = Store::open_in_memory()?;
        store.upsert_item("SKU-P1", "Relay 24V", "", "pcs", "E-01", 2, "1600")?;
        store.receive("SKU-P1", 10, 400, "PO-10")?;
        let pl = store.create_pick_list(
            "manual",
            "ORD-99",
            &[PickLineInput {
                sku: "SKU-P1".into(),
                qty: 3,
            }],
        )?;
        assert_eq!(pl.status, "open");
        let committed = store.commit_pick_list(pl.id)?;
        assert_eq!(committed.status, "committed");
        assert_eq!(store.get_item("SKU-P1")?.qty_on_hand, 7);
        let csv = store.export_csv()?;
        assert!(csv.contains("SKU-P1,\"Relay 24V\""));
        Ok(())
    }

    #[test]
    fn inbox_drain_consumes_bus_events() -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "stockcraft-bus-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let bus = Bus::new(&dir);
        let _ = bus.emit(
            "billcraft",
            "stockcraft",
            "invoice.issued",
            json!({
                "invoice_no": "INV-2026-01",
                "lines": [{ "sku": "SKU-B1", "qty": 2 }]
            }),
        );
        let store = Store::open_in_memory()?;
        let ctx = Ctx {
            store: &store,
            bus: Some(&bus),
        };
        let out = dispatch(&ctx, Command::InboxDrain)?;
        assert_eq!(out["drained"], 1);
        assert_eq!(out["rejected"], 0);
        let lists = store.list_pick_lists()?;
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].reference, "INV-2026-01");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn lots_writedown_reorder_and_csv_import() -> Result<()> {
        let store = Store::open_in_memory()?;
        let csv = "sku,name,ean,unit,location,qty_on_hand,min_stock,unit_cost_cents,ekr_account\n\
                   PASTE-01,\"Solder Paste\",4001,jar,C-02,3,5,2000,1600\n";
        assert_eq!(store.import_csv(csv)?, 1);

        let (_mv, lot) =
            store.receive_lot("PASTE-01", 2, 2000, "LOT-2026-A", "2026-11-20", "PO-LOT")?;
        assert_eq!(lot.lot_no, "LOT-2026-A");
        assert_eq!(store.list_lots(Some("PASTE-01"))?.len(), 1);
        assert_eq!(store.expiring_lots(30)?.len(), 1);

        // Niederstwertprinzip: write-up rejected, write-down accepted
        assert!(store.writedown("PASTE-01", 2500, "invalid up").is_err());
        let wd = store.writedown("PASTE-01", 1500, "shelf-life discount")?;
        assert_eq!(wd.kind, "writedown");
        assert_eq!(store.get_item("PASTE-01")?.unit_cost_cents, 1500);

        let plan = store.reorder_plan()?;
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].suggested_qty, 5);

        let found = store.find_by_code("4001")?;
        assert_eq!(found.sku, "PASTE-01");
        let ctx = Ctx {
            store: &store,
            bus: None,
        };
        let scanned = dispatch(
            &ctx,
            Command::Scan {
                code: "4001".into(),
                action: "issue".into(),
                qty: 1,
                unit_cost_cents: None,
                reference: "USB-SCAN".into(),
            },
        )?;
        assert_eq!(scanned["item"]["qty_on_hand"], 4);
        Ok(())
    }
}
