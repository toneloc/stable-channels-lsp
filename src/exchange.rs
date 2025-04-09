// src/exchange.rs

use eframe::{egui, App, Frame};
use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::{
    lightning::ln::msgs::SocketAddress,
    lightning::ln::types::ChannelId,
    Event, Builder
};
use std::str::FromStr;
use std::time::{Duration, Instant};

use crate::base::AppState;
use crate::price_feeds::get_cached_price;
use crate::stable::{self, check_stability, update_balances};
use crate::types::{StableChannel, Bitcoin, USD};

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
    // Store official balances here:
    pub stable_channel: StableChannel,
    last_stability_check: Instant,
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

        // TODO : now need for stable channel on exchnage app
        // Initialize a default StableChannel (adjust fields as desired)
        let stable_channel = StableChannel {
            channel_id: ChannelId::from_bytes([0u8; 32]),
            counterparty: PublicKey::from_str("02289a7dcb107fe975d9383ec14ff7849cee976eb723f4b1ff4a33c080617b77b4").unwrap(),
            is_stable_receiver: false, // Exchange as provider
            expected_usd: USD::from_f64(10.0),
            expected_btc: Bitcoin::from_btc(0.0005),
            stable_receiver_btc: Bitcoin::default(),
            stable_receiver_usd: USD::default(),
            stable_provider_btc: Bitcoin::default(),
            stable_provider_usd: USD::default(),
            latest_price: base.btc_price,
            risk_level: 0,
            payment_made: false,
            timestamp: 0,
            formatted_datetime: "".to_string(),
            sc_dir: EXCHANGE_DATA_DIR.to_string(),
            prices: "".to_string(),
        };

        let mut app = Self {
            base,
            channel_info: String::new(),
            node_id_input: String::new(),
            net_address_input: "127.0.0.1:9737".to_string(),
            channel_amount_input: "100000".to_string(),
            stable_channel,
            last_stability_check: Instant::now(),
        };

        // Initial channel info load
        app.update_channel_info();

        // Optionally check stability right away
        check_stability(&app.base.node, &mut app.stable_channel, app.base.btc_price);
        app
    }

    fn poll_exchange_events(&mut self) {
        while let Some(event) = self.base.node.next_event() {
            match event {
                Event::PaymentReceived { amount_msat, .. } => {
                    let (found, _) = update_balances(&self.base.node, &mut self.stable_channel);
                    
                    self.base.status_message = if found {
                        format!("Received payment of {} msats", amount_msat)
                    } else {
                        format!(
                            "Received {} msats but no matching stable channel found",
                            amount_msat
                        )
                    };
                }
                Event::ChannelReady { channel_id, .. } => {
                    self.base.status_message = format!("Channel {channel_id} is now ready");
                }
                Event::ChannelClosed { channel_id, .. } => {
                    self.base.status_message = format!("Channel {channel_id} has been closed");
                }
                _ => {}
            }
            self.base.node.event_handled();
        }
    }

    fn update_channel_info(&mut self) {
        // This returns a debug string about channels; we keep it for display
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

                    // Show stable channel balances
                    ui.group(|ui| {
                        ui.heading("Stable Channel Balance (Exchange Side)");
                        let stable_btc = if self.stable_channel.is_stable_receiver {
                            self.stable_channel.stable_receiver_btc
                        } else {
                            self.stable_channel.stable_provider_btc
                        };
                        let stable_usd = if self.stable_channel.is_stable_receiver {
                            self.stable_channel.stable_receiver_usd
                        } else {
                            self.stable_channel.stable_provider_usd
                        };
                        ui.label(format!("BTC: {:.8}", stable_btc));
                        ui.label(format!("USD: {}", stable_usd));
                        ui.label(format!("Target USD: {}", self.stable_channel.expected_usd));
                    });

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
        self.poll_exchange_events();

        // Periodic stability check
        if self.last_stability_check.elapsed() > Duration::from_secs(30) {
            let current_price = get_cached_price();
            if current_price > 0.0 {
                check_stability(&self.base.node, &mut self.stable_channel, current_price);
                self.base.btc_price = current_price;
                self.base.last_update = Instant::now();
            }
            self.last_stability_check = Instant::now();
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
