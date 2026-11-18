//! `stockcraft-app` (L3): native `egui 0.37` desktop workstation over `craft-biz-ui`.
//! Mutations go through `stockcraft_core::dispatch` (same path as CLI and MCP).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_biz::bus::Bus;
use craft_biz::Money;
use craft_biz_ui::app::{main_with, AppMenus, Frame, NavEntry, SuiteApp, Wiring};
use craft_biz_ui::help::{AppInfo, Doc};
use craft_biz_ui::menu::{Item, Menu};
use craft_biz_ui::theme::Tone;
use craft_biz_ui::widgets::{self, Kpi};
use egui::{Grid, RichText, ScrollArea, Ui};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use stockcraft_core::{
    dispatch, tools, Command, Ctx, InventurLine, Item as StockItem, Lot, Movement, PickLine,
    PickList, Store, APP, VERSION,
};

struct StockApp {
    store: Store,
    bus: Bus,
    db_path: PathBuf,
    items: Vec<StockItem>,
    movements: Vec<Movement>,
    low: Vec<StockItem>,
    lots: Vec<Lot>,
    expiring_lots: Vec<Lot>,
    pick_lists: Vec<PickList>,
    selected_pick_id: Option<i64>,
    pick_lines: Vec<PickLine>,
    active_inventur_id: Option<i64>,
    inventur_lines: Vec<InventurLine>,
    inventur_inputs: HashMap<String, String>,
    total_valuation_cents: i64,
    selected_sku: Option<String>,
    scan_code: String,
    scan_action: String,
}

impl StockApp {
    fn open(path: &Path) -> Result<Self, String> {
        let store = Store::open(path).map_err(|e| e.to_string())?;
        let bus = Bus::new(Bus::default_root());
        let mut app = Self {
            store,
            bus,
            db_path: path.to_path_buf(),
            items: Vec::new(),
            movements: Vec::new(),
            low: Vec::new(),
            lots: Vec::new(),
            expiring_lots: Vec::new(),
            pick_lists: Vec::new(),
            selected_pick_id: None,
            pick_lines: Vec::new(),
            active_inventur_id: None,
            inventur_lines: Vec::new(),
            inventur_inputs: HashMap::new(),
            total_valuation_cents: 0,
            selected_sku: None,
            scan_code: String::new(),
            scan_action: "lookup".into(),
        };
        app.reload()?;
        Ok(app)
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            store: &self.store,
            bus: Some(&self.bus),
        }
    }
}

