#[cfg(feature = "user")]
use eframe::{egui, App, Frame};
use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network},
    lightning::ln::msgs::SocketAddress,
    Builder, Node, Event
};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use image::{GrayImage, Luma};
use qrcode::{QrCode, Color};

use std::str::FromStr;

use egui::TextureOptions;
use crate::price_feeds::get_latest_price;
use crate::types::*;

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
const DEFAULT_LSP_PUBKEY: &str = "02fe1194d6359a045419c88304bc9bd77de4b4b19f22f5160f0ca7eb722bd86d27";
#[cfg(feature = "user")]
const DEFAULT_LSP_ADDRESS: &str = "127.0.0.1:9737";
#[cfg(feature = "user")]
const DEFAULT_LSP_AUTH: &str = "00000000000000000000000000000000";
#[cfg(feature = "user")]
const EXPECTED_USD: f64 = 1.0;

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
    invoice_result: String,
    qr_texture: Option<egui::TextureHandle>,
    waiting_for_payment: bool,
    stable_channel: StableChannel,
    is_stable_channel_initialized: bool,
    last_stability_check: Instant,
}

#[cfg(feature = "user")]
impl UserApp {
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
        
        let show_onboarding = node.list_channels().is_empty();
        
        let mut stable_channel = StableChannel::default();
        stable_channel.expected_usd = USD::from_f64(EXPECTED_USD);
        let is_stable_channel_initialized = false;

        let channels = node.list_channels();
        let show_onboarding = channels.is_empty();
    
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
    
    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
            ldk_node::lightning_invoice::Description::new("Stable Channel JIT payment".to_string()).unwrap()
        );
        
    
        // Default to 20k sats (~$10-12 at current prices)
        let result = self.node.bolt11_payment().receive_via_jit_channel(
            2_000_000, // 20k sats in msats
            &description,
            3600, // 1 hour expiry
            Some(10_000_000), // minimum channel size of 10k sats
        );
    
        match result {
            Ok(invoice) => {
                self.invoice_result = invoice.to_string();
                
                // Generate QR code
                let code = QrCode::new(&self.invoice_result).unwrap();
                let bits = code.to_colors();
                let width = code.width();
                let scale_factor = 4;
                let mut imgbuf = GrayImage::new(
                    (width * scale_factor) as u32, 
                    (width * scale_factor) as u32
                );
    
                for y in 0..width {
                    for x in 0..width {
                        let color = if bits[y * width + x] == Color::Dark { 0 } else { 255 };
                        for dy in 0..scale_factor {
                            for dx in 0..scale_factor {
                                imgbuf.put_pixel(
                                    (x * scale_factor + dx) as u32,
                                    (y * scale_factor + dy) as u32,
                                    Luma([color]),
                                );
                            }
                        }
                    }
                }
                
                // Convert to egui texture
                let (w, h) = (imgbuf.width() as usize, imgbuf.height() as usize);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for pixel in imgbuf.pixels() {
                    let lum = pixel[0];
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(lum);
                    rgba.push(255);
                }
                
                let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                self.qr_texture = Some(ctx.load_texture("qr_code", color_image, TextureOptions::LINEAR));
                
                self.status_message = "Invoice generated. Pay it to create a JIT channel.".to_string();
                self.waiting_for_payment = true;
            }
            Err(e) => {
                self.invoice_result = format!("Error: {e:?}");
                self.status_message = format!("Failed to generate invoice: {}", e);
            }
        }
    }
    
    fn poll_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    self.status_message = format!("Channel {} is now ready", channel_id);
                    self.show_onboarding = false;
                    self.waiting_for_payment = false; 
                }
                
                Event::PaymentReceived { amount_msat, .. } => {
                    self.status_message = format!("Received payment of {} msats", amount_msat);

                    // if self.waiting_for_payment {
                    //     self.waiting_for_payment = false;
                    // }
                }
                
                Event::ChannelClosed { channel_id, .. } => {
                    self.status_message = format!("Channel {} has been closed", channel_id);
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
        self.balance_btc = balances.total_lightning_balance_sats as f64 / 100_000_000.0;
        self.balance_usd = self.balance_btc * self.btc_price;
    }

    fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);

            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Send yourself bitcoin to stabilize.")
                        .size(16.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                );
                ui.add_space(3.0);
                ui.label("This is a Bolt11 Lightning invoice.");
                ui.add_space(8.0);

                if let Some(ref qr) = self.qr_texture {
                    ui.image(qr);
                } else {
                    ui.label("Lightning QR Missing");
                }

                ui.add_space(8.0);

                ui.add(
                    egui::TextEdit::multiline(&mut self.invoice_result)
                        .frame(true)
                        .desired_width(400.0)
                        .desired_rows(3)
                        .hint_text("Invoice..."),
                );

                ui.add_space(8.0);

                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Copy Invoice")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0),
                ).clicked() {
                    ui.output_mut(|o| {
                        o.copied_text = self.invoice_result.clone();
                    });
                }
                
                ui.add_space(5.0); 
                
                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Back")
                            .color(egui::Color32::BLACK)
                            .size(16.0), 
                    )
                    .min_size(egui::vec2(120.0, 36.0))
                    .fill(egui::Color32::from_gray(220))
                    .rounding(6.0), 
                ).clicked() {
                    self.waiting_for_payment = false;
                }
                
                ui.add_space(8.0); 
            });
        });
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
                    self.get_jit_invoice(ctx);
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
                let balances = self.node.list_balances();
                let lightning_balance_btc = Bitcoin::from_sats(balances.total_lightning_balance_sats);
                let lightning_balance_usd = USD::from_bitcoin(lightning_balance_btc, self.btc_price);
      
                ui.add_space(30.0);

                ui.group(|ui| {
                    ui.add_space(20.0);
                    ui.heading("Your Stable Balance");
                    ui.add(egui::Label::new(
                        egui::RichText::new(lightning_balance_usd.to_string())
                            .size(36.0)
                            .strong(),
                    ));
                    ui.label(format!("Agreed Peg USD: {}", self.stable_channel.expected_usd));
                    ui.label(format!("Bitcoin: {}", lightning_balance_btc.to_string()));
                    ui.add_space(20.0);
                });

                ui.add_space(20.0);

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
        
        // Update balances frequently (every 5 seconds)
        if self.last_update.elapsed() > Duration::from_secs(5) {
            self.update_balances();
            self.last_update = Instant::now();
        }
        
        // Update price and check stability less frequently (every 30 seconds)
        if self.last_stability_check.elapsed() > Duration::from_secs(30) {
            // Get price once and use it for both operations
            if let Ok(latest_price) = get_latest_price(&ureq::Agent::new()) {
                self.btc_price = latest_price;
                
                // Update the stable channel's price
                self.stable_channel.latest_price = latest_price;
                
                // Call the stability check from stable.rs
                crate::stable::check_stability(&self.node, &mut self.stable_channel);
                
                // Update USD balance with new price
                self.update_balances();
            }
            
            self.last_stability_check = Instant::now();
        }
        
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