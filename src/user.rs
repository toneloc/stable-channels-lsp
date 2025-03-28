// src/user.rs
use eframe::{egui, App, Frame};
use ldk_node::{
    bitcoin::secp256k1::PublicKey,
    lightning::ln::msgs::SocketAddress,
    lightning_invoice::Bolt11Invoice,
};
use lightning::util::wakers::Sleeper;
use ureq::Agent;
use std::str::FromStr;
use std::time::{Duration, Instant};
use image::{GrayImage, Luma};
use qrcode::{QrCode, Color};
use egui::TextureOptions;

use crate::base::AppState;
use crate::price_feeds::get_latest_price;
use crate::types::*;

// Configuration constants
const USER_DATA_DIR: &str = "data/user";
const USER_NODE_ALIAS: &str = "user";
const USER_PORT: u16 = 9736;
const DEFAULT_LSP_PUBKEY: &str = "028d1fe921fc54dd6a9b7d86a3f031c3d84212493f48e754b6c6d99107f339e7e5";
const DEFAULT_LSP_ADDRESS: &str = "127.0.0.1:9737";
const DEFAULT_LSP_AUTH: &str = "00000000000000000000000000000000";
const EXPECTED_USD: f64 = 8.0;
const DEFAULT_GATEWAY_PUBKEY: &str = "02eb14024272877a1afc404b637a87dee29cebfa1765c74677754d16dae3052c78";

#[cfg(feature = "user")]
pub struct UserApp {
    // Base app state
    base: AppState,
    
    // User-specific fields
    show_onboarding: bool,
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
        
        // Configure LSP settings before creating base
        let lsp_pubkey = PublicKey::from_str(DEFAULT_LSP_PUBKEY).unwrap();
        
        // Initialize the base AppState
        let mut base = AppState::new(
            USER_DATA_DIR, 
            USER_NODE_ALIAS, 
            USER_PORT,
            Some((DEFAULT_LSP_ADDRESS.to_string(), lsp_pubkey))
        );

        // Additional setup specific to the user node
        // Connect to LSP if available
        if let key = lsp_pubkey.clone() {
            println!("here");
            if let Ok(socket_addr) = DEFAULT_LSP_ADDRESS.parse::<std::net::SocketAddr>() {
                println!("Setting LSP with address: {} and pubkey: {}", 
                         DEFAULT_LSP_ADDRESS, key);
                // This would ideally be handled before node creation, but we can keep it here for now
            }
        }
        
        // Connect to gateway node
        if let Ok(pubkey) = PublicKey::from_str(DEFAULT_GATEWAY_PUBKEY) {
            let socket_addr = SocketAddress::from_str("127.0.0.1:9735").unwrap(); 
            if let Err(e) = base.node.connect(pubkey, socket_addr, true) {
                println!("Failed to connect to gateway: {}", e);
            }
        }

        if let Ok(pubkey) = PublicKey::from_str(DEFAULT_LSP_ADDRESS) {
            let socket_addr = SocketAddress::from_str("127.0.0.1:9737").unwrap(); 
            if let Err(e) = base.node.connect(pubkey, socket_addr, true) {
                println!("Failed to connect to lsp: {}", e);
            }
            else {
                println!("Hi");
            }
        }
        
        
        // Initialize price (ensure we have a valid price before proceeding)
        let current_price = crate::price_feeds::get_cached_price();
        if current_price <= 0.0 {
            // If cache is empty, try to get a fresh price
            let agent = Agent::new();
            if let Ok(price) = crate::price_feeds::get_latest_price(&agent) {
                base.btc_price = price;
            }
        } else {
            base.btc_price = current_price;
        }
        
        // Create an empty stable channel with default values
        let default_lsp_pubkey = PublicKey::from_str(DEFAULT_LSP_PUBKEY).unwrap_or_else(|_| {
            panic!("Invalid LSP pubkey: {}", DEFAULT_LSP_PUBKEY);
        });
        
        let stable_channel = StableChannel {
            channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes([0u8; 32]),
            counterparty: lsp_pubkey, // user's counteparty is always the LSP
            is_stable_receiver: true,
            expected_usd: USD::from_f64(EXPECTED_USD),
            expected_btc: Bitcoin::from_usd(USD::from_f64(EXPECTED_USD), base.btc_price),
            stable_receiver_btc: Bitcoin::default(),
            stable_receiver_usd: USD::default(),
            stable_provider_btc: Bitcoin::default(),
            stable_provider_usd: USD::default(),
            latest_price: base.btc_price,
            risk_level: 0,
            payment_made: false,
            timestamp: 0,
            formatted_datetime: "2021-06-01 12:00:00".to_string(),
            sc_dir: "/".to_string(),
            prices: "".to_string(),
        };
        
        // Check if we need to show onboarding
        let show_onboarding = base.node.list_channels().is_empty();
        
        let mut app = Self {
            base,
            show_onboarding,
            qr_texture: None,
            waiting_for_payment: false,
            stable_channel,
            is_stable_channel_initialized: true,
            last_stability_check: Instant::now(),
        };
        
        // Initialize stability with the current price
        let current_price = app.base.btc_price;
        crate::stable::check_stability(&app.base.node, &mut app.stable_channel, current_price);
        
