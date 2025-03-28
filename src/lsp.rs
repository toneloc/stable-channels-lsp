// src/lsp.rs
use eframe::{egui, App, Frame};
use ldk_node::{
    bitcoin::{Network, Address, secp256k1::PublicKey},
    lightning_invoice::Bolt11Invoice,
    lightning::ln::{msgs::SocketAddress, types::ChannelId},
    config::ChannelConfig,
    Builder, Node, liquidity::LSPS2ServiceConfig
};
use std::str::FromStr;
use std::time::{Duration, Instant};
use hex;

use crate::base::AppState;
use crate::types::*;
use crate::stable;
use crate::price_feeds::get_cached_price;


// Configuration constants
const LSP_DATA_DIR: &str = "data/lsp";
const LSP_NODE_ALIAS: &str = "lsp";
const LSP_PORT: u16 = 9737;
const EXPECTED_USD: f64 = 15.0;  // Default expected USD value for stable channels

#[cfg(feature = "lsp")]
pub struct LspApp {
    // Base app state with common fields
    base: AppState,
    
    // LSP-specific fields
    channel_id_to_close: String,
    stable_channels: Vec<StableChannel>,
    selected_channel_id: String,
    stable_channel_amount: String,
    last_stability_check: Instant,
}

#[cfg(feature = "lsp")]
impl LspApp {
    fn new() -> Self {
        println!("Initializing LSP node...");
        
        // Create a node builder
        let mut builder = Builder::new();
        
        // Configure LSPS2 service
        let service_config = LSPS2ServiceConfig {
            require_token: None,
            advertise_service: true,
            channel_opening_fee_ppm: 1_000,
            channel_over_provisioning_ppm: 1_000_000,
            min_channel_opening_fee_msat: 0,
            min_channel_lifetime: 100,
            max_client_to_self_delay: 1024,
            min_payment_size_msat: 0,
            max_payment_size_msat: 100_000_000_000,
        };

        builder.set_liquidity_provider_lsps2(service_config);
        
        // Initialize the base AppState with our custom builder
        let base = AppState::new(
            LSP_DATA_DIR, 
            LSP_NODE_ALIAS, 
            LSP_PORT, 
            None
        );
        
        let mut app = Self {
            base,
            channel_id_to_close: String::new(),
            stable_channels: Vec::new(),
            selected_channel_id: String::new(),
            stable_channel_amount: EXPECTED_USD.to_string(),
            last_stability_check: Instant::now(),
        };
        
        app
    }

