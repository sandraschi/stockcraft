//! `stockcraft`: CLI and MCP stdio server in one binary (L2).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::{Parser, Subcommand};
use craft_biz::bus::Bus;
use craft_biz::mcp::{serve_stdio, tools_markdown, ToolHost};
use serde_json::Value;
use std::path::PathBuf;
use std::process::ExitCode;
use stockcraft_core::{dispatch, tools, Command, Ctx, Store, APP, VERSION};

#[derive(Parser)]
#[command(
    name = "stockcraft",
    version,
    about = "Local-first inventory & light warehouse management (Lagerhaltung). CLI + MCP."
)]
struct Cli {
    /// Inventory database. Default: $STOCKCRAFT_DB or %LOCALAPPDATA%\stockcraft\stockcraft.db
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    /// Print machine-readable JSON only.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Inventory status, total valuation, and low-stock count.
    Status,
    /// Seed demo items and movements if empty.
    Seed,
    /// List all items.
    List,
    /// Create or update an item SKU.
    ItemUpsert {
        sku: String,
        name: String,
        #[arg(long, default_value = "")]
        ean: String,
        #[arg(long, default_value = "pcs")]
        unit: String,
        #[arg(long, default_value = "A-01-01")]
        location: String,
        #[arg(long, default_value_t = 0)]
        min_stock: i64,
        #[arg(long, default_value = "1600")]
        ekr: String,
    },
    /// Wareneingang: receive stock at unit_cost_cents.
    Receive {
        sku: String,
        qty: i64,
        #[arg(long)]
        cost_cents: i64,
        #[arg(long, default_value = "")]
        reference: String,
    },
    /// Warenausgang: issue stock at moving-average cost.
    Issue {
        sku: String,
        qty: i64,
        #[arg(long, default_value = "")]
        reference: String,
    },
    /// Umlagerung: move item to another bin location.
    Transfer {
        sku: String,
        to_location: String,
        #[arg(long, default_value = "")]
        reference: String,
    },
    /// Reverse a movement with an immutable Storno entry.
    Storno {
        movement_id: i64,
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Recent inventory movements.
    Movements {
        #[arg(long, default_value_t = 25)]
        limit: i64,
    },
    /// Items at or below Meldebestand.
    LowAlerts,
    /// Total inventory valuation.
    Valuation,
    /// Start an Inventur (stocktake) session.
    InventurStart {
        #[arg(long, default_value = "Inventur")]
        note: String,
    },
    /// Record counted quantity for a SKU in a stocktake.
    InventurCount {
        stocktake_id: i64,
        sku: String,
        counted_qty: i64,
    },
    /// Commit a stocktake and post InventurDiff movements.
    InventurCommit { stocktake_id: i64 },
    /// List open and committed pick lists.
    PickList,
    /// Commit a pick list and issue all picked SKUs.
    PickCommit { pick_list_id: i64 },
    /// Export stock levels and valuation as CSV.
    ExportCsv,
    /// Import SKUs (and optional initial stock) from a CSV file.
    ImportCsv { path: PathBuf },
    /// Drain pending craft-biz bus events into pick lists.
    InboxDrain,
    /// Receive stock into a tracked batch/lot with optional MHD expiry date.
    LotReceive {
        sku: String,
        qty: i64,
        #[arg(long)]
        cost_cents: i64,
        #[arg(long)]
        lot: String,
        #[arg(long, default_value = "")]
        expires: String,
        #[arg(long, default_value = "")]
        reference: String,
    },
    /// List tracked batches/lots in FEFO order.
    Lots {
        #[arg(long)]
        sku: Option<String>,
    },
    /// List active lots expiring within N days (MHD alert).
    ExpiringLots {
        #[arg(long, default_value_t = 30)]
        within_days: i64,
    },
    /// Niederstwertprinzip: write down unit cost to a lower market value.
    Writedown {
        sku: String,
        new_cost_cents: i64,
        #[arg(long, default_value = "Niederstwertprinzip")]
        reason: String,
    },
    /// Generate reorder suggestions for SKUs at or below Meldebestand.
    ReorderPlan {
        #[arg(long)]
        emit_bus: bool,
    },
    /// USB-HID barcode/EAN or SKU scan: lookup, receive, or issue by code.
    Scan {
        code: String,
        #[arg(long, default_value = "lookup")]
        action: String,
        #[arg(long, default_value_t = 1)]
        qty: i64,
        #[arg(long)]
        cost_cents: Option<i64>,
        #[arg(long, default_value = "SCAN")]
        reference: String,
    },
    /// Print the MCP tool catalog (JSON, or markdown for docs/TOOLS.md).
    Tools {
        #[arg(long)]
        markdown: bool,
    },
    /// Run MCP 2024-11-05 JSON-RPC server over stdio.
    Mcp,
}

fn default_db() -> PathBuf {
    if let Ok(p) = std::env::var("STOCKCRAFT_DB") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("stockcraft").join("stockcraft.db")
}

struct StockHost<'a> {
    ctx: Ctx<'a>,
}

