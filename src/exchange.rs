// src/exchange.rs

use eframe::{egui, App, Frame};
use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::{
    lightning::ln::msgs::SocketAddress,
    Event, Builder
};
use std::str::FromStr;
use std::time::{Duration, Instant};

use crate::base::AppState;
use crate::price_feeds::get_cached_price;

const EXCHANGE_DATA_DIR: &str = "data/exchange";
const EXCHANGE_NODE_ALIAS: &str = "exchange";
const EXCHANGE_PORT: u16 = 9735;

#[cfg(feature = "exchange")]
pub struct ExchangeApp {
    pub base: AppState,
    channel_info: String,
    node_id_input: String,
    net_address_input: String,
    channel_amount_input: String,
    last_update: Instant,
}

#[cfg(feature = "exchange")]
impl ExchangeApp {
    pub fn new() -> Self {
        let builder = Builder::new();
        let mut base = AppState::new(builder, EXCHANGE_DATA_DIR, EXCHANGE_NODE_ALIAS, EXCHANGE_PORT);

        // Attempt to load a cached BTC price for display
        let current_price = get_cached_price();
        if current_price > 0.0 {
            base.btc_price = current_price;
        }

        let mut app = Self {
            base,
            channel_info: String::new(),
            node_id_input: String::new(),
            net_address_input: "127.0.0.1:9737".to_string(),
            channel_amount_input: "100000".to_string(),
            last_update: Instant::now(),
        };

        // Initial channel info load
        app.update_channel_info();

        app
    }

    fn update_channel_info(&mut self) {
        self.channel_info = self.base.update_channel_info();
    }

    /// Displays a table of channels (purely informational).
    fn show_channels_table(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Lightning Channels");
            if ui.button("Refresh Channel List").clicked() {
                self.update_channel_info();
            }
            let channels = self.base.node.list_channels();
            ui.label(format!("Found {} channels", channels.len()));
            if channels.is_empty() {
                ui.label("No channels found.");
                return;
            }
            // You can reuse your existing table logic or keep it simple:
            for c in channels {
                ui.label(format!(
                    "ID: {} - Capacity: {} sats",
                    c.channel_id, c.channel_value_sats
                ));
            }
        });
    }

    fn show_exchange_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Exchange");
                    ui.add_space(10.0);

                    // Node info
                    self.base.show_node_info_section(ui, EXCHANGE_PORT);
                    ui.add_space(20.0);

                    // Balance section
                    self.base.show_balance_section(ui);
                    ui.add_space(20.0);

                    // Open channel UI
                    ui.group(|ui| {
                        ui.heading("Open Channel");
                        ui.horizontal(|ui| {
                            ui.label("Node ID:");
                            ui.text_edit_singleline(&mut self.node_id_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Net Address:");
                            ui.text_edit_singleline(&mut self.net_address_input);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Amount (sats):");
                            ui.text_edit_singleline(&mut self.channel_amount_input);
                        });
                        if ui.button("Open Channel").clicked() {
                            if self.base.open_channel(
                                &self.node_id_input,
                                &self.net_address_input,
                                &self.channel_amount_input,
                            ) {
                                self.node_id_input.clear();
                                self.channel_amount_input = "100000".to_string();
                            }
                        }
                    });

                    ui.add_space(20.0);

                    // Common invoice UI
                    self.base.show_invoice_section(ui);
                    ui.add_space(10.0);
                    self.base.show_pay_invoice_section(ui);
                    ui.add_space(10.0);
                    self.base.show_onchain_address_section(ui);
                    ui.add_space(10.0);
                    self.base.show_onchain_send_section(ui);
                    ui.add_space(10.0);

                    // Channels table
                    self.show_channels_table(ui);

                    ui.group(|ui| {
                        ui.label("Channel Management");
                        if ui.button("Close All Channels").clicked() {
                            for channel in self.base.node.list_channels().iter() {
                                let user_channel_id = channel.user_channel_id.clone();
                                let counterparty_node_id = channel.counterparty_node_id;
                                match self.base.node.close_channel(
                                    &user_channel_id,
                                    counterparty_node_id,
                                ) {
                                    Ok(_) => {
                                        self.base.status_message =
                                            "Closing all channels...".to_string()
                                    }
                                    Err(e) => {
                                        self.base.status_message =
                                            format!("Error closing channel: {}", e)
                                    }
                                }
                            }
                        }
                    });

                    ui.add_space(10.0);
                    if !self.base.status_message.is_empty() {
                        ui.label(self.base.status_message.clone());
                    }
                });
            });
        });
    }
}

#[cfg(feature = "exchange")]
impl App for ExchangeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.base.poll_events();
    
        // Periodic update of balances and price
        if self.base.last_update.elapsed() > Duration::from_secs(30) {
            let current_price = get_cached_price();
            if current_price > 0.0 {
                self.base.btc_price = current_price;
            }
    
            self.base.update_balances(); // <- ensure this exists and is wired correctly
    
            self.base.last_update = Instant::now();
        }
    
        self.show_exchange_screen(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(feature = "exchange")]
pub fn run() {
    println!("Starting Exchange Interface...");
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([500.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Lightning Exchange",
        native_options,
        Box::new(|_cc| Ok(Box::new(ExchangeApp::new()))),
    )
    .unwrap_or_else(|e| {
        eprintln!("Error starting the application: {:?}", e);
    });
}