use eframe::{egui, App, Frame};
use ldk_node::{
    bitcoin::{Network, Address, secp256k1::PublicKey},
    lightning_invoice::Bolt11Invoice,
    lightning::ln::msgs::SocketAddress,
    config::ChannelConfig,
    Builder, Node, Event
};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};
use ureq::Agent;

use crate::price_feeds::get_latest_price;

// Configuration constants
const EXCHANGE_DATA_DIR: &str = "data/exchange";
const EXCHANGE_NODE_ALIAS: &str = "exchange";
const EXCHANGE_PORT: u16 = 9735;
const DEFAULT_NETWORK: &str = "signet";
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";

#[cfg(feature = "exchange")]
struct ExchangeApp {
    node: Node,
    invoice_amount: String,
    invoice_result: String,
    invoice_to_pay: String,
    on_chain_address: String,
    on_chain_amount: String,
    status_message: String,
    last_update: Instant,
    channel_info: String,
    lightning_balance_btc: f64,
    onchain_balance_btc: f64,
    btc_price: f64,
    lightning_balance_usd: f64,
    onchain_balance_usd: f64,
    total_balance_btc: f64,
    total_balance_usd: f64,
    // Channel opening fields
    node_id_input: String,
    net_address_input: String,
    channel_amount_input: String,
}

#[cfg(feature = "exchange")]
impl ExchangeApp {
    fn new() -> Self {
        println!("Initializing exchange node...");
        
        // Ensure data directory exists
        let data_dir = PathBuf::from(EXCHANGE_DATA_DIR);
        if !data_dir.exists() {
            std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to create data directory: {}", e);
            });
        }

        let mut builder = Builder::new();
        
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
        println!("Setting storage directory: {}", EXCHANGE_DATA_DIR);
        builder.set_storage_dir_path(EXCHANGE_DATA_DIR.to_string());
        
        // Set up listening address
        let listen_addr = format!("127.0.0.1:{}", EXCHANGE_PORT).parse().unwrap();
        println!("Setting listening address: {}", listen_addr);
        builder.set_listening_addresses(vec![listen_addr]).unwrap();
        
        // Set node alias
        builder.set_node_alias(EXCHANGE_NODE_ALIAS.to_string());
        
        // Build the node
        let node = match builder.build() {
            Ok(node) => {
                println!("Exchange node built successfully");
                node
            },
            Err(e) => {
                panic!("Failed to build exchange node: {:?}", e);
            }
        };
        
        // Start the node
        if let Err(e) = node.start() {
            panic!("Failed to start exchange node: {:?}", e);
        }
        
        println!("Exchange node started with ID: {}", node.node_id());
        
        let mut app = Self {
            node,
            invoice_amount: "10000".to_string(), // Default 10k sats
            invoice_result: String::new(),
            invoice_to_pay: String::new(),
            on_chain_address: String::new(),
            on_chain_amount: "10000".to_string(), // Default 10k sats
            status_message: String::new(),
            last_update: Instant::now(),
            channel_info: String::new(),
            lightning_balance_btc: 0.0,
            onchain_balance_btc: 0.0,
            btc_price: 55000.0, // Default BTC price
            lightning_balance_usd: 0.0,
            onchain_balance_usd: 0.0,
            total_balance_btc: 0.0,
            total_balance_usd: 0.0,
            node_id_input: String::new(),
            net_address_input: "127.0.0.1:9736".to_string(), // Default to user node port
            channel_amount_input: "100000".to_string(), // Default 100k sats
        };
        
        // Update balances once initially
        app.update_balances();
        // Update channel info
        app.update_channel_info();
        
        app
    }
    
    fn update_balances(&mut self) {
        let balances = self.node.list_balances();
        
        self.lightning_balance_btc = balances.total_lightning_balance_sats as f64 / 100_000_000.0;
        self.onchain_balance_btc = balances.total_onchain_balance_sats as f64 / 100_000_000.0;
        
        // Fetch the latest BTC price
        if let Ok(latest_price) = get_latest_price(&Agent::new()) {
            self.btc_price = latest_price;
        }
        
        // Calculate USD values
        self.lightning_balance_usd = self.lightning_balance_btc * self.btc_price;
        self.onchain_balance_usd = self.onchain_balance_btc * self.btc_price;
        
        self.total_balance_btc = self.lightning_balance_btc + self.onchain_balance_btc;
        self.total_balance_usd = self.lightning_balance_usd + self.onchain_balance_usd;
    }
    
    fn update_channel_info(&mut self) {
        let channels = self.node.list_channels();
        if channels.is_empty() {
            self.channel_info = "No channels found.".to_string();
        } else {
            let mut info = String::new();
            for (i, channel) in channels.iter().enumerate() {
                info.push_str(&format!(
                    "Channel {}: ID: {}, Value: {} sats, Ready: {}\n", 
                    i + 1,
                    channel.channel_id, 
                    channel.channel_value_sats,
                    channel.is_channel_ready
                ));
            }
            self.channel_info = info;
        }
    }
}