impl<'a> ToolHost for StockHost<'a> {
    fn server_name(&self) -> &str {
        APP
    }

    fn server_version(&self) -> &str {
        VERSION
    }

    fn tools(&self) -> Vec<Value> {
        tools()
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let cmd = Command::from_tool(name, args).map_err(|e| e.to_string())?;
        dispatch(&self.ctx, cmd).map_err(|e| e.to_string())
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Cmd::Tools { markdown } = cli.cmd {
        if markdown {
            print!("{}", tools_markdown(APP, &tools()));
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&tools()).unwrap_or_else(|_| "[]".into())
            );
        }
        return ExitCode::SUCCESS;
    }

    let db = cli.db.unwrap_or_else(default_db);
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("stockcraft: failed to open {}: {e}", db.display());
            return ExitCode::FAILURE;
        }
    };
    let bus = Bus::new(Bus::default_root());
    let ctx = Ctx {
        store: &store,
        bus: Some(&bus),
    };

    if matches!(cli.cmd, Cmd::Mcp) {
        let mut host = StockHost { ctx };
        if let Err(e) = serve_stdio(&mut host) {
            eprintln!("stockcraft mcp error: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    let cmd = match cli.cmd {
        Cmd::Status => Command::Status,
        Cmd::Seed => Command::SeedDemo,
        Cmd::List => Command::ListItems,
        Cmd::ItemUpsert {
            sku,
            name,
            ean,
            unit,
            location,
            min_stock,
            ekr,
        } => Command::ItemUpsert {
            sku,
            name,
            ean,
            unit,
            location,
            min_stock,
            ekr_account: ekr,
        },
        Cmd::Receive {
            sku,
            qty,
            cost_cents,
            reference,
        } => Command::Receive {
            sku,
            qty,
            unit_cost_cents: cost_cents,
            reference,
        },
        Cmd::Issue {
            sku,
            qty,
            reference,
        } => Command::Issue {
            sku,
            qty,
            reference,
        },
        Cmd::Transfer {
            sku,
            to_location,
            reference,
        } => Command::Transfer {
            sku,
            to_location,
            reference,
        },
        Cmd::Storno {
            movement_id,
            reason,
        } => Command::Storno {
            movement_id,
            reason,
        },
        Cmd::Movements { limit } => Command::Movements { limit },
        Cmd::LowAlerts => Command::LowAlerts,
        Cmd::Valuation => Command::Valuation,
        Cmd::InventurStart { note } => Command::InventurStart { note },
        Cmd::InventurCount {
            stocktake_id,
            sku,
            counted_qty,
        } => Command::InventurCount {
            stocktake_id,
            sku,
            counted_qty,
        },
        Cmd::InventurCommit { stocktake_id } => Command::InventurCommit { stocktake_id },
        Cmd::PickList => Command::PickList,
        Cmd::PickCommit { pick_list_id } => Command::PickCommit { pick_list_id },
        Cmd::ExportCsv => Command::ExportCsv,
        Cmd::ImportCsv { path } => {
            let csv = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("stockcraft: failed to read {}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
            };
            Command::ImportCsv { csv }
        }
        Cmd::InboxDrain => Command::InboxDrain,
        Cmd::LotReceive {
            sku,
            qty,
            cost_cents,
            lot,
            expires,
            reference,
        } => Command::LotReceive {
            sku,
            qty,
            unit_cost_cents: cost_cents,
            lot_no: lot,
            expires_at: expires,
            reference,
        },
        Cmd::Lots { sku } => Command::Lots { sku },
        Cmd::ExpiringLots { within_days } => Command::ExpiringLots { within_days },
        Cmd::Writedown {
            sku,
            new_cost_cents,
            reason,
        } => Command::Writedown {
            sku,
            new_cost_cents,
            reason,
        },
        Cmd::ReorderPlan { emit_bus } => Command::ReorderPlan { emit_bus },
        Cmd::Scan {
            code,
            action,
            qty,
            cost_cents,
            reference,
        } => Command::Scan {
            code,
            action,
            qty,
            unit_cost_cents: cost_cents,
            reference,
        },
        Cmd::Tools { .. } | Cmd::Mcp => unreachable!(),
    };

    match dispatch(&ctx, cmd) {
        Ok(val) => {
            if cli.json {
                println!("{val}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("stockcraft: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_biz::mcp::docs_in_sync;

    #[test]
    fn tools_doc_is_current() {
        let committed = include_str!("../../../docs/TOOLS.md");
        assert!(
            docs_in_sync(committed, &tools_markdown(APP, &tools())),
            "docs/TOOLS.md is stale: run `just docs`"
        );
    }
}
