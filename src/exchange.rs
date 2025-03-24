// src/exchange.rs
use eframe::{egui, App, Frame};
use std::time::{Duration, Instant};

use crate::base::AppState;

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

    fn show_exchange_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Add a scrollable area
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Exchange");
                    ui.add_space(10.0);
                    
                    // Node information
                    ui.group(|ui| {
                        ui.label(format!("Node ID: {}", self.base.node.node_id()));
                        ui.label(format!("Listening on: 127.0.0.1:{}", EXCHANGE_PORT));
                    });
                    
                    ui.add_space(20.0);
                    
                    // BALANCE SECTION
                    ui.group(|ui| {
                        ui.heading("Balances");
                        ui.add_space(5.0);
                        
                        // Lightning balance
                        ui.horizontal(|ui| {
                            ui.label("Lightning:");
                            ui.monospace(format!("{:.8} BTC", self.base.lightning_balance_btc));
                            ui.monospace(format!("(${:.2})", self.base.lightning_balance_usd));
                        });
                        
                        // On-chain balance
                        ui.horizontal(|ui| {
                            ui.label("On-chain:  ");
                            ui.monospace(format!("{:.8} BTC", self.base.onchain_balance_btc));
                            ui.monospace(format!("(${:.2})", self.base.onchain_balance_usd));
                        });
                        
                        // Total balance
                        ui.horizontal(|ui| {
                            ui.label("Total:     ");
                            ui.strong(format!("{:.8} BTC", self.base.total_balance_btc));
                            ui.strong(format!("(${:.2})", self.base.total_balance_usd));
                        });
                        
                        ui.add_space(5.0);
                        ui.label(format!("Price: ${:.2} | Updated: {} seconds ago", 
                                         self.base.btc_price,
                                         self.base.last_update.elapsed().as_secs()));
                    });
                    
                    ui.add_space(20.0);
                    
                    // Open Channel section
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
                    
                    // Get Invoice
                    ui.group(|ui| {
                        ui.label("Generate Invoice");
                        ui.horizontal(|ui| {
                            ui.label("Amount (sats):");
                            ui.text_edit_singleline(&mut self.base.invoice_amount);
                            if ui.button("Get Invoice").clicked() {
                                self.base.generate_invoice();
                            }
                        });
                        
                        if !self.base.invoice_result.is_empty() {
                            ui.text_edit_multiline(&mut self.base.invoice_result);
                            if ui.button("Copy").clicked() {
                                ui.output_mut(|o| o.copied_text = self.base.invoice_result.clone());
                            }
                        }
                    });
                    
                    ui.add_space(10.0);
                    
                    // Pay Invoice
                    ui.group(|ui| {
                        ui.label("Pay Invoice");
                        ui.text_edit_multiline(&mut self.base.invoice_to_pay);
                        if ui.button("Pay Invoice").clicked() {
                            self.base.pay_invoice();
                        }
                    });
                    
                    ui.add_space(10.0);
                    
                    // Get On-chain Address
                    ui.group(|ui| {
                        ui.label("On-chain Address");
                        if ui.button("Get Address").clicked() {
                            self.base.get_address();
                        }
                        
                        if !self.base.on_chain_address.is_empty() {
                            ui.label(self.base.on_chain_address.clone());
                            if ui.button("Copy").clicked() {
                                ui.output_mut(|o| o.copied_text = self.base.on_chain_address.clone());
                            }
                        }
                    });
                    
                    ui.add_space(10.0);
                    
                    // On-chain Send
                    ui.group(|ui| {
                        ui.label("On-chain Send");
                        ui.horizontal(|ui| {
                            ui.label("Address:");
                            ui.text_edit_singleline(&mut self.base.on_chain_address);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Amount (sats):");
                            ui.text_edit_singleline(&mut self.base.on_chain_amount);
                        });
                        
                        if ui.button("Send On-chain").clicked() {
                            self.base.send_onchain();
                        }
                    });
                    
                    ui.add_space(10.0);
                    
                    // List Channels
                    ui.group(|ui| {
                        ui.heading("Channels");
                        if ui.button("Refresh Channel List").clicked() {
                            self.update_channel_info();
                        }
                        
                        ui.text_edit_multiline(&mut self.channel_info);
                    });
                    
                    // Channel management section
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
        
        // Update balances and other info periodically
        if self.base.last_update.elapsed() > Duration::from_secs(30) {
            self.base.update_balances();
            self.update_channel_info();
            self.base.last_update = Instant::now();
        }
        
        // Show the exchange interface
        self.show_exchange_screen(ctx);
        
        // Request a repaint frequently to keep the UI responsive
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