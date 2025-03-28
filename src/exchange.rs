// src/exchange.rs
use eframe::{egui, App, Frame};
use std::time::{Duration, Instant};

use crate::base::AppState;
use crate::price_feeds::get_cached_price;

// Configuration constants
const EXCHANGE_DATA_DIR: &str = "data/exchange";
const EXCHANGE_NODE_ALIAS: &str = "exchange";
const EXCHANGE_PORT: u16 = 9735;

#[cfg(feature = "exchange")]
pub struct ExchangeApp {
    base: AppState,
    channel_info: String,
    node_id_input: String,
    net_address_input: String,
    channel_amount_input: String,
}

#[cfg(feature = "exchange")]
impl ExchangeApp {
    fn new() -> Self {
        // Initialize the base AppState
        let base = AppState::new(
            EXCHANGE_DATA_DIR, 
            EXCHANGE_NODE_ALIAS, 
            EXCHANGE_PORT
        );
        
        let mut app = Self {
            base,
            channel_info: String::new(),
            node_id_input: String::new(),
            net_address_input: "127.0.0.1:9737".to_string(), // Default to user node port
            channel_amount_input: "100000".to_string(), // Default 100k sats
        };
        
        // Update channel info initially
        app.update_channel_info();
        
        app
    }
    
    fn update_channel_info(&mut self) {
        self.channel_info = self.base.update_channel_info();
    }

