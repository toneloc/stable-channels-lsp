use std::str::FromStr;
use std::time::{Duration, Instant};
use eframe::{egui, App, Frame};
use image::{GrayImage, Luma};
use qrcode::{QrCode, Color};
use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network},
    lightning::ln::msgs::SocketAddress,
    Builder, Node, Event
};
use ureq::Agent;

use crate::price_feeds::get_latest_price;
use crate::types::*;
use crate::stable; // Add this import

// Configuration constants
const USER_DATA_DIR: &str = "data/user";
const USER_NODE_ALIAS: &str = "user";
const USER_PORT: u16 = 9736;
const DEFAULT_NETWORK: &str = "signet";
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";
const DEFAULT_LSP_PUBKEY: &str = "02fe1194d6359a045419c88304bc9bd77de4b4b19f22f5160f0ca7eb722bd86d27";
const DEFAULT_LSP_ADDRESS: &str = "127.0.0.1:9737";
const DEFAULT_LSP_AUTH: &str = "00000000000000000000000000000000";
const DEFAULT_EXPECTED_USD: f64 = 20.0; // Default stable channel amount

/// The main app state for the user interface
#[cfg(feature = "user")]
struct UserApp {
    node: Node,
    balance_btc: f64,
    balance_usd: f64,
    btc_price: f64,
    show_onboarding: bool,
    last_update: Instant,
    status_message: String,
    // Added fields for invoice and QR code
    invoice_result: String,
    qr_texture: Option<egui::TextureHandle>,
    waiting_for_payment: bool,
    // Added stable channel fields
    stable_channel: StableChannel,
    is_stable_channel_initialized: bool,
    last_stability_check: Instant,
}

#[cfg(feature = "user")]
impl UserApp {
    fn new() -> Self {
        println!("Initializing user node...");
        
        // Ensure data directory exists
        let data_dir = std::path::PathBuf::from(USER_DATA_DIR);
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
            match DEFAULT_LSP_ADDRESS.parse::<std::net::SocketAddr>() {
                Ok(socket_addr) => {
                    // Convert to SocketAddress for LDK
                    let ldk_socket_addr = SocketAddress::from(socket_addr);
                    
                    println!("Setting LSP with address: {} and pubkey: {}", 
                             DEFAULT_LSP_ADDRESS, lsp_pubkey);
                    
                    // The correct parameter order is (pubkey, address, auth_token)
                    builder.set_liquidity_source_lsps2(
                        lsp_pubkey,
                        ldk_socket_addr, 
                        Some(DEFAULT_LSP_AUTH.to_string())
                    );
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

        // connect to exchange node
        let target_node_id = "037639cf15e5a71adf33c4e92522aa17de79773d18d870ba17f100a3728dbab0e6";
        if let Ok(pubkey) = PublicKey::from_str(target_node_id) {
            let socket_addr = SocketAddress::from_str("127.0.0.1:9735").unwrap(); 
            match node.connect(pubkey, socket_addr, true) {
                Ok(_) => println!("Successfully connected to node: {}", target_node_id),
                Err(e) => println!("Failed to connect to node: {}", e),
            }
        } else {
            println!("Failed to parse node ID: {}", target_node_id);
        }
        
        // Initialize stable channel (we'll try to set it up when a channel is created)
        let stable_channel = StableChannel::default();
        
        let show_onboarding = node.list_channels().is_empty();
        let is_stable_channel_initialized = false;
        
        Self {
            node,
            balance_btc: 0.0,
            balance_usd: 0.0,
            btc_price: 0.0, 
            show_onboarding,
            last_update: Instant::now(),
            status_message: String::new(),
            invoice_result: String::new(),
            qr_texture: None,
            waiting_for_payment: false,
            stable_channel,
            is_stable_channel_initialized,
            last_stability_check: Instant::now(),
        }
    }
    
    // Add stability check function
    fn check_stability(&mut self) {
        if self.last_stability_check.elapsed() >= Duration::from_secs(30) {
            // If we have no channels, don't try to check stability
            if self.node.list_channels().is_empty() {
                return;
            }
            
            if !self.is_stable_channel_initialized {
                // Try to initialize with first channel
                let channels = self.node.list_channels();
                if let Some(channel) = channels.first() {
                    // Initialize the stable channel
                    match stable::initialize_stable_channel(
                        &self.node,
                        self.stable_channel.clone(),
                        &channel.channel_id.to_string(),
                        true, // We're the stable receiver
                        DEFAULT_EXPECTED_USD,
                        0.0 // No native bitcoin amount
                    ) {
                        Ok(updated_channel) => {
                            self.stable_channel = updated_channel;
                            self.is_stable_channel_initialized = true;
                            self.status_message = "Stable channel initialized".to_string();
                        },
                        Err(e) => {
                            self.status_message = format!("Failed to initialize stable channel: {}", e);
                        }
                    }
                }
            } else {
                // Run stability check on existing channel
                let (action, updated_channel) = stable::check_stability(
                    &self.node,
                    self.stable_channel.clone(),
                    self.is_stable_channel_initialized
                );
                
                // Update our stored channel
                self.stable_channel = updated_channel;
                
                // Handle the stability action
                match action {
                    stable::StabilityAction::Pay(amount) => {
                        self.status_message = "Paying to maintain stability...".to_string();
                        
                        match stable::execute_payment(&self.node, amount, &self.stable_channel) {
                            Ok(payment_id) => {
                                self.status_message = format!("Stability payment sent: {}", payment_id);
                            },
                            Err(e) => {
                                self.status_message = format!("Stability payment failed: {}", e);
                            }
                        }
                    },
                    stable::StabilityAction::Wait => {
                        self.status_message = "Waiting for counterparty payment to maintain stability...".to_string();
                    },
                    stable::StabilityAction::DoNothing => {
                        self.status_message = "Channel is stable.".to_string();
                    },
                    stable::StabilityAction::HighRisk(risk) => {
                        self.status_message = format!("Channel has high risk level: {}", risk);
                    },
                    stable::StabilityAction::NotInitialized => {
                        // Channel may have been closed
                        self.is_stable_channel_initialized = false;
                        self.status_message = "Stable channel no longer valid.".to_string();
                    }
                }
            }
            
            self.last_stability_check = Instant::now();
        }
    }
    
    // Existing functions...
    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        // Existing code...
    }
    
    fn poll_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    self.status_message = format!("Channel {} is now ready", channel_id);
                    self.show_onboarding = false;
                    self.waiting_for_payment = false; // Exit payment screen if we were waiting
                    
                    // Try to initialize stable channel if not already initialized
                    if !self.is_stable_channel_initialized {
                        // Initialize the stable channel with this channel
                        match stable::initialize_stable_channel(
                            &self.node,
                            self.stable_channel.clone(),
                            &channel_id.to_string(),
                            true, // We're the stable receiver
                            DEFAULT_EXPECTED_USD,
                            0.0 // No native bitcoin amount
                        ) {
                            Ok(updated_channel) => {
                                self.stable_channel = updated_channel;
                                self.is_stable_channel_initialized = true;
                                self.status_message = "Stable channel initialized".to_string();
                            },
                            Err(e) => {
                                self.status_message = format!("Failed to initialize stable channel: {}", e);
                            }
                        }
                    }
                }
                
