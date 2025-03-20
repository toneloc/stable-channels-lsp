#[cfg(feature = "user")]
use eframe::{egui, App, Frame};
use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network},
    Builder, Node, Event
};
use std::path::PathBuf;
use std::str::FromStr;
use hex;
use std::time::{Duration, Instant};

// Configuration constants
#[cfg(feature = "user")]
const USER_DATA_DIR: &str = "data/user";
#[cfg(feature = "user")]
const USER_NODE_ALIAS: &str = "user";
#[cfg(feature = "user")]
const USER_PORT: u16 = 9736;
#[cfg(feature = "user")]
const DEFAULT_NETWORK: &str = "signet";
#[cfg(feature = "user")]
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";
#[cfg(feature = "user")]
const DEFAULT_LSP_PUBKEY: &str = "02f66757a6204814d0996bf819a47024de6f18c3878e7797938d13a69a54d3791b";
#[cfg(feature = "user")]
const DEFAULT_LSP_ADDRESS: &str = "127.0.0.1:9737";
#[cfg(feature = "user")]
const DEFAULT_LSP_AUTH: &str = "00000000000000000000000000000000";

/// The main app state for the user interface
#[cfg(feature = "user")]
struct StableChannelsApp {
    node: Node,
    balance_btc: f64,
    balance_usd: f64,
    btc_price: f64,
    show_onboarding: bool,
    last_update: Instant,
    status_message: String,
}

#[cfg(feature = "user")]
impl StableChannelsApp {
    fn new() -> Self {
        println!("Initializing user node...");
        
        // Ensure data directory exists
        let data_dir = PathBuf::from(USER_DATA_DIR);
        if !data_dir.exists() {
            std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to create data directory: {}", e);
            });
        }

        let mut builder = Builder::new();
        
        // Parse LSP pubkey if available
        let lsp_pubkey = if !DEFAULT_LSP_PUBKEY.is_empty() {
            match hex::decode(DEFAULT_LSP_PUBKEY) {
                Ok(bytes) => {
                    match PublicKey::from_slice(&bytes) {
                        Ok(key) => {
                            println!("Setting LSP pubkey: {}", key);
                            Some(key)
                        },
                        Err(e) => {
                            println!("Error parsing LSP pubkey: {:?}", e);
                            None
                        }
                    }
                },
                Err(e) => {
                    println!("Error decoding LSP pubkey: {:?}", e);
                    None
                }
            }
        } else {
            None
        };
        
        // Configure LSP if pubkey is available
        if let Some(lsp_pubkey) = lsp_pubkey {
            match DEFAULT_LSP_ADDRESS.parse() {
                Ok(addr) => {
                    builder.set_liquidity_source_lsps2(addr, lsp_pubkey, Some(DEFAULT_LSP_AUTH.to_string()));
                    println!("LSP configured with address: {}", DEFAULT_LSP_ADDRESS);
                },
                Err(e) => {
                    println!("Error parsing LSP address: {:?}", e);
                }
            }
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
        println!("Setting storage directory: {}", USER_DATA_DIR);
        builder.set_storage_dir_path(USER_DATA_DIR.to_string());
        
        // Set up listening address for the user node
        let listen_addr = format!("127.0.0.1:{}", USER_PORT).parse().unwrap();
        println!("Setting listening address: {}", listen_addr);
        builder.set_listening_addresses(vec![listen_addr]).unwrap();
        
        // Set node alias
        builder.set_node_alias(USER_NODE_ALIAS.to_string());
        
        // Build the node
        let node = match builder.build() {
            Ok(node) => {
                println!("User node built successfully");
                node
            },
            Err(e) => {
                panic!("Failed to build user node: {:?}", e);
            }
        };
        
        // Start the node
        if let Err(e) = node.start() {
            panic!("Failed to start user node: {:?}", e);
        }
        
        println!("User node started with ID: {}", node.node_id());
        
        // Determine if we show onboarding based on existing channels
        let show_onboarding = node.list_channels().is_empty();
        
        Self {
            node,
            balance_btc: 0.0,
            balance_usd: 0.0,
            btc_price: 60000.0, // Default placeholder price
            show_onboarding,
            last_update: Instant::now(),
            status_message: String::new(),
        }
    }
}

