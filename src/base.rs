// src/base.rs
use ldk_node::{
    bitcoin::{Network, Address, secp256k1::PublicKey},
    lightning_invoice::Bolt11Invoice,
    lightning::ln::{msgs::SocketAddress, types::ChannelId},
    config::ChannelConfig,
    Builder, Node, Event
};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};
use ureq::Agent;

use crate::price_feeds::get_cached_price;

// Common configuration constants can stay here or move to a constants.rs
pub const DEFAULT_NETWORK: &str = "signet";
pub const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";

pub struct AppState {
    pub node: Node,
    pub btc_price: f64,
    pub status_message: String,
    pub last_update: Instant,
    
    // Common UI fields
    pub invoice_amount: String,
    pub invoice_result: String,
    pub invoice_to_pay: String,
    pub on_chain_address: String,
    pub on_chain_amount: String,
    
    // Balance fields
    pub lightning_balance_btc: f64,
    pub onchain_balance_btc: f64,
    pub lightning_balance_usd: f64,
    pub onchain_balance_usd: f64,
    pub total_balance_btc: f64,
    pub total_balance_usd: f64,
}

impl AppState {
    pub fn new(
        mut builder: Builder,
        data_dir: &str,
        node_alias: &str,
        port: u16,
    ) -> Self {

        // Ensure data directory exists
        let data_dir_path = PathBuf::from(data_dir);
        if !data_dir_path.exists() {
            std::fs::create_dir_all(&data_dir_path).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to create data directory: {}", e);
            });
        }
        
        // Configure the network
        let network = match DEFAULT_NETWORK.to_lowercase().as_str() {
            "signet" => Network::Signet,
            "testnet" => Network::Testnet,
            "bitcoin" => Network::Bitcoin,
            _ => {
                println!("Warning: Unknown network in config, defaulting to Signet");
                Network::Signet
            }
        };
    
        println!("Setting network to: {:?}", network);
        builder.set_network(network);
        
        // Set up Esplora chain source
        println!("Setting Esplora API URL: {}", DEFAULT_CHAIN_SOURCE_URL);
        builder.set_chain_source_esplora(DEFAULT_CHAIN_SOURCE_URL.to_string(), None);
        
        // Set up data directory
        println!("Setting storage directory: {}", data_dir);
        builder.set_storage_dir_path(data_dir.to_string());
        
        // Set up listening address
        let listen_addr = format!("127.0.0.1:{}", port).parse().unwrap();
        println!("Setting listening address: {}", listen_addr);
        builder.set_listening_addresses(vec![listen_addr]).unwrap();
        
        // Set node alias
        builder.set_node_alias(node_alias.to_string());
        
        // Build the node
        let node = match builder.build() {
            Ok(node) => {
                println!("Node built successfully");
                node
            },
            Err(e) => {
                panic!("Failed to build node: {:?}", e);
            }
        };
        
        // Start the node
        if let Err(e) = node.start() {
            panic!("Failed to start node: {:?}", e);
        }
        
        println!("Node started with ID: {}", node.node_id());
        
        // Get initial price
        let btc_price = get_cached_price();
    
        let mut app_state = Self {
            node,
            btc_price,
            status_message: String::new(),
            last_update: Instant::now(),
            invoice_amount: "1000".to_string(),
            invoice_result: String::new(),
            invoice_to_pay: String::new(),
            on_chain_address: String::new(),
            on_chain_amount: "10000".to_string(),
            lightning_balance_btc: 0.0,
            onchain_balance_btc: 0.0,
            lightning_balance_usd: 0.0,
            onchain_balance_usd: 0.0,
            total_balance_btc: 0.0,
            total_balance_usd: 0.0,
        };
        
        // Update balances initially
        app_state.update_balances();
        
        app_state
    }
    
    pub fn update_balances(&mut self) {
        let current_price = get_cached_price();
        if current_price > 0.0 {
            self.btc_price = current_price;
        }
        
        let balances = self.node.list_balances();
        
        self.lightning_balance_btc = balances.total_lightning_balance_sats as f64 / 100_000_000.0;
        self.onchain_balance_btc = balances.total_onchain_balance_sats as f64 / 100_000_000.0;
        
        // Calculate USD values
        self.lightning_balance_usd = self.lightning_balance_btc * self.btc_price;
        self.onchain_balance_usd = self.onchain_balance_btc * self.btc_price;
        
        self.total_balance_btc = self.lightning_balance_btc + self.onchain_balance_btc;
        self.total_balance_usd = self.lightning_balance_usd + self.onchain_balance_usd;
    }
    
    pub fn poll_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    self.status_message = format!("Channel {} is now ready", channel_id);
                    self.update_balances();
                }
                
                Event::PaymentReceived { amount_msat, .. } => {
                    self.status_message = format!("Received payment of {} msats", amount_msat);
                    self.update_balances();
                }
                
                Event::ChannelClosed { channel_id, .. } => {
                    self.status_message = format!("Channel {} has been closed", channel_id);
                    self.update_balances();
                }
                
                _ => {} // Ignore other events for now
            }
            self.node.event_handled(); // Mark event as handled
        }
    }
    
    pub fn generate_invoice(&mut self) -> bool {
        if let Ok(amount) = self.invoice_amount.parse::<u64>() {
            let msats = amount * 1000;
            match self.node.bolt11_payment().receive(
                msats,
                &ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                    ldk_node::lightning_invoice::Description::new("Invoice".to_string()).unwrap()
                ),
                3600,
            ) {
                Ok(invoice) => {
                    self.invoice_result = invoice.to_string();
                    self.status_message = "Invoice generated".to_string();
                    true
                },
                Err(e) => {
                    self.status_message = format!("Error: {}", e);
                    false
                }
            }
        } else {
            self.status_message = "Invalid amount".to_string();
            false
        }
    }
    
    pub fn pay_invoice(&mut self) -> bool {
        match Bolt11Invoice::from_str(&self.invoice_to_pay) {
            Ok(invoice) => {
                match self.node.bolt11_payment().send(&invoice, None) {
                    Ok(payment_id) => {
                        self.status_message = format!("Payment sent, ID: {}", payment_id);
                        self.invoice_to_pay.clear();
                        self.update_balances();
                        true
                    },
                    Err(e) => {
                        self.status_message = format!("Payment error: {}", e);
                        false
                    }
                }
            },
            Err(e) => {
                self.status_message = format!("Invalid invoice: {}", e);
                false
            }
        }
    }
    
    pub fn get_address(&mut self) -> bool {
        match self.node.onchain_payment().new_address() {
            Ok(address) => {
                self.on_chain_address = address.to_string();
                self.status_message = "Address generated".to_string();
                true
            },
            Err(e) => {
                self.status_message = format!("Error: {}", e);
                false
            }
        }
    }
    
    pub fn send_onchain(&mut self) -> bool {
        if let Ok(amount) = self.on_chain_amount.parse::<u64>() {
            match Address::from_str(&self.on_chain_address) {
                Ok(addr) => match addr.require_network(Network::Signet) {
                    Ok(addr_checked) => {
                        match self.node.onchain_payment().send_to_address(&addr_checked, amount, None) {
                            Ok(txid) => {
                                self.status_message = format!("Transaction sent: {}", txid);
                                self.update_balances();
                                true
                            },
                            Err(e) => {
                                self.status_message = format!("Transaction error: {}", e);
                                false
                            }
                        }
                    },
                    Err(_) => {
                        self.status_message = "Invalid address for this network".to_string();
                        false
                    }
                },
                Err(_) => {
                    self.status_message = "Invalid address".to_string();
                    false
                }
            }
        } else {
            self.status_message = "Invalid amount".to_string();
            false
        }
    }
    
    pub fn update_channel_info(&self) -> String {
        let channels = self.node.list_channels();
        if channels.is_empty() {
            return "No channels found.".to_string();
        } else {
            let mut info = String::new();
            for (i, channel) in channels.iter().enumerate() {
                info.push_str(&format!(
                    "Channel {}: ID: {}\n  Value: {} sats\n  Ready: {}\n  Outbound Capacity: {} msats\n  Next Outbound HTLC Limit: {} msats\n\n", 
                    i + 1,
                    channel.channel_id, 
                    channel.channel_value_sats,
                    channel.is_channel_ready,
                    channel.outbound_capacity_msat,
                    channel.next_outbound_htlc_limit_msat
                ));
            }
            return info;
        }
    }
    
    pub fn open_channel(&mut self, node_id_str: &str, net_address_str: &str, channel_amount_str: &str) -> bool {
        // Parse the node ID
        match PublicKey::from_str(node_id_str) {
            Ok(node_id) => {
                match SocketAddress::from_str(net_address_str) {
                    Ok(net_address) => {
                        match channel_amount_str.parse::<u64>() {
                            Ok(sats) => {
                                // Calculate push_msat (half the channel amount for testing)
                                let push_msat = (sats / 2) * 1000;
                                let channel_config: Option<ChannelConfig> = None;
                                
                                match self.node.open_announced_channel(
                                    node_id,
                                    net_address,
                                    sats,
                                    Some(push_msat),
                                    channel_config,
                                ) {
                                    Ok(_) => {
                                        self.status_message = format!(
                                            "Channel opening initiated with {} for {} sats", 
                                            node_id, sats
                                        );
                                        true
                                    },
                                    Err(e) => {
                                        self.status_message = format!("Error opening channel: {}", e);
                                        false
                                    }
                                }
                            },
                            Err(_) => {
                                self.status_message = "Invalid amount format".to_string();
                                false
                            }
                        }
                    },
                    Err(_) => {
                        self.status_message = "Invalid network address format".to_string();
                        false
                    }
                }
            },
            Err(_) => {
                self.status_message = "Invalid node ID format".to_string();
                false
            }
        }
    }
    pub fn show_invoice_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.label("Generate Invoice");
            ui.horizontal(|ui| {
                ui.label("Amount (sats):");
                ui.text_edit_singleline(&mut self.invoice_amount);
                if ui.button("Get Invoice").clicked() {
                    self.generate_invoice();
                }
            });
            
            if !self.invoice_result.is_empty() {
                ui.text_edit_multiline(&mut self.invoice_result);
                if ui.button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = self.invoice_result.clone());
                }
            }
        });
    }
    
    pub fn show_pay_invoice_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.label("Pay Invoice");
            ui.text_edit_multiline(&mut self.invoice_to_pay);
            if ui.button("Pay Invoice").clicked() {
                self.pay_invoice();
            }
        });
    }
    
    pub fn show_onchain_address_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.label("On-chain Address");
            if ui.button("Get Address").clicked() {
                self.get_address();
            }
            
            if !self.on_chain_address.is_empty() {
                ui.label(self.on_chain_address.clone());
                if ui.button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = self.on_chain_address.clone());
                }
            }
        });
    }
    
    pub fn show_onchain_send_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.label("On-chain Send");
            ui.horizontal(|ui| {
                ui.label("Address:");
                ui.text_edit_singleline(&mut self.on_chain_address);
            });
            ui.horizontal(|ui| {
                ui.label("Amount (sats):");
                ui.text_edit_singleline(&mut self.on_chain_amount);
            });
            
            if ui.button("Send On-chain").clicked() {
                self.send_onchain();
            }
        });
    }
    
    pub fn show_balance_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Balances");
            ui.add_space(5.0);
            
            // Lightning balance
            ui.horizontal(|ui| {
                ui.label("Lightning:");
                ui.monospace(format!("{:.8} BTC", self.lightning_balance_btc));
                ui.monospace(format!("(${:.2})", self.lightning_balance_usd));
            });
            
            // On-chain balance
            ui.horizontal(|ui| {
                ui.label("On-chain:  ");
                ui.monospace(format!("{:.8} BTC", self.onchain_balance_btc));
                ui.monospace(format!("(${:.2})", self.onchain_balance_usd));
            });
            
            // Total balance
            ui.horizontal(|ui| {
                ui.label("Total:     ");
                ui.strong(format!("{:.8} BTC", self.total_balance_btc));
                ui.strong(format!("(${:.2})", self.total_balance_usd));
            });
            
            ui.add_space(5.0);
            ui.label(format!("Price: ${:.2} | Updated: {} seconds ago", 
                             self.btc_price,
                             self.last_update.elapsed().as_secs()));
        });
    }
    
    pub fn show_node_info_section(&mut self, ui: &mut egui::Ui, port: u16) {
        ui.group(|ui| {
            ui.label(format!("Node ID: {}", self.node.node_id()));
            ui.label(format!("Listening on: 127.0.0.1:{}", port));
        });
    }
    
    pub fn show_channels_section(&mut self, ui: &mut egui::Ui, channel_info: &mut String) {
        ui.group(|ui| {
            ui.heading("Channels");
            if ui.button("Refresh Channel List").clicked() {
                *channel_info = self.update_channel_info();
            }
            
            ui.text_edit_multiline(channel_info);
        });
    }
}