    fn show_channels_table(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Lightning Channels");
            
            // Add refresh button at the top
            if ui.button("Refresh Channel List").clicked() {
                self.update_channel_info();
            }
            
            // Get channels list
            let channels = self.base.node.list_channels();
            
            // Debug info
            ui.label(format!("Debug: Found {} channels", channels.len()));
            if !channels.is_empty() {
                ui.label(format!("First channel ID: {}", channels[0].channel_id));
            }
            
            if channels.is_empty() {
                ui.label("No channels found.");
                return;
            }

            // Use egui_extras for better table rendering
            use egui_extras::{Column, TableBuilder};
            
            // Calculate text height first
            let text_height = egui::TextStyle::Body
                .resolve(ui.style())
                .size
                .max(ui.spacing().interact_size.y);
                
            // Store current price to avoid borrow issues
            let btc_price = self.base.btc_price;
            
            // Store the available height before creating the table builder
            let available_height = ui.available_height();
            
            // Pre-compute the data we need to display for each channel
            let channel_data: Vec<_> = channels.iter().enumerate().map(|(i, channel)| {
                // Calculate values
                let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
                let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
                let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                
                let our_btc = our_balance_sats as f64 / 100_000_000.0;
                let their_btc = their_balance_sats as f64 / 100_000_000.0;
                
                let our_usd = our_btc * btc_price;
                let their_usd = their_btc * btc_price;
                
                // Format channel ID
                let channel_id = channel.channel_id.to_string();
                let short_id = if channel_id.len() > 14 {
                    format!("{}...{}", &channel_id[0..6], &channel_id[channel_id.len() - 6..])
                } else {
                    channel_id.clone()
                };
                
                // Format counterparty pubkey
                let pubkey = channel.counterparty_node_id.to_string();
                let short_pubkey = if pubkey.len() > 8 {
                    format!("{}...", &pubkey[0..8])
                } else {
                    pubkey.clone()
                };
                
                (i + 1, short_id, short_pubkey, our_btc, their_btc, our_usd, their_usd)
            }).collect();

            // Debug information for the pre-computed data
            ui.label(format!("Debug: Pre-computed {} rows of data", channel_data.len()));

            ui.add_space(10.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                // Create the table
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::auto().resizable(true))  // #
                    .column(Column::auto().at_least(100.0).resizable(true))  // Channel ID
                    .column(Column::auto().at_least(80.0).resizable(true))   // Counterparty
                    .column(Column::auto().at_least(80.0).resizable(true))   // Our BTC
                    .column(Column::auto().at_least(80.0).resizable(true))   // Their BTC
                    .column(Column::auto().at_least(80.0).resizable(true))   // Our USD
                    .column(Column::auto().at_least(80.0).resizable(true))   // Their USD
                    .column(Column::auto().at_least(80.0).resizable(true))   // Agreed USD
                    .min_scrolled_height(0.0)
                    .max_scroll_height(available_height) 
                    .header(20.0, |mut header| {
                        header.col(|ui| { ui.strong("#"); });
                        header.col(|ui| { ui.strong("Channel ID"); });
                        header.col(|ui| { ui.strong("Counterparty"); });
                        header.col(|ui| { ui.strong("Our BTC"); });
                        header.col(|ui| { ui.strong("Their BTC"); });
                        header.col(|ui| { ui.strong("Our USD"); });
                        header.col(|ui| { ui.strong("Their USD"); });
                        header.col(|ui| { ui.strong("Agreed USD"); });
                    })
                    .body(|mut body| {
                        for (i, data) in channel_data.iter().enumerate() {
                            body.row(22.0, |mut row| {
                                row.col(|ui| { ui.label(data.0.to_string()); });
                                row.col(|ui| { ui.label(&data.1); });
                                row.col(|ui| { ui.label(&data.2); });
                                row.col(|ui| { ui.label(format!("{:.8}", data.3)); });
                                row.col(|ui| { ui.label(format!("{:.8}", data.4)); });
                                row.col(|ui| { ui.label(format!("${:.2}", data.5)); });
                                row.col(|ui| { ui.label(format!("${:.2}", data.6)); });
                                row.col(|ui| { ui.label("N/A"); });
                            });
                        }
                    });
                });     
                ui.add_space(10.0);

        });
    }

    fn show_exchange_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Add a scrollable area
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Exchange");
                    ui.add_space(10.0);
                    
                    // Node information (using common component)
                    self.base.show_node_info_section(ui, EXCHANGE_PORT);
                    
                    ui.add_space(20.0);
                    
                    // Balance section (using common component)
                    self.base.show_balance_section(ui);
                    
                    ui.add_space(20.0);
                    
                    // Open Channel section (exchange-specific, keep as is)
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
                                &self.channel_amount_input
                            ) {
                                // Success - keep the IP address, clear other fields
                                self.node_id_input.clear();
                                self.channel_amount_input = "100000".to_string();
                            }
                        }
                    });
                    
                    ui.add_space(20.0);
                    
                    // Invoice section (using common component)
                    self.base.show_invoice_section(ui);
                    
                    ui.add_space(10.0);
                    
                    // Pay Invoice section (using common component)
                    self.base.show_pay_invoice_section(ui);
                    
                    ui.add_space(10.0);
                    
                    // Get On-chain Address section (using common component)
                    self.base.show_onchain_address_section(ui);
                    
                    ui.add_space(10.0);
                    
                    // On-chain Send section (using common component)
                    self.base.show_onchain_send_section(ui);
                    
                    ui.add_space(10.0);
                    
                    // Use the new table component instead of the old channels section
                    self.show_channels_table(ui);
                    
                    // Channel management section (exchange-specific, keep as is)
                    ui.group(|ui| {
                        ui.label("Channel Management");
                        
                        if ui.button("Close All Channels").clicked() {
                            for channel in self.base.node.list_channels().iter() {
                                let user_channel_id = channel.user_channel_id.clone();
                                let counterparty_node_id = channel.counterparty_node_id;
                                match self.base.node.close_channel(&user_channel_id, counterparty_node_id) {
                                    Ok(_) => self.base.status_message = "Closing all channels...".to_string(),
                                    Err(e) => self.base.status_message = format!("Error closing channel: {}", e),
                                }
                            }
                        }
                    });
                    
                    ui.add_space(10.0);
                    
                    // Status message
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
        // Poll for LDK node events
        self.base.poll_events();
        
        if self.base.last_update.elapsed() > Duration::from_secs(30) {
            let current_price = get_cached_price();
            
            if current_price > 0.0 {
                self.base.btc_price = current_price;
            }
            
            self.base.update_balances();
            self.update_channel_info();
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
        Box::new(|_cc| {
            // Create the app with initialized LDK node
            Ok(Box::new(ExchangeApp::new()))
        }),
    ).unwrap_or_else(|e| {
        eprintln!("Error starting the application: {:?}", e);
    });
}