                Event::PaymentReceived { amount_msat, .. } => {
                    self.status_message = format!("Received payment of {} msats", amount_msat);
                    // Move to main screen if we were waiting
                    if self.waiting_for_payment {
                        self.waiting_for_payment = false;
                    }
                }
                
                Event::ChannelClosed { channel_id, .. } => {
                    self.status_message = format!("Channel {} has been closed", channel_id);
                    
                    // If this was our stable channel, mark as uninitialized
                    if self.is_stable_channel_initialized && channel_id == self.stable_channel.channel_id {
                        self.is_stable_channel_initialized = false;
                    }
                    
                    // If no channels left, go back to onboarding
                    if self.node.list_channels().is_empty() {
                        self.show_onboarding = true;
                        self.waiting_for_payment = false;
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
        
        // Get the latest BTC price
        if let Ok(latest_price) = get_latest_price(&Agent::new()) {
            self.btc_price = latest_price;
            // Update the stable channel price too if initialized
            if self.is_stable_channel_initialized {
                self.stable_channel.latest_price = latest_price;
            }
        }
        
        // Calculate USD value
        self.balance_usd = self.balance_btc * self.btc_price;
    }

    fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
        // Existing code...
    }

    fn show_onboarding_screen(&mut self, ctx: &egui::Context) {
        // Existing code...
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
                ui.add_space(30.0);

                // Stable balance display
                ui.group(|ui| {
                    ui.add_space(20.0);
                    ui.heading("Your Stable Balance");
                    
                    if self.is_stable_channel_initialized {
                        ui.add(egui::Label::new(
                            egui::RichText::new(self.stable_channel.stable_receiver_usd.to_string())
                                .size(36.0)
                                .strong(),
                        ));
                        ui.label(format!("Agreed Peg USD: {}", self.stable_channel.expected_usd));
                        ui.label(format!("Bitcoin: {}", self.stable_channel.stable_receiver_btc.to_string()));
                    } else {
                        ui.label("No stable channel initialized yet.");
                        if !self.node.list_channels().is_empty() {
                            ui.label("Stability check runs automatically every 30 seconds.");
                        }
                    }
                    ui.add_space(20.0);
                });

                ui.add_space(20.0);

                // Bitcoin price info
                ui.group(|ui| {
                    ui.add_space(20.0);
                    ui.heading("Bitcoin Price");
                    ui.label(format!("${:.2}", self.btc_price));
                    ui.add_space(20.0);

                    let last_updated = self.last_update.elapsed().as_secs();
                    ui.add_space(5.0);
                    ui.label(
                        egui::RichText::new(format!("Last updated: {}s ago", last_updated))
                            .size(12.0)
                            .color(egui::Color32::GRAY),
                    );
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
                            
                            // Show if this is the stable channel
                            if self.is_stable_channel_initialized && channel.channel_id == self.stable_channel.channel_id {
                                ui.label(
                                    egui::RichText::new("(Stable Channel)")
                                        .italics()
                                        .color(egui::Color32::GREEN)
                                );
                            }
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
impl App for UserApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Poll for LDK node events
        self.poll_events();
        
        // Update balances periodically
        if self.last_update.elapsed() > Duration::from_secs(5) {
            self.update_balances();
            self.last_update = Instant::now();
        }
        
        // Run stability check (this will only do something every 30 seconds)
        self.check_stability();
        
        // Show appropriate screen
        if self.waiting_for_payment {
            self.show_waiting_for_payment_screen(ctx);
        } else if self.show_onboarding {
            self.show_onboarding_screen(ctx);
        } else {
            self.show_main_screen(ctx);
        }
        
        // Request a repaint frequently to keep the UI responsive
        ctx.request_repaint_after(Duration::from_millis(100));
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
            Ok(Box::new(UserApp::new()))
        }),
    ).unwrap_or_else(|e| {
        eprintln!("Error starting the application: {:?}", e);
    });
}