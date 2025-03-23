#[cfg(feature = "user")]
use eframe::{egui, App, Frame};
use ldk_node::{
    bitcoin::{secp256k1::PublicKey, Network}, lightning::ln::{msgs::SocketAddress, types::ChannelId}, lightning_invoice::Bolt11Invoice, Builder, Event, Node
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
const DEFAULT_LSP_PUBKEY: &str = "03cd003757e0b02089abefeaa68b48965d6eac77c456a4d915a60016e76994a881";
#[cfg(feature = "user")]
const DEFAULT_LSP_ADDRESS: &str = "127.0.0.1:9737";
#[cfg(feature = "user")]
const DEFAULT_LSP_AUTH: &str = "00000000000000000000000000000000";
#[cfg(feature = "user")]
const EXPECTED_USD: f64 = 15.0;
const DEFAULT_GATEWAY_PUBKEY: &str = "033232aa4a2a78ca7e7e61d8378fa5398e7112e86f8a62684ab6513d1f3f4598bc";

/// This is the main app state for the user interface
#[cfg(feature = "user")]
struct UserApp {
    node: Node,
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
    invoice_amount: String,
    invoice_to_pay: String,
}

#[cfg(feature = "user")]
impl UserApp {
    fn new() -> Self {
        // Hello, let's start up
        println!("Initializing user node ... ");
        
        // We should ensure the data directory exists. 
        // This is where we store all the data!
        let data_dir = PathBuf::from(USER_DATA_DIR);
        if !data_dir.exists() {
            std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
                eprintln!("Warning: Failed to create data directory: {}", e);
            });
        }

        // This is for LDK
        let mut builder = Builder::new();
        
        // Parse LSP pubkey if available
        // Should extract this
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
                    let ldk_socket_addr = SocketAddress::from(socket_addr);
                    
                    println!("Setting LSP with address: {} and pubkey: {}", 
                             DEFAULT_LSP_ADDRESS, lsp_pubkey);
                    
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
        
        println!("Setting Esplora API URL: {}", DEFAULT_CHAIN_SOURCE_URL);
        builder.set_chain_source_esplora(DEFAULT_CHAIN_SOURCE_URL.to_string(), None);
        
        println!("Setting storage directory: {}", USER_DATA_DIR);
        builder.set_storage_dir_path(USER_DATA_DIR.to_string());
        
        let listen_addr = format!("127.0.0.1:{}", USER_PORT).parse().unwrap();
        println!("Setting listening address: {}", listen_addr);
        builder.set_listening_addresses(vec![listen_addr]).unwrap();
        
        let _ = builder.set_node_alias(USER_NODE_ALIAS.to_string());
        
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

        // This connects to the "gateway" node
        // This is a node that connects to our LSP
        if let Ok(pubkey) = PublicKey::from_str(DEFAULT_GATEWAY_PUBKEY) {
            let socket_addr = SocketAddress::from_str("127.0.0.1:9735").unwrap(); 
            match node.connect(pubkey, socket_addr, true) {
                Ok(_) => println!("Successfully connected to node: {}", DEFAULT_GATEWAY_PUBKEY),
                Err(e) => println!("Failed to connect to node: {}", e),
            }
        } else {
            println!("Failed to parse node ID: {}", DEFAULT_GATEWAY_PUBKEY);
        }
                
        // Now we have inited the LDK node
        // Let's init the Stable Channel
        let expected_usd = USD::from_f64(EXPECTED_USD);
        let agent = ureq::Agent::new();
        let latest_price = get_latest_price(&agent);
        let price = latest_price.expect("latest_price fetch failed");

        // Use the first available channel, if any
        let channel = node.list_channels().into_iter().next();

        let mut stable_channel = if let Some(channel) = channel {
            let channel_id = channel.channel_id;
            let counterparty = channel.counterparty_node_id;
            let is_stable_receiver = true;

            let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
            let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
            let their_balance_sats = channel.channel_value_sats - our_balance_sats;

            let (stable_receiver_btc, stable_provider_btc) = if is_stable_receiver {
                (Bitcoin::from_sats(our_balance_sats), Bitcoin::from_sats(their_balance_sats))
            } else {
                (Bitcoin::from_sats(their_balance_sats), Bitcoin::from_sats(our_balance_sats))
            };

            let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, price);
            let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, price);

            let stable_provider_btc = Bitcoin::from_usd(stable_provider_usd, price);
            let expected_btc = Bitcoin::from_usd(expected_usd, price);

            StableChannel {
                channel_id,
                counterparty: lsp_pubkey.unwrap(),
                is_stable_receiver,
                expected_usd,
                expected_btc,
                stable_receiver_btc,
                stable_receiver_usd,
                stable_provider_btc,
                stable_provider_usd,
                latest_price: price,
                risk_level: 0,
                payment_made: false,
                timestamp: 0,
                formatted_datetime: "2021-06-01 12:00:00".to_string(),
                sc_dir: "/".to_string(),
                prices: "".to_string(),
            }
        } else {
            eprintln!("No channels found. Creating empty stable channel.");
            StableChannel {
                channel_id: ChannelId::from_bytes([0u8; 32]),
                counterparty: lsp_pubkey.unwrap(),
                is_stable_receiver: true,
                expected_usd,
                expected_btc: Bitcoin::from_btc(0.0),
                stable_receiver_btc: Bitcoin::from_btc(0.0),
                stable_receiver_usd: USD::from_f64(0.0),
                stable_provider_btc: Bitcoin::from_btc(0.0),
                stable_provider_usd: USD::from_f64(0.0),
                latest_price: price,
                risk_level: 0,
                payment_made: false,
                timestamp: 0,
                formatted_datetime: "2021-06-01 12:00:00".to_string(),
                sc_dir: "/".to_string(),
                prices: "".to_string(),
            }
        };

        let channels = node.list_channels();
        let show_onboarding = channels.is_empty();

        crate::stable::check_stability(&node, &mut stable_channel);

        Self {
            node,
            btc_price: stable_channel.latest_price,
            show_onboarding,
            last_update: Instant::now(),
            status_message: String::new(),
            invoice_result: String::new(),
            qr_texture: None,
            waiting_for_payment: false,
            stable_channel,
            is_stable_channel_initialized: true,
            last_stability_check: Instant::now(),
            invoice_amount: String::new(),
            invoice_to_pay: String::new(),
        }

    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
            ldk_node::lightning_invoice::Description::new("Stable Channel JIT payment".to_string()).unwrap()
        );
        

        let result = self.node.bolt11_payment().receive_via_jit_channel(
            20_000_000, 
            &description,
            3600, // 1 hour expiry
            Some(1_000_000), // minimum channel size of 10k sats
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
                    crate::stable::check_stability(&self.node, &mut self.stable_channel);
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

    /// Shows the “waiting for payment” screen with the JIT invoice
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

    /// The “onboarding” screen that prompts the user to stabilize
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

    /// The main screen once the user has a channel
    fn show_main_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);

                // Display stable channel user balances
                ui.group(|ui| {
                    ui.add_space(20.0);
                    ui.heading("Your Stable Balance");

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

                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("{}", stable_usd))
                                .size(36.0)
                                .strong(),
                        )
                    );
                    ui.label(format!("Agreed Peg USD: {}", self.stable_channel.expected_usd));
                    ui.label(format!("Bitcoin: {:.8}", stable_btc));
                    ui.add_space(20.0);
                });
    
                ui.add_space(20.0);
    
                // Display the fetched BTC price
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

                // Simple invoice generator UI
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
                                        ldk_node::lightning_invoice::Description::new("LSP Invoice".to_string()).unwrap()
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
                                        // TODO

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
        
        // We are no longer calling self.update_balances() every 5 seconds,
        // because we now rely on stable channel data for the user’s UI balance.

        // Update stable channel & price every 30 seconds
        if self.last_stability_check.elapsed() > Duration::from_secs(10) {
            if let Ok(latest_price) = get_latest_price(&ureq::Agent::new()) {
                self.btc_price = latest_price;
                self.stable_channel.latest_price = latest_price;
                
                // Run stability check on the stable channel
                crate::stable::check_stability(&self.node, &mut self.stable_channel);

                // Record time so we can display “last updated X seconds ago”
                self.last_update = Instant::now();
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