    fn designate_stable_channel(&mut self) {
        if self.selected_channel_id.is_empty() {
            self.base.status_message = "Please select a channel ID".to_string();
            return;
        }
        
        let amount = match self.stable_channel_amount.parse::<f64>() {
            Ok(val) => val,
            Err(_) => {
                self.base.status_message = "Invalid amount format".to_string();
                return;
            }
        };
        
        let channel_id_str = self.selected_channel_id.trim();
        
        for channel in self.base.node.list_channels() {
            let channel_id_string = channel.channel_id.to_string();
            
            if channel_id_string.contains(channel_id_str) {
                let expected_usd = USD::from_f64(amount);
                let expected_btc = Bitcoin::from_usd(expected_usd, self.base.btc_price);
                
                let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
                let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
                let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                
                let stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
                let stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
                
                let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, self.base.btc_price);
                let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, self.base.btc_price);
                
                let stable_channel = StableChannel {
                    channel_id: channel.channel_id,
                    counterparty: channel.counterparty_node_id,
                    is_stable_receiver: false, 
                    expected_usd,
                    expected_btc,
                    stable_receiver_btc,
                    stable_receiver_usd,
                    stable_provider_btc,
                    stable_provider_usd,
                    latest_price: self.base.btc_price,
                    risk_level: 0,
                    payment_made: false,
                    timestamp: 0,
                    formatted_datetime: "".to_string(),
                    sc_dir: LSP_DATA_DIR.to_string(),
                    prices: "".to_string(),
                };
                
                let mut found = false;
                for sc in &mut self.stable_channels {
                    if sc.channel_id == channel.channel_id {
                        *sc = stable_channel.clone();
                        found = true;
                        break;
                    }
                }
                
                if !found {
                    self.stable_channels.push(stable_channel);
                }
                
                self.base.status_message = format!(
                    "Channel {} designated as stable with target amount of ${}",
                    channel_id_string, amount
                );
                
                self.selected_channel_id.clear();
                self.stable_channel_amount = EXPECTED_USD.to_string();
                
                return;
            }
        }
        
        self.base.status_message = format!("No channel found matching: {}", self.selected_channel_id);
    }
    
    fn check_and_update_stable_channels(&mut self) {
        let current_price = get_cached_price();

        if current_price > 0.0 {
            self.base.btc_price = current_price;
        }
        
        for sc in &mut self.stable_channels {
            if !stable::channel_exists(&self.base.node, &sc.channel_id) {
                continue;
            }
            
            sc.latest_price = current_price;
            
            // Pass the current price to check_stability
            stable::check_stability(&self.base.node, sc, current_price);
        }
    }
    
    fn update_channel_info(&mut self) -> String {
        let channels = self.base.node.list_channels();
        if channels.is_empty() {
            return "No channels found.".to_string();
        } else {
            let mut info = String::new();
            for (i, channel) in channels.iter().enumerate() {
                // Check if this channel is a stable channel
                let is_stable = self.stable_channels.iter().any(|sc| sc.channel_id == channel.channel_id);
                
                info.push_str(&format!(
                    "Channel {}: ID: {}, Value: {} sats, Ready: {}{}\n", 
                    i + 1,
                    channel.channel_id, 
                    channel.channel_value_sats,
                    channel.is_channel_ready,
                    if is_stable { " [STABLE]" } else { "" }
                ));
            }
            return info;
        }
    }
    
    fn close_specific_channel(&mut self) {
        if self.channel_id_to_close.is_empty() {
            self.base.status_message = "Please enter a channel ID to close".to_string();
            return;
        }
    
        // Try to parse the channel ID (could be hex or formatted)
        let channel_id_str = self.channel_id_to_close.trim();
        
        // First check if this is a channel ID in hex format
        if channel_id_str.len() == 64 && channel_id_str.chars().all(|c| c.is_ascii_hexdigit()) {
            // It's a hex string, convert to bytes
            match hex::decode(channel_id_str) {
                Ok(bytes) if bytes.len() == 32 => {
                    let mut found = false;
                    for channel in self.base.node.list_channels().iter() {
                        // Compare the bytes of the channel ID
                        let channel_id_bytes = channel.channel_id.0.to_vec();
                        if channel_id_bytes == bytes {
                            found = true;
                            let user_channel_id = channel.user_channel_id.clone();
                            let counterparty_node_id = channel.counterparty_node_id;
                            match self.base.node.close_channel(&user_channel_id, counterparty_node_id) {
                                Ok(_) => {
                                    self.base.status_message = format!("Closing channel with ID: {}", self.channel_id_to_close);
                                    self.channel_id_to_close.clear(); // Clear the field after successful operation
                                    
                                    // Remove from stable channels if it was a stable channel
                                    self.stable_channels.retain(|sc| sc.channel_id != channel.channel_id);
                                },
                                Err(e) => {
                                    self.base.status_message = format!("Error closing channel: {}", e);
                                }
                            }
                            break;
                        }
                    }
                    
                    if !found {
                        self.base.status_message = format!("No channel found with ID: {}", self.channel_id_to_close);
                    }
                },
                _ => {
                    self.base.status_message = "Invalid channel ID format".to_string();
                }
            }
        } else {
            // Try to find a channel with ID that contains the provided string
            // This allows for partial matching with formatted channel IDs
            let mut found = false;
            for channel in self.base.node.list_channels().iter() {
                let channel_id_string = channel.channel_id.to_string();
                if channel_id_string.contains(channel_id_str) {
                    found = true;
                    let user_channel_id = channel.user_channel_id.clone();
                    let counterparty_node_id = channel.counterparty_node_id;
                    match self.base.node.close_channel(&user_channel_id, counterparty_node_id) {
                        Ok(_) => {
                            self.base.status_message = format!("Closing channel with ID: {}", channel_id_string);
                            self.channel_id_to_close.clear(); // Clear the field after successful operation
                            
                            // Remove from stable channels if it was a stable channel
                            self.stable_channels.retain(|sc| sc.channel_id != channel.channel_id);
                        },
                        Err(e) => {
                            self.base.status_message = format!("Error closing channel: {}", e);
                        }
                    }
                    break;
                }
            }
            
            if !found {
                self.base.status_message = format!("No channel found matching: {}", self.channel_id_to_close);
            }
        }
    }

    fn show_lsp_screen(&mut self, ctx: &egui::Context) {
        let channel_info = self.update_channel_info();
        
        egui::CentralPanel::default().show(ctx, |ui| {
            // Add a scrollable area that encompasses the entire central panel
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("Lightning Service Provider");
                    ui.add_space(10.0);
                    
                    // Node information (using common component)
                    self.base.show_node_info_section(ui, LSP_PORT);
                    
                    ui.add_space(20.0);
                    
                    // Balance section (using common component)
                    self.base.show_balance_section(ui);
                    
                    // STABLE CHANNELS SECTION (LSP-specific, keep as is)
                    ui.add_space(20.0);
                    ui.group(|ui| {
                        ui.heading("Stable Channels");
                        
                        // Display existing stable channels
                        if self.stable_channels.is_empty() {
                            ui.label("No stable channels configured");
                        } else {
                            for (i, sc) in self.stable_channels.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{}. Channel: {}", i+1, sc.channel_id));
                                    ui.label(format!("Target: ${:.2}", sc.expected_usd.0));
                                });
                                
                                // Show balances
                                ui.horizontal(|ui| {
                                    ui.label("    User balance:");
                                    ui.label(format!("{:.8} BTC (${:.2})", sc.stable_receiver_btc.to_btc(), sc.stable_receiver_usd.0));
                                });
                                
                                ui.horizontal(|ui| {
                                    ui.label("    LSP balance:");
                                    ui.label(format!("{:.8} BTC (${:.2})", sc.stable_provider_btc.to_btc(), sc.stable_provider_usd.0));
                                });
                                
                                ui.add_space(5.0);
                            }
                        }
                        
                        ui.add_space(10.0);
                        
                        ui.label("Designate Stable Channel:");
                        
                        ui.horizontal(|ui| {
                            ui.label("Channel ID:");
                            ui.text_edit_singleline(&mut self.selected_channel_id);
                        });
                        
                        ui.horizontal(|ui| {
                            ui.label("Target USD amount:");
                            ui.text_edit_singleline(&mut self.stable_channel_amount);
                        });
                        
                        if ui.add(
                            egui::Button::new("Designate as Stable")
                                .min_size(egui::vec2(150.0, 30.0))
                        ).clicked() {
                            self.designate_stable_channel();
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
                    
                    // Close Specific Channel (LSP-specific, keep as is)
                    ui.group(|ui| {
                        ui.heading("Close Specific Channel");
                        ui.horizontal(|ui| {
                            ui.label("Channel ID:");
                            ui.text_edit_singleline(&mut self.channel_id_to_close);
                            
                            if ui.button("Close Channel").clicked() {
                                self.close_specific_channel();
                            }
                        });
                    });
                    
                    ui.add_space(10.0);
                    
                    // List Channels section (using common component)
                    self.base.show_channels_section(ui, &mut channel_info.clone());
                    
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

#[cfg(feature = "lsp")]
impl App for LspApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Poll for LDK node events
        self.base.poll_events();
        
        // Update balances and other info periodically
        if self.base.last_update.elapsed() > Duration::from_secs(30) {
            // Get the cached price
            let current_price = get_cached_price();
            
            // Update the base price if valid
            if current_price > 0.0 {
                self.base.btc_price = current_price;
            }
            
            self.base.update_balances();
            self.base.last_update = Instant::now();
        }
        
        // Check stability of stable channels periodically
        if self.last_stability_check.elapsed() > Duration::from_secs(10) {
            self.check_and_update_stable_channels();
            self.last_stability_check = Instant::now();
        }
        
        // Show the LSP interface
        self.show_lsp_screen(ctx);
        
        // Request a repaint frequently to keep the UI responsive
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(feature = "lsp")]
pub fn run() {
    println!("Starting LSP Interface...");
    
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([500.0, 800.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "Lightning Service Provider",
        native_options,
        Box::new(|_cc| {
            // Create the app with initialized LDK node
            Ok(Box::new(LspApp::new()))
        }),
    ).unwrap_or_else(|e| {
        eprintln!("Error starting the application: {:?}", e);
    });
}