#[cfg(feature = "user")]
impl App for StableChannelsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Poll for LDK node events
        self.poll_events();
        
        // Update balances periodically
        if self.last_update.elapsed() > Duration::from_secs(5) {
            self.update_balances();
            self.last_update = Instant::now();
        }
        
        // Show appropriate screen
        if self.show_onboarding {
            self.show_onboarding_screen(ctx);
        } else {
            self.show_main_screen(ctx);
        }
        
        // Request a repaint frequently to keep the UI responsive
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(feature = "user")]
impl StableChannelsApp {
    fn poll_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    self.status_message = format!("Channel {} is now ready", channel_id);
                    self.show_onboarding = false;
                }
                
                Event::PaymentReceived { payment_hash, amount_msat, .. } => {
                    self.status_message = format!("Received payment of {} msats", amount_msat);
                }
                
                Event::ChannelClosed { channel_id, .. } => {
                    self.status_message = format!("Channel {} has been closed", channel_id);
                    // If no channels left, go back to onboarding
                    if self.node.list_channels().is_empty() {
                        self.show_onboarding = true;
                    }
                }
                
                _ => {} // Ignore other events for now
            }
            self.node.event_handled(); // Mark event as handled
        }
    }
    
    fn update_balances(&mut self) {
        let balances = self.node.list_balances();
        // Convert from sats to BTC
        self.balance_btc = balances.total_lightning_balance_sats as f64 / 100_000_000.0;
        // Calculate USD value
        self.balance_usd = self.balance_btc * self.btc_price;
    }

    fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Stable Channels v0.1")
                        .size(28.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(50.0);
    
                // Step 1
                ui.heading(
                    egui::RichText::new("Step 1: Get a Lightning invoice ⚡")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(r#"Press the "Stabilize" button below."#)
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 2
                ui.heading(
                    egui::RichText::new("Step 2: Send yourself bitcoin 💸")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Over Lightning, from an app or an exchange.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(20.0);
    
                // Step 3
                ui.heading(
                    egui::RichText::new("Step 3: Stable channel created 🔧")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Self-custody. Your keys, your coins.")
                        .color(egui::Color32::GRAY),
                );
    
                ui.add_space(50.0);
    
                // Create channel button
                let subtle_orange = egui::Color32::from_rgba_premultiplied(247, 147, 26, 200); 
                let create_channel_button = egui::Button::new(
                    egui::RichText::new("Stabilize")
                        .color(egui::Color32::WHITE)
                        .strong()
                        .size(18.0),
                )
                .min_size(egui::vec2(200.0, 55.0))
                .fill(subtle_orange)
                .rounding(8.0);
    
                if ui.add(create_channel_button).clicked() {
                    self.status_message = "Getting JIT channel invoice...".to_string();
                    // TODO: Implement JIT invoice generation
                }
                
                // Show status message if there is one
                if !self.status_message.is_empty() {
                    ui.add_space(20.0);
                    ui.label(self.status_message.clone());
                }
                
                // Show node ID
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.label("Node ID: ");
                    let node_id = self.node.node_id().to_string();
                    let node_id_short = format!("{}...{}", &node_id[0..10], &node_id[node_id.len()-10..]);
                    ui.monospace(node_id_short);
                    
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = node_id);
                    }
                });
            });
        });
    }

    fn show_main_screen(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Stable Channels");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Settings").clicked() {
                        // TODO: Show settings
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.group(|ui| {
                    ui.heading("Your Stable Balance");
                    ui.add_space(5.0);
                    ui.heading(format!("${:.2}", self.balance_usd));
                    ui.label(format!("BTC: {:.8}", self.balance_btc));
                });
                
                ui.add_space(20.0);
                
                ui.group(|ui| {
                    ui.heading("Bitcoin Price");
                    ui.add_space(5.0);
                    ui.label(format!("${:.2}", self.btc_price));
                });
                
                ui.add_space(20.0);
                
                // Show channels
                ui.group(|ui| {
                    ui.heading("Lightning Channels");
                    ui.add_space(5.0);
                    
                    let channels = self.node.list_channels();
                    if channels.is_empty() {
                        ui.label("No channels found.");
                    } else {
                        for channel in channels {
                            ui.label(format!(
                                "Channel: {} - {} sats", 
                                channel.channel_id, 
                                channel.channel_value_sats
                            ));
                        }
                    }
                });
                
                ui.add_space(20.0);
                
                // Status message
                if !self.status_message.is_empty() {
                    ui.label(self.status_message.clone());
                    ui.add_space(10.0);
                }
                
                // Action buttons
                if ui.button("Create New Channel").clicked() {
                    self.show_onboarding = true;
                }
                
                if ui.button("Get On-chain Address").clicked() {
                    match self.node.onchain_payment().new_address() {
                        Ok(address) => {
                            self.status_message = format!("Deposit address: {}", address);
                        },
                        Err(e) => {
                            self.status_message = format!("Error generating address: {}", e);
                        }
                    }
                }
            });
        });
    }
}

#[cfg(feature = "user")]
pub fn run() {
    println!("Starting User Interface...");
    
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([460.0, 700.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "Stable Channels",
        native_options,
        Box::new(|_cc| {
            // Create the app with initialized LDK node and wrap in Ok()
            Ok(Box::new(StableChannelsApp::new()))
        }),
    ).unwrap_or_else(|e| {
        eprintln!("Error starting the application: {:?}", e);
    });
}