impl SuiteApp for StockApp {
    fn name(&self) -> &'static str {
        "StockCraft"
    }

    fn id(&self) -> &'static str {
        APP
    }

    fn tone(&self) -> Tone {
        Tone::Teal
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn tools(&self) -> Vec<Value> {
        tools()
    }

    fn call(&mut self, tool: &str, args: &Value) -> Result<Value, String> {
        let cmd = Command::from_tool(tool, args).map_err(|e| e.to_string())?;
        dispatch(&self.ctx(), cmd).map_err(|e| e.to_string())
    }

    fn reload(&mut self) -> Result<(), String> {
        self.items = self.store.list_items().map_err(|e| e.to_string())?;
        self.movements = self.store.list_movements(60).map_err(|e| e.to_string())?;
        self.low = self.store.low_alerts().map_err(|e| e.to_string())?;
        self.lots = self.store.list_lots(None).map_err(|e| e.to_string())?;
        self.expiring_lots = self.store.expiring_lots(30).map_err(|e| e.to_string())?;
        self.pick_lists = self.store.list_pick_lists().map_err(|e| e.to_string())?;
        if self.selected_pick_id.is_none() {
            self.selected_pick_id = self.pick_lists.first().map(|p| p.id);
        }
        self.pick_lines = match self.selected_pick_id {
            Some(id) => self.store.get_pick_lines(id).unwrap_or_default(),
            None => Vec::new(),
        };
        self.total_valuation_cents = self
            .store
            .total_valuation_cents()
            .map_err(|e| e.to_string())?;
        self.active_inventur_id = self.store.active_inventur_id().map_err(|e| e.to_string())?;
        self.inventur_lines = match self.active_inventur_id {
            Some(id) => self.store.inventur_lines(id).unwrap_or_default(),
            None => Vec::new(),
        };
        for line in &self.inventur_lines {
            let val = line
                .counted_qty
                .map(|q| q.to_string())
                .unwrap_or_else(|| line.expected_qty.to_string());
            self.inventur_inputs.entry(line.sku.clone()).or_insert(val);
        }
        Ok(())
    }

    fn nav(&self, f: &Frame) -> Vec<NavEntry> {
        let open_picks = self
            .pick_lists
            .iter()
            .filter(|p| p.status == "open")
            .count();
        let mut entries = vec![
            NavEntry::new("S", f.t("Lagerbestand", "Inventory")).group(f.t("Lager", "Warehouse")),
            NavEntry::new("M", f.t("Bewegungen", "Movements")),
            NavEntry::new("P", f.t("Kommissionierung", "Pick lists")),
            NavEntry::new("I", "Inventur"),
            NavEntry::new("L", f.t("Chargen & MHD", "Lots & MHD")),
            NavEntry::new("!", f.t("Meldebestand", "Low stock")),
        ];
        if !self.items.is_empty() {
            entries[0] = NavEntry::new("S", f.t("Lagerbestand", "Inventory"))
                .group(f.t("Lager", "Warehouse"))
                .badge(self.items.len(), Tone::Teal);
        }
        if open_picks > 0 {
            entries[2] = NavEntry::new("P", f.t("Kommissionierung", "Pick lists"))
                .badge(open_picks, Tone::Info);
        }
        if !self.expiring_lots.is_empty() {
            entries[4] = NavEntry::new("L", f.t("Chargen & MHD", "Lots & MHD"))
                .badge(self.expiring_lots.len(), Tone::Warn);
        }
        if !self.low.is_empty() {
            entries[5] = NavEntry::new("!", f.t("Meldebestand", "Low stock"))
                .badge(self.low.len(), Tone::Warn);
        }
        entries
    }

    fn view(&mut self, ui: &mut Ui, view: usize, f: &mut Frame) {
        match view {
            0 => self.view_inventory(ui, f),
            1 => self.view_movements(ui, f),
            2 => self.view_pick_lists(ui, f),
            3 => self.view_inventur(ui, f),
            4 => self.view_lots(ui, f),
            _ => self.view_low_alerts(ui, f),
        }
    }

    fn top_actions(&mut self, ui: &mut Ui, f: &mut Frame) {
        if ui.button(f.t("+ Artikel", "+ New SKU")).clicked() {
            f.form("stock_item_upsert", &[]);
        }
        if ui.button(f.t("Wareneingang", "Receive")).clicked() {
            let pre = self
                .selected_sku
                .as_deref()
                .map(|s| vec![("sku", s)])
                .unwrap_or_default();
            f.form("stock_receive", &pre);
        }
        if ui.button(f.t("Warenausgang", "Issue")).clicked() {
            let pre = self
                .selected_sku
                .as_deref()
                .map(|s| vec![("sku", s)])
                .unwrap_or_default();
            f.form("stock_issue", &pre);
        }
    }

    fn bus(&self) -> &Bus {
        &self.bus
    }

    fn wiring(&self) -> Wiring {
        (
            vec![
                ("invoice.issued", "Create pick list from billcraft invoice"),
                ("bom.exported", "Create pick list from modcraft BOM"),
            ],
            vec![
                ("stock.low", "Alert taskcraft when SKU hits Meldebestand"),
                ("stock.cogs_booked", "Post COGS (EKR 5000) to buchcraft"),
                (
                    "stock.inventory_adjusted",
                    "Post Inventur & writedown variance to buchcraft",
                ),
                (
                    "stock.reorder_requested",
                    "Send reorder suggestion to taskcraft",
                ),
            ],
        )
    }

    fn drain_tool(&self) -> Option<&'static str> {
        Some("stock_inbox_drain")
    }

    fn prefill(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(sku) = &self.selected_sku {
            out.push(("sku".into(), sku.clone()));
        }
        if let Some(id) = self.active_inventur_id {
            out.push(("stocktake_id".into(), id.to_string()));
        }
        if let Some(id) = self.selected_pick_id {
            out.push(("pick_list_id".into(), id.to_string()));
        }
        out
    }

    fn status_info(&self) -> String {
        format!(
            "{}  |  {} SKUs  |  {}",
            self.db_path.display(),
            self.items.len(),
            Money::from_cents(self.total_valuation_cents).format_de()
        )
    }

    fn select(&mut self, key: &str) {
        self.selected_sku = Some(key.to_string());
    }

    fn smoke_tool(&self) -> &'static str {
        "stock_status"
    }

    fn menus(&self, f: &Frame) -> AppMenus {
        AppMenus {
            file_new: vec![
                Item::action("stock.new_sku", f.t("Neuer Artikel...", "New SKU...")),
                Item::action(
                    "stock.import_csv",
                    f.t("CSV importieren...", "Import CSV..."),
                ),
                Item::action("stock.seed", f.t("Demo-Daten laden", "Seed demo data")),
            ],
            export: vec![Item::action(
                "stock.export_csv",
                f.t("Bestandsliste (CSV)", "Inventory CSV"),
            )],
            middle: vec![Menu::new(
                f.t("Lager", "Stock"),
                vec![
                    Item::action("stock.receive", f.t("Wareneingang...", "Receive...")),
                    Item::action(
                        "stock.lot_receive",
                        f.t("Charge / MHD einbuchen...", "Receive lot / MHD..."),
                    ),
                    Item::action("stock.issue", f.t("Warenausgang...", "Issue...")),
                    Item::action("stock.transfer", f.t("Umlagerung...", "Transfer bin...")),
                    Item::action(
                        "stock.writedown",
                        f.t(
                            "Abwertung (Niederstwert)...",
                            "Write-down (Niederstwert)...",
                        ),
                    ),
                    Item::sep(),
                    Item::action("stock.reorder", f.t("Bestellvorschlag", "Reorder plan")),
                    Item::action("stock.drain", f.t("Bus-Eingang prüfen", "Drain bus inbox")),
                ],
            )],
            ..Default::default()
        }
    }

    fn menu_action(&mut self, id: &str, f: &mut Frame) {
        match id {
            "stock.new_sku" => f.form("stock_item_upsert", &[]),
            "stock.import_csv" => f.form("stock_import_csv", &[]),
            "stock.seed" => f.run(f.t("Demo laden", "Seed demo"), "stock_seed", json!({})),
            "stock.export_csv" => f.run_show("stock_export_csv", json!({})),
            "stock.receive" => f.form("stock_receive", &[]),
            "stock.lot_receive" => f.form("stock_lot_receive", &[]),
            "stock.issue" => f.form("stock_issue", &[]),
            "stock.transfer" => f.form("stock_transfer", &[]),
            "stock.writedown" => f.form("stock_writedown", &[]),
            "stock.reorder" => f.run_show("stock_reorder_plan", json!({ "emit_bus": false })),
            "stock.drain" => f.run(
                f.t("Bus prüfen", "Drain inbox"),
                "stock_inbox_drain",
                json!({}),
            ),
            _ => {}
        }
    }

    fn info(&self) -> AppInfo {
        AppInfo {
            name: "StockCraft".into(),
            version: VERSION.into(),
            description: "Local-first inventory & light warehouse management (Lagerhaltung, Moving-Average Valuation, Inventur, Pick Lists, Lots/MHD).".into(),
            license: "MIT OR Apache-2.0".into(),
            repository: "https://github.com/sandraschi/stockcraft".into(),
            safety: "Zero-panic Rust core (#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]), immutable SQLite WAL movement ledger.".into(),
            ..Default::default()
        }
    }

    fn docs(&self) -> Vec<Doc> {
        vec![
            Doc {
                title: "README.md".into(),
                path: PathBuf::from("README.md"),
                text: Some(include_str!("../../../README.md")),
            },
            Doc {
                title: "HELP.md".into(),
                path: PathBuf::from("HELP.md"),
                text: Some(include_str!("../../../HELP.md")),
            },
            Doc {
                title: "docs/PRD.md".into(),
                path: PathBuf::from("docs/PRD.md"),
                text: Some(include_str!("../../../docs/PRD.md")),
            },
            Doc {
                title: "docs/TOOLS.md".into(),
                path: PathBuf::from("docs/TOOLS.md"),
                text: Some(include_str!("../../../docs/TOOLS.md")),
            },
        ]
    }
}