        app
    }

    fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
            ldk_node::lightning_invoice::Description::new("Stable Channel JIT payment".to_string()).unwrap()
        );
        
        let result = self.base.node.bolt11_payment().receive_via_jit_channel(
            20_000_000, 
            &description,
            3600, // 1 hour expiry
            Some(10_000_000), // minimum channel size of 10k sats
        );
    
        match result {
            Ok(invoice) => {
                self.base.invoice_result = invoice.to_string();
                
                // Generate QR code
                let code = QrCode::new(&self.base.invoice_result).unwrap();
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
                
                self.base.status_message = "Invoice generated. Pay it to create a JIT channel.".to_string();
                self.waiting_for_payment = true;
            }
            Err(e) => {
                self.base.invoice_result = format!("Error: {e:?}");
                self.base.status_message = format!("Failed to generate invoice: {}", e);
            }
        }
    }
    
    fn process_events(&mut self) {
        // Extends the base poll_events with user-specific event handling
        while let Some(event) = self.base.node.next_event() {
            match event {
                ldk_node::Event::ChannelReady { channel_id, .. } => {
                    self.base.status_message = format!("Channel {} is now ready", channel_id);
                    self.show_onboarding = false;
                    self.waiting_for_payment = false; 
                }
                
                ldk_node::Event::PaymentReceived { amount_msat, .. } => {
                    self.base.status_message = format!("Received payment of {} msats", amount_msat);
                }
                
                ldk_node::Event::ChannelClosed { channel_id, .. } => {
                    self.base.status_message = format!("Channel {} has been closed", channel_id);
                    // If no channels left, go back to onboarding
                    if self.base.node.list_channels().is_empty() {
                        self.show_onboarding = true;
                        self.waiting_for_payment = false;
                    }
                }
                
                _ => {} // Ignore other events for now
            }
            self.base.node.event_handled(); // Mark event as handled
        }
    }

    // The "waiting for payment" screen with the JIT invoice
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
                    egui::TextEdit::multiline(&mut self.base.invoice_result)
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
                        o.copied_text = self.base.invoice_result.clone();
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

    // The "onboarding" screen that prompts the user to stabilize
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
                    self.base.status_message = "Getting JIT channel invoice...".to_string();
                    self.get_jit_invoice(ctx);
                }
                
                // Show status message if there is one
                if !self.base.status_message.is_empty() {
                    ui.add_space(20.0);
                    ui.label(self.base.status_message.clone());
                }
                
                // Show node ID
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.label("Node ID: ");
                    let node_id = self.base.node.node_id().to_string();
                    let node_id_short = format!("{}...{}", &node_id[0..10], &node_id[node_id.len()-10..]);
                    ui.monospace(node_id_short);
                    
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = node_id);
                    }
                });
            });
        });
    }

    // The main screen once the user has a channel
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
                    ui.label(format!("${:.2}", self.base.btc_price));
                    ui.add_space(20.0);
    
                    let last_updated = self.base.last_update.elapsed().as_secs();
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
                    
                    let channels = self.base.node.list_channels();
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
                if !self.base.status_message.is_empty() {
                    ui.label(self.base.status_message.clone());
                    ui.add_space(10.0);
                }

                // Simple invoice generator UI
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

                // Pay Invoice
                ui.group(|ui| {
                    ui.label("Pay Invoice");
                    ui.text_edit_multiline(&mut self.base.invoice_to_pay);
                    if ui.button("Pay Invoice").clicked() {
                        self.base.pay_invoice();
                    }
                });
                
                // Action buttons
                if ui.button("Create New Channel").clicked() {
                    self.show_onboarding = true;
                }
                
                if ui.button("Get On-chain Address").clicked() {
                    self.base.get_address();
                }
            });
        });
    }
}

#[cfg(feature = "user")]
impl App for UserApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Process events
        self.process_events();
        
        if self.last_stability_check.elapsed() > Duration::from_secs(30) {
            // Get the current cached price (this will trigger an update if needed)
            let current_price = crate::price_feeds::get_cached_price();
            
            // Skip stability check if we don't have a valid price yet
            if current_price > 0.0 {
                // Update the app's price
                self.base.btc_price = current_price;
                
                // Run stability check with the current price
                crate::stable::check_stability(&self.base.node, &mut self.stable_channel, current_price);
            } else {
                println!("Skipping stability check: No valid price available");
            }
            
            // Always update this timestamp
            self.last_stability_check = Instant::now();
            
            // Update the last update timestamp if we got a valid price
            if current_price > 0.0 {
                self.base.last_update = Instant::now();
            }
        }

        // Now, actually show the UI:
        if self.waiting_for_payment {
            self.show_waiting_for_payment_screen(ctx);
        } else if self.show_onboarding {
            self.show_onboarding_screen(ctx);
        } else {
            self.show_main_screen(ctx);
        }

        // Request a repaint
        ctx.request_repaint_after(Duration::from_millis(100));
        
        // Request a repaint to keep the UI responsive
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
            // Create the app with initialized LDK node
            Ok(Box::new(UserApp::new()))
        }),
    ).unwrap_or_else(|e| {
        eprintln!("Error starting the application: {:?}", e);
    });
}