#[cfg(feature = "exchange")]
impl App for ExchangeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Poll for LDK node events
        self.poll_events();
        
        // Update balances and other info periodically
        if self.last_update.elapsed() > Duration::from_secs(30) {
            self.update_balances();
            self.update_channel_info();
            self.last_update = Instant::now();
        }
        
        // Show the exchange interface
        self.show_exchange_screen(ctx);
        
        // Request a repaint frequently to keep the UI responsive
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(feature = "exchange")]
impl ExchangeApp {
    fn poll_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    self.status_message = format!("Channel {} is now ready", channel_id);
                    self.update_channel_info();
                    self.update_balances();
                }
                
                Event::PaymentReceived { payment_hash, amount_msat, .. } => {
                    self.status_message = format!("Received payment of {} msats", amount_msat);
                    self.update_balances();
                }
                
                Event::ChannelClosed { channel_id, .. } => {
                    self.status_message = format!("Channel {} has been closed", channel_id);
                    self.update_channel_info();
                    self.update_balances();
                }
                
                _ => {} // Ignore other events for now
            }
            self.node.event_handled(); // Mark event as handled
        }
    }

    fn show_exchange_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Add a scrollable area that encompasses the entire central panel
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                ui.heading("Exchange");
                ui.add_space(10.0);
                
                // Node information
                ui.group(|ui| {
                    ui.label(format!("Node ID: {}", self.node.node_id()));
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
                
                ui.add_space(20.0);
                
                // Open Channel - New functionality for exchange
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
                        // Parse the node ID
                        match PublicKey::from_str(&self.node_id_input) {
                            Ok(node_id) => {
                                match SocketAddress::from_str(&self.net_address_input) {
                                    Ok(net_address) => {
                                        match self.channel_amount_input.parse::<u64>() {
                                            Ok(sats) => {
                                                // Calculate push_msat (half the channel amount)
                                                // For testing purposes!
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
                                                        // Clear the inputs
                                                        // Keep the IP address in case they want to open another channel
                                                        self.node_id_input.clear();
                                                        self.channel_amount_input = "100000".to_string(); // Reset to default
                                                    },
                                                    Err(e) => {
                                                        self.status_message = format!("Error opening channel: {}", e);
                                                    }
                                                }
                                            },
                                            Err(_) => {
                                                self.status_message = "Invalid amount format".to_string();
                                            }
                                        }
                                    },
                                    Err(_) => {
                                        self.status_message = "Invalid network address format".to_string();
                                    }
                                }
                            },
                            Err(_) => {
                                self.status_message = "Invalid node ID format".to_string();
                            }
                        }
                    }
                });
                
                ui.add_space(20.0);
                
                // Get Invoice
                ui.group(|ui| {
                    ui.label("Generate Invoice");
                    ui.horizontal(|ui| {
                        ui.label("Amount (sats):");
                        ui.text_edit_singleline(&mut self.invoice_amount);
                        if ui.button("Get Invoice").clicked() {
                            if let Ok(amount) = self.invoice_amount.parse::<u64>() {
                                let msats = amount * 1000;
                                match self.node.bolt11_payment().receive(
                                    msats,
                                    &ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
                                        ldk_node::lightning_invoice::Description::new("Exchange Invoice".to_string()).unwrap()
                                    ),
                                    3600,
                                ) {
                                    Ok(invoice) => {
                                        self.invoice_result = invoice.to_string();
                                        self.status_message = "Invoice generated".to_string();
                                    },
                                    Err(e) => {
                                        self.status_message = format!("Error: {}", e);
                                    }
                                }
                            } else {
                                self.status_message = "Invalid amount".to_string();
                            }
                        }
                    });
                    
                    if !self.invoice_result.is_empty() {
                        ui.text_edit_multiline(&mut self.invoice_result);
                        if ui.button("Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = self.invoice_result.clone());
                        }
                    }
                });
                
                ui.add_space(10.0);
                
                // Pay Invoice
                ui.group(|ui| {
                    ui.label("Pay Invoice");
                    ui.text_edit_multiline(&mut self.invoice_to_pay);
                    if ui.button("Pay Invoice").clicked() {
                        match Bolt11Invoice::from_str(&self.invoice_to_pay) {
                            Ok(invoice) => {
                                match self.node.bolt11_payment().send(&invoice, None) {
                                    Ok(payment_id) => {
                                        self.status_message = format!("Payment sent, ID: {}", payment_id);
                                        self.invoice_to_pay.clear();
                                        // Update balances after payment
                                        self.update_balances();
                                    },
                                    Err(e) => {
                                        self.status_message = format!("Payment error: {}", e);
                                    }
                                }
                            },
                            Err(e) => {
                                self.status_message = format!("Invalid invoice: {}", e);
                            }
                        }
                    }
                });
                
                ui.add_space(10.0);
                
                // Get On-chain Address
                ui.group(|ui| {
                    ui.label("On-chain Address");
                    if ui.button("Get Address").clicked() {
                        match self.node.onchain_payment().new_address() {
                            Ok(address) => {
                                self.on_chain_address = address.to_string();
                                self.status_message = "Address generated".to_string();
                            },
                            Err(e) => {
                                self.status_message = format!("Error: {}", e);
                            }
                        }
                    }
                    
                    if !self.on_chain_address.is_empty() {
                        ui.label(self.on_chain_address.clone());
                        if ui.button("Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = self.on_chain_address.clone());
                        }
                    }
                });
                
                ui.add_space(10.0);
                
                // On-chain Send
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
                        if let Ok(amount) = self.on_chain_amount.parse::<u64>() {
                            match Address::from_str(&self.on_chain_address) {
                                Ok(addr) => match addr.require_network(Network::Signet) {
                                    Ok(addr_checked) => {
                                        match self.node.onchain_payment().send_to_address(&addr_checked, amount, None) {
                                            Ok(txid) => {
                                                self.status_message = format!("Transaction sent: {}", txid);
                                                // Update balances after sending
                                                self.update_balances();
                                            },
                                            Err(e) => {
                                                self.status_message = format!("Transaction error: {}", e);
                                            }
                                        }
                                    },
                                    Err(_) => {
                                        self.status_message = "Invalid address for this network".to_string();
                                    }
                                },
                                Err(_) => {
                                    self.status_message = "Invalid address".to_string();
                                }
                            }
                        } else {
                            self.status_message = "Invalid amount".to_string();
                        }
                    }
                });
                
                ui.add_space(10.0);
                
                // List Channels
                ui.group(|ui| {
                    ui.label("Channels");
                    if ui.button("Refresh Channel List").clicked() {
                        self.update_channel_info();
                    }
                    
                    ui.text_edit_multiline(&mut self.channel_info);
                });
                
                // Channel management section
                ui.group(|ui| {
                    ui.label("Channel Management");
                    
                    if ui.button("Close All Channels").clicked() {
                        for channel in self.node.list_channels().iter() {
                            let user_channel_id = channel.user_channel_id.clone();
                            let counterparty_node_id = channel.counterparty_node_id;
                            match self.node.close_channel(&user_channel_id, counterparty_node_id) {
                                Ok(_) => self.status_message = "Closing all channels...".to_string(),
                                Err(e) => self.status_message = format!("Error closing channel: {}", e),
                            }
                        }
                    }
                });
                
                ui.add_space(10.0);
                
                // Status message
                if !self.status_message.is_empty() {
                    ui.label(self.status_message.clone());
                }
            });
            });
        });
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