impl StockApp {
    fn view_inventory(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(
            ui,
            f.t("Lagerbestand", "Inventory"),
            f.t(
                "Gleitender Durchschnittspreis (Cent-genau) & Lagerorte",
                "Moving-average valuation (integer cents) & bin locations",
            ),
            |ui| {
                if self.items.is_empty()
                    && widgets::primary(ui, f.t("Demo laden", "Seed demo")).clicked()
                {
                    f.run("Demo", "stock_seed", json!({}));
                }
            },
        );

        let val_str = Money::from_cents(self.total_valuation_cents).format_de();
        let kpis = vec![
            Kpi::new(f.t("Artikel", "SKUs"), self.items.len().to_string()),
            Kpi::new(f.t("Lagerwert", "Valuation"), val_str).note("EKR 1600", Tone::Teal),
            Kpi::new(f.t("Meldebestand", "Low stock"), self.low.len().to_string()).note(
                if self.low.is_empty() { "OK" } else { "Action" },
                if self.low.is_empty() {
                    Tone::Ok
                } else {
                    Tone::Warn
                },
            ),
            Kpi::new(
                f.t("Kommissionen", "Open picks"),
                self.pick_lists
                    .iter()
                    .filter(|p| p.status == "open")
                    .count()
                    .to_string(),
            ),
        ];
        widgets::kpi_row(ui, &kpis);
        ui.add_space(10.0);

        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(f.t("Scanner (EAN/SKU):", "Barcode / EAN:")).strong());
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.scan_code)
                        .hint_text(f.t("Barcode scannen + Enter...", "Scan barcode + Enter..."))
                        .desired_width(220.0),
                );
                for (id, label) in [
                    ("lookup", f.t("Suchen", "Lookup")),
                    ("receive", f.t("+1 Eingang", "+1 Receive")),
                    ("issue", f.t("-1 Ausgang", "-1 Issue")),
                ] {
                    ui.selectable_value(&mut self.scan_action, id.to_string(), label);
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (enter || ui.button(f.t("Ausführen", "Scan")).clicked())
                    && !self.scan_code.trim().is_empty()
                {
                    let code = self.scan_code.trim().to_string();
                    f.run(
                        format!("Scan {code}"),
                        "stock_scan",
                        json!({ "code": code, "action": self.scan_action, "qty": 1 }),
                    );
                    self.scan_code.clear();
                }
            });
        });
        ui.add_space(8.0);

        if self.items.is_empty() {
            widgets::empty_state(
                ui,
                f.t("Noch keine Artikel im Lager", "No SKUs in inventory yet"),
                f.t(
                    "Klicke 'Demo laden' oder '+ Artikel'.",
                    "Click 'Seed demo' or '+ New SKU'.",
                ),
            );
            return;
        }

        let needle = f.search.trim().to_ascii_lowercase();
        widgets::card(ui, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                Grid::new("stock_items_grid")
                    .striped(true)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        for hdr in [
                            "SKU",
                            f.t("Bezeichnung", "Name"),
                            f.t("Lagerort", "Bin"),
                            f.t("Bestand", "Qty"),
                            "Min",
                            f.t("Ø-Preis", "Avg Cost"),
                            f.t("Wert", "Value"),
                            f.t("Aktion", "Action"),
                        ] {
                            ui.label(RichText::new(hdr).strong());
                        }
                        ui.end_row();

                        for it in &self.items {
                            if !needle.is_empty()
                                && !it.sku.to_ascii_lowercase().contains(&needle)
                                && !it.name.to_ascii_lowercase().contains(&needle)
                                && !it.location.to_ascii_lowercase().contains(&needle)
                            {
                                continue;
                            }
                            let selected = self.selected_sku.as_deref() == Some(it.sku.as_str());
                            if ui.selectable_label(selected, &it.sku).clicked() {
                                self.selected_sku = Some(it.sku.clone());
                            }
                            ui.label(&it.name);
                            ui.label(&it.location);
                            let qty_txt = format!("{} {}", it.qty_on_hand, it.unit);
                            if it.min_stock > 0 && it.qty_on_hand <= it.min_stock {
                                widgets::pill(ui, &qty_txt, Tone::Warn);
                            } else {
                                ui.label(qty_txt);
                            }
                            ui.label(it.min_stock.to_string());
                            ui.label(Money::from_cents(it.unit_cost_cents).format_de());
                            let total =
                                Money::from_cents(it.qty_on_hand * it.unit_cost_cents).format_de();
                            widgets::money(ui, &total, false);
                            ui.horizontal(|ui| {
                                if ui.small_button("+").clicked() {
                                    f.form("stock_receive", &[("sku", &it.sku)]);
                                }
                                if ui.small_button("-").clicked() {
                                    f.form("stock_issue", &[("sku", &it.sku)]);
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
        });
    }

    fn view_movements(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(
            ui,
            f.t("Lagerbewegungen", "Stock Movements"),
            f.t(
                "Unveränderliches Buchungsjournal (Storno statt Löschen)",
                "Immutable movement ledger (Storno reversals only)",
            ),
            |_| {},
        );
        if self.movements.is_empty() {
            widgets::empty_state(
                ui,
                f.t("Keine Bewegungen", "No movements yet"),
                f.t(
                    "Buche einen Wareneingang oder Warenausgang.",
                    "Receive or issue stock to record movements.",
                ),
            );
            return;
        }
        widgets::card(ui, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                Grid::new("stock_movements_grid")
                    .striped(true)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for hdr in [
                            "ID",
                            f.t("Typ", "Kind"),
                            "SKU",
                            "Delta",
                            f.t("Wert", "Value"),
                            f.t("Referenz", "Ref"),
                            "",
                        ] {
                            ui.label(RichText::new(hdr).strong());
                        }
                        ui.end_row();
                        for mv in &self.movements {
                            ui.label(format!("#{}", mv.id));
                            let tone = match mv.kind.as_str() {
                                "receive" => Tone::Ok,
                                "issue" => Tone::Info,
                                "storno" => Tone::Danger,
                                _ => Tone::Neutral,
                            };
                            widgets::pill(ui, &mv.kind, tone);
                            ui.label(&mv.sku);
                            ui.label(format!("{:+}", mv.qty_delta));
                            let cents = mv.qty_delta * mv.unit_cost_cents;
                            widgets::money(ui, &Money::from_cents(cents).format_de(), cents < 0);
                            ui.label(&mv.reference);
                            if !mv.stornoed && mv.kind != "storno" {
                                if ui.small_button("Storno").clicked() {
                                    f.run_confirm(
                                        format!("Movement #{} ({}) stornieren?", mv.id, mv.sku),
                                        "Storno",
                                        "stock_storno",
                                        json!({ "movement_id": mv.id, "reason": "UI Storno" }),
                                    );
                                }
                            } else {
                                ui.label("-");
                            }
                            ui.end_row();
                        }
                    });
            });
        });
    }

    fn view_pick_lists(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(
            ui,
            f.t("Kommissionierung", "Pick Lists"),
            f.t(
                "Aus billcraft-Rechnungen, modcraft-Stücklisten oder manuell",
                "From billcraft invoices, modcraft BOMs, or manual orders",
            ),
            |ui| {
                if widgets::primary(ui, f.t("Bus-Eingang prüfen", "Drain bus inbox")).clicked() {
                    f.run("Bus drain", "stock_inbox_drain", json!({}));
                }
            },
        );
        if self.pick_lists.is_empty() {
            widgets::empty_state(
                ui,
                f.t("Keine offenen Kommissionen", "No pick lists yet"),
                f.t(
                    "Prüfe den Bus-Eingang oder erstelle eine Kommission über Ctrl+K.",
                    "Drain the bus inbox or create one via Ctrl+K.",
                ),
            );
            return;
        }
        widgets::card(ui, |ui| {
            for pl in &self.pick_lists {
                ui.horizontal(|ui| {
                    let sel = self.selected_pick_id == Some(pl.id);
                    if ui
                        .selectable_label(sel, format!("#{} {}", pl.id, pl.reference))
                        .clicked()
                    {
                        self.selected_pick_id = Some(pl.id);
                        self.pick_lines = self.store.get_pick_lines(pl.id).unwrap_or_default();
                    }
                    widgets::pill(ui, &pl.source, Tone::Info);
                    let tone = if pl.status == "open" {
                        Tone::Warn
                    } else {
                        Tone::Ok
                    };
                    widgets::pill(ui, &pl.status, tone);
                    ui.label(format!("{} lines", pl.line_count));
                    if pl.status == "open"
                        && ui.small_button(f.t("Ausbuchen", "Commit pick")).clicked()
                    {
                        f.run(
                            "Commit pick",
                            "stock_pick_commit",
                            json!({ "pick_list_id": pl.id }),
                        );
                    }
                });
            }
        });
        if !self.pick_lines.is_empty() {
            ui.add_space(8.0);
            widgets::section(
                ui,
                f.t("Positionen", "Pick lines"),
                Some(self.pick_lines.len()),
            );
            widgets::card(ui, |ui| {
                Grid::new("pick_lines_grid")
                    .striped(true)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        for hdr in [
                            "SKU",
                            f.t("Menge", "Pick Qty"),
                            f.t("Lagerort", "Bin"),
                            f.t("Verfügbar", "On Hand"),
                        ] {
                            ui.label(RichText::new(hdr).strong());
                        }
                        ui.end_row();
                        for line in &self.pick_lines {
                            ui.label(&line.sku);
                            ui.label(line.qty.to_string());
                            ui.label(&line.location);
                            let tone = if line.qty_on_hand >= line.qty {
                                Tone::Ok
                            } else {
                                Tone::Danger
                            };
                            widgets::pill(ui, &line.qty_on_hand.to_string(), tone);
                            ui.end_row();
                        }
                    });
            });
        }
    }

    fn view_inventur(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(
            ui,
            "Inventur",
            f.t(
                "Stichtagszählung und Differenzbuchung",
                "Physical stocktake and variance posting",
            ),
            |ui| {
                if self.active_inventur_id.is_none()
                    && widgets::primary(ui, f.t("Inventur starten", "Start Inventur")).clicked()
                {
                    f.run(
                        "Start Inventur",
                        "stock_inventur_start",
                        json!({ "note": "Inventur" }),
                    );
                }
                if let Some(id) = self.active_inventur_id {
                    if widgets::danger(ui, f.t("Inventur abschließen", "Commit Inventur")).clicked()
                    {
                        f.run_confirm(
                            format!("Inventur #{id} abschließen und Differenzen buchen?"),
                            "Commit Inventur",
                            "stock_inventur_commit",
                            json!({ "stocktake_id": id }),
                        );
                    }
                }
            },
        );
        let Some(id) = self.active_inventur_id else {
            widgets::empty_state(
                ui,
                f.t("Keine aktive Inventur", "No active stocktake"),
                f.t(
                    "Klicke 'Inventur starten' um eine Zählung zu beginnen.",
                    "Click 'Start Inventur' to begin counting.",
                ),
            );
            return;
        };
        widgets::card(ui, |ui| {
            Grid::new("inventur_grid").striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
                for hdr in ["SKU", f.t("Name", "Name"), f.t("Lagerort", "Bin"), f.t("Soll", "Expected"), f.t("Ist", "Counted"), ""] {
                    ui.label(RichText::new(hdr).strong());
                }
                ui.end_row();
                for line in &self.inventur_lines {
                    ui.label(&line.sku);
                    ui.label(&line.name);
                    ui.label(&line.location);
                    ui.label(line.expected_qty.to_string());
                    let buf = self
                        .inventur_inputs
                        .entry(line.sku.clone())
                        .or_insert_with(|| line.expected_qty.to_string());
                    ui.add(egui::TextEdit::singleline(buf).desired_width(64.0));
                    if ui.small_button(f.t("Speichern", "Save")).clicked() {
                        if let Ok(counted) = buf.trim().parse::<i64>() {
                            f.run(
                                "Count",
                                "stock_inventur_count",
                                json!({ "stocktake_id": id, "sku": line.sku, "counted_qty": counted }),
                            );
                        }
                    }
                    ui.end_row();
                }
            });
        });
    }

    fn view_lots(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(
            ui,
            f.t("Chargen & MHD", "Lots & Expiry (FEFO)"),
            f.t(
                "Sortiert nach Ablaufdatum (First Expired, First Out)",
                "Sorted by expiry date (First Expired, First Out)",
            ),
            |ui| {
                if widgets::primary(ui, f.t("+ Charge einbuchen", "+ Receive lot")).clicked() {
                    let pre = self
                        .selected_sku
                        .as_deref()
                        .map(|s| vec![("sku", s)])
                        .unwrap_or_default();
                    f.form("stock_lot_receive", &pre);
                }
            },
        );
        if self.lots.is_empty() {
            widgets::empty_state(
                ui,
                f.t("Keine Chargen erfasst", "No tracked lots yet"),
                f.t(
                    "Nutze '+ Charge einbuchen' um Chargen- und MHD-Daten zu verfolgen.",
                    "Use '+ Receive lot' to track batch numbers and MHD expiry dates.",
                ),
            );
            return;
        }
        widgets::card(ui, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                Grid::new("lots_grid")
                    .striped(true)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        for hdr in [
                            "SKU",
                            f.t("Charge", "Lot No"),
                            f.t("MHD / Ablauf", "Expires"),
                            f.t("Bestand", "Qty"),
                            f.t("EK / Einheit", "Unit Cost"),
                        ] {
                            ui.label(RichText::new(hdr).strong());
                        }
                        ui.end_row();
                        for lot in &self.lots {
                            ui.label(&lot.sku);
                            ui.label(&lot.lot_no);
                            let is_exp = self.expiring_lots.iter().any(|e| e.id == lot.id);
                            if lot.expires_at.is_empty() {
                                ui.label("-");
                            } else if is_exp {
                                widgets::pill(ui, &lot.expires_at, Tone::Warn);
                            } else {
                                widgets::pill(ui, &lot.expires_at, Tone::Ok);
                            }
                            ui.label(lot.qty_on_hand.to_string());
                            ui.label(Money::from_cents(lot.unit_cost_cents).format_de());
                            ui.end_row();
                        }
                    });
            });
        });
    }

    fn view_low_alerts(&mut self, ui: &mut Ui, f: &mut Frame) {
        widgets::page_header(
            ui,
            f.t("Meldebestand", "Low Stock Alerts"),
            f.t(
                "Artikel auf oder unter Mindestbestand",
                "SKUs at or below min_stock threshold",
            ),
            |ui| {
                if !self.low.is_empty()
                    && widgets::primary(ui, f.t("Bestellvorschlag", "Reorder plan")).clicked()
                {
                    f.run_show("stock_reorder_plan", json!({ "emit_bus": false }));
                }
            },
        );
        if self.low.is_empty() {
            widgets::empty_state(
                ui,
                f.t(
                    "Alle Bestände im grünen Bereich",
                    "All stock levels healthy",
                ),
                f.t(
                    "Kein Artikel liegt unter dem Meldebestand.",
                    "No SKUs are at or below min_stock.",
                ),
            );
            return;
        }
        widgets::card(ui, |ui| {
            Grid::new("low_stock_grid")
                .striped(true)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    for hdr in [
                        "SKU",
                        f.t("Bezeichnung", "Name"),
                        f.t("Bestand", "On Hand"),
                        "Min",
                        "",
                    ] {
                        ui.label(RichText::new(hdr).strong());
                    }
                    ui.end_row();
                    for it in &self.low {
                        ui.label(&it.sku);
                        ui.label(&it.name);
                        widgets::pill(ui, &format!("{} {}", it.qty_on_hand, it.unit), Tone::Warn);
                        ui.label(it.min_stock.to_string());
                        if ui
                            .small_button(f.t("Nachbestellen / Eingang", "Receive"))
                            .clicked()
                        {
                            f.form("stock_receive", &[("sku", &it.sku)]);
                        }
                        ui.end_row();
                    }
                });
        });
    }
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

fn main() -> ExitCode {
    main_with(default_db(), StockApp::open, "stock_seed")
}

#[cfg(test)]
mod tests {
    use super::*;

    const COVERED: &[(&str, &str)] = &[
        ("stock_status", "Status bar + --smoke"),
        (
            "stock_seed",
            "File > Seed demo data + Inventory empty state",
        ),
        ("stock_list", "Inventory view"),
        ("stock_item_upsert", "Top bar + New SKU / File > New SKU"),
        ("stock_receive", "Top bar Receive + Inventory + button"),
        ("stock_issue", "Top bar Issue + Inventory - button"),
        ("stock_transfer", "Stock > Transfer bin..."),
        ("stock_storno", "Movements view Storno button"),
        ("stock_movements", "Movements view"),
        ("stock_low_alerts", "Low Stock view"),
        ("stock_valuation", "Inventory KPI row + status bar"),
        ("stock_inventur_start", "Inventur view Start button"),
        ("stock_inventur_count", "Inventur view Save button"),
        ("stock_inventur_commit", "Inventur view Commit button"),
        ("stock_pick_create", "Ctrl+K palette form"),
        ("stock_pick_list", "Pick lists view"),
        ("stock_pick_commit", "Pick lists view Commit pick button"),
        ("stock_export_csv", "File > Export > Inventory CSV"),
        ("stock_import_csv", "File > Import CSV..."),
        (
            "stock_inbox_drain",
            "Pick lists Drain bus inbox + Signals dock",
        ),
        ("stock_lot_receive", "Lots & MHD view + Receive lot"),
        ("stock_lots", "Lots & MHD view"),
        (
            "stock_expiring_lots",
            "Lots & MHD nav badge + warning pills",
        ),
        ("stock_writedown", "Stock > Write-down (Niederstwert)..."),
        ("stock_reorder_plan", "Low Stock view Reorder plan button"),
        ("stock_scan", "Inventory view USB-HID Barcode/EAN Scan bar"),
    ];

    #[test]
    fn all_tools_are_covered() -> Result<(), String> {
        craft_biz_ui::coverage::check(&tools(), COVERED)
    }
}
