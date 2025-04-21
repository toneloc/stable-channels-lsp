// src/user.rs
use eframe::{egui, App, Frame};
use ldk_node::bitcoin::Network;
use ldk_node::lightning_invoice::Bolt11Invoice;
use ldk_node::{Builder, Node};
use ldk_node::{
    bitcoin::secp256k1::PublicKey,
    lightning::ln::msgs::SocketAddress,
};
use ureq::Agent;
// use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use image::{GrayImage, Luma};
use qrcode::{QrCode, Color};
use egui::TextureOptions;

use crate::stable::update_balances;
use crate::types::*;
use crate::price_feeds::{get_cached_price, get_latest_price};
use crate::stable;

const USER_DATA_DIR: &str = "data/user";
const USER_NODE_ALIAS: &str = "user";
const USER_PORT: u16 = 9736;
const DEFAULT_LSP_PUBKEY: &str = "02d3db21cb7de67f543c6bfa576e5122109325e308013d11cdfda18c6ce4f91a89";
const DEFAULT_LSP_ADDRESS: &str = "54.210.112.22:9737";
const EXPECTED_USD: f64 = 8.0;
const DEFAULT_GATEWAY_PUBKEY: &str = "03809c504e5b078daeaa0052a1b10bd3f48f4d6547fcf7d689965de299b76988f2";
const DEFAULT_NETWORK: &str = "signet";
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";

#[cfg(feature = "user")]
pub struct UserApp {
    pub node: Arc<Node>,
    pub status_message: String,
    pub btc_price: f64,
    show_onboarding: bool,
    qr_texture: Option<egui::TextureHandle>,
    waiting_for_payment: bool,
    stable_channel: Arc<Mutex<StableChannel>>,
    background_started: bool,

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

#[cfg(feature = "user")]
impl UserApp {
    pub fn new() -> Self {
        println!("Initializing user node...");

        let user_data_dir = USER_DATA_DIR;
        let lsp_pubkey = PublicKey::from_str(DEFAULT_LSP_PUBKEY).unwrap();

        let mut builder = Builder::new();
        builder.set_network(Network::Signet);
        builder.set_chain_source_esplora(DEFAULT_CHAIN_SOURCE_URL.to_string(), None);
        builder.set_storage_dir_path(user_data_dir.to_string());
        builder.set_listening_addresses(vec![format!("127.0.0.1:{}", USER_PORT).parse().unwrap()]).unwrap();
        builder.set_node_alias(USER_NODE_ALIAS.to_string());

        builder.set_liquidity_source_lsps2(
            lsp_pubkey,
            SocketAddress::from_str(DEFAULT_LSP_ADDRESS).unwrap(),
            None,
        );
        builder.set_liquidity_source_lsps1(
            lsp_pubkey,
            SocketAddress::from_str(DEFAULT_LSP_ADDRESS).unwrap(),
            None,
        );

        let node = Arc::new(builder.build().expect("Failed to build node"));
        node.start().expect("Failed to start node");
        println!("User node started: {}", node.node_id());

        let mut btc_price = crate::price_feeds::get_cached_price();
        if btc_price <= 0.0 {
            if let Ok(price) = get_latest_price(&ureq::Agent::new()) {
                btc_price = price;
            }
        }

        let sc_init = StableChannel {
            channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes([0; 32]),
            counterparty: lsp_pubkey,
            is_stable_receiver: true,
            expected_usd: USD::from_f64(EXPECTED_USD),
            expected_btc: Bitcoin::from_usd(USD::from_f64(EXPECTED_USD), btc_price),
            stable_receiver_btc: Bitcoin::default(),
            stable_receiver_usd: USD::default(),
            stable_provider_btc: Bitcoin::default(),
            stable_provider_usd: USD::default(),
            latest_price: btc_price,
            risk_level: 0,
            payment_made: false,
            timestamp: 0,
            formatted_datetime: "2021-06-01 12:00:00".to_string(),
            sc_dir: "/".to_string(),
            prices: String::new(),
        };
        let stable_channel = Arc::new(Mutex::new(sc_init));

        let show_onboarding = node.list_channels().is_empty();

        let app = Self {
            node: Arc::clone(&node),
            status_message: String::new(),
            invoice_result: String::new(),
            show_onboarding,
            qr_texture: None,
            waiting_for_payment: false,
            stable_channel: Arc::clone(&stable_channel),
            background_started: false,
            btc_price,
            invoice_amount: "0".to_string(),        
            invoice_to_pay: String::new(),
            on_chain_address: String::new(),
            on_chain_amount: "0".to_string(),  
            lightning_balance_btc: 0.0,
            onchain_balance_btc: 0.0,
            lightning_balance_usd: 0.0,
            onchain_balance_usd: 0.0,
            total_balance_btc: 0.0,
            total_balance_usd: 0.0,
        };

        {
            let mut sc = app.stable_channel.lock().unwrap();
            stable::check_stability(&app.node, &mut sc, btc_price);
            update_balances(&app.node, &mut sc);
        }

        let node_arc = Arc::clone(&app.node);
        let sc_arc = Arc::clone(&app.stable_channel);

        std::thread::spawn(move || {
            use std::{thread::sleep, time::{Duration, SystemTime, UNIX_EPOCH}};

            fn current_unix_time() -> i64 {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .try_into()
                    .unwrap_or(0)
            }

            loop {
                let price = match get_latest_price(&ureq::Agent::new()) {
                    Ok(p) if p > 0.0 => p,
                    _ => crate::price_feeds::get_cached_price()
                };

                if price > 0.0 && !node_arc.list_channels().is_empty() {
                    if let Ok(mut sc) = sc_arc.lock() {
                        stable::check_stability(&*node_arc, &mut sc, price);
                        update_balances(&*node_arc, &mut sc);

                        sc.latest_price = price;
                        sc.timestamp = current_unix_time();
                    }
                }
                sleep(Duration::from_secs(30));
            }
        });

        app
    }
    // fn get_app_data_dir(component: &str) -> PathBuf {
    //     let mut path = dirs::data_local_dir()
    //         .unwrap_or_else(|| PathBuf::from("./data"))
    //         .join("com.stablechannels");
        
    //     if !component.is_empty() {
    //         path = path.join(component);
    //     }
        
    //     // Ensure the directory exists
    //     std::fs::create_dir_all(&path).unwrap_or_else(|e| {
    //         eprintln!("Warning: Failed to create data directory: {}", e);
    //     });
        
    //     path
    // }
  
    fn start_background_if_needed(&mut self) {
        if self.background_started {
            return;
        }

        let node_arc = Arc::clone(&self.node);
        let sc_arc = Arc::clone(&self.stable_channel);

        std::thread::spawn(move || {
            loop {
                // Always try to get the latest price first
                let price = match crate::price_feeds::get_latest_price(&ureq::Agent::new()) {
                    Ok(p) if p > 0.0 => p,
                    _ => crate::price_feeds::get_cached_price()
                };

                // Only proceed if we have a valid price and active channels
                if price > 0.0 && !node_arc.list_channels().is_empty() {
                    if let Ok(mut sc) = sc_arc.lock() {
                        crate::stable::check_stability(&*node_arc, &mut sc, price);
                        crate::stable::update_balances(&*node_arc, &mut sc);
                    }
                }

                // Sleep between checks, but be ready to interrupt if needed
                std::thread::sleep(Duration::from_secs(30));
            }
        });

        self.background_started = true;
    }

        fn get_jit_invoice(&mut self, ctx: &egui::Context) {
        let latest_price = {
            let sc = self.stable_channel.lock().unwrap();
            sc.latest_price
        };
        let description = ldk_node::lightning_invoice::Bolt11InvoiceDescription::Direct(
            ldk_node::lightning_invoice::Description::new(
                "Stable Channel JIT payment".to_string(),
            )
            .unwrap(),
        );
        let result = self.node.bolt11_payment().receive_via_jit_channel(
            USD::to_msats(USD::from_f64(EXPECTED_USD), latest_price),
            &description,
            3600,
            Some(10_000_000),
        );
        match result {
            Ok(invoice) => {
                self.invoice_result = invoice.to_string();
                let code = QrCode::new(&self.invoice_result).unwrap();
                let bits = code.to_colors();
                let width = code.width();
                let scale = 4;
                let mut imgbuf =
                    GrayImage::new((width * scale) as u32, (width * scale) as u32);
                for y in 0..width {
                    for x in 0..width {
                        let color = if bits[y * width + x] == Color::Dark {
                            0
                        } else {
                            255
                        };
                        for dy in 0..scale {
                            for dx in 0..scale {
                                imgbuf.put_pixel(
                                    (x * scale + dx) as u32,
                                    (y * scale + dy) as u32,
                                    Luma([color]),
                                );
                            }
                        }
                    }
                }
                let (w, h) = (imgbuf.width() as usize, imgbuf.height() as usize);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for p in imgbuf.pixels() {
                    let lum = p[0];
                    rgba.extend_from_slice(&[lum, lum, lum, 255]);
                }
                let tex = ctx.load_texture(
                    "qr_code",
                    egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
                    TextureOptions::LINEAR,
                );
                self.qr_texture = Some(tex);
                self.status_message =
                    "Invoice generated. Pay it to create a JIT channel.".to_string();
                self.waiting_for_payment = true;
            }
            Err(e) => {
                self.invoice_result = format!("Error: {e:?}");
                self.status_message = format!("Failed to generate invoice: {}", e);
            }
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

    // fn get_lsps1_channel(&mut self) {
    //     let lsp_balance_sat = 10_000;
    //     let client_balance_sat = 10_000;
    //     let lsps1 = self.node.lsps1_liquidity();
    //     match lsps1.request_channel(lsp_balance_sat, client_balance_sat, 2016, false) {
    //         Ok(status) => {
    //             self.status_message =
    //                 format!("LSPS1 channel order initiated! Status: {status:?}");
    //         }
    //         Err(e) => {
    //             self.status_message = format!("LSPS1 channel request failed: {e:?}");
    //         }
    //     }
    // }

    fn process_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                ldk_node::Event::ChannelReady { channel_id, .. } => {
                    self.status_message =
                        format!("Channel {channel_id} is now ready");
                    self.show_onboarding = false;
                    self.waiting_for_payment = false;
                }
                ldk_node::Event::PaymentReceived { amount_msat, .. } => {
                    self.status_message = format!("Received payment of {} msats", amount_msat);
                    let mut sc = self.stable_channel.lock().unwrap();
                    update_balances(&self.node, &mut sc);
                    self.show_onboarding = false;
                    self.waiting_for_payment = false;
                }
                ldk_node::Event::PaymentSuccessful { payment_id: _, payment_hash, payment_preimage: _, fee_paid_msat: _ } => {
                    self.status_message = format!("Sent payment {}", payment_hash);
                    let mut sc = self.stable_channel.lock().unwrap();
                    update_balances(&self.node, &mut sc);
                }
                ldk_node::Event::ChannelClosed { channel_id, .. } => {
                    self.status_message =
                        format!("Channel {channel_id} has been closed");
                    if self.node.list_channels().is_empty() {
                        self.show_onboarding = true;
                        self.waiting_for_payment = false;
                    }
                }
                _ => {}
            }
            let _ = self.node.event_handled();
        }
    }

    fn show_waiting_for_payment_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("Send yourself bitcoin to make it stable.")
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
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Copy Invoice")
                                .color(egui::Color32::BLACK)
                                .size(16.0),
                        )
                        .min_size(egui::vec2(120.0, 36.0))
                        .fill(egui::Color32::from_gray(220))
                        .rounding(6.0),
                    )
                    .clicked()
                {
                    ui.output_mut(|o| o.copied_text = self.invoice_result.clone());
                }
                ui.add_space(5.0);
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Back")
                                .color(egui::Color32::BLACK)
                                .size(16.0),
                        )
                        .min_size(egui::vec2(120.0, 36.0))
                        .fill(egui::Color32::from_gray(220))
                        .rounding(6.0),
                    )
                    .clicked()
                {
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
                ui.heading(
                    egui::RichText::new("Step 1: Get a Lightning invoice ⚡")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new(r#"Press the \"Make stable\" button below."#)
                        .color(egui::Color32::GRAY),
                );
                ui.add_space(20.0);
                ui.heading(
                    egui::RichText::new("Step 2: Send yourself bitcoin 💸")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Over Lightning, from an app or an exchange.")
                        .color(egui::Color32::GRAY),
                );
                ui.add_space(20.0);
                ui.heading(
                    egui::RichText::new("Step 3: Stable channel created 🔧")
                        .color(egui::Color32::WHITE),
                );
                ui.label(
                    egui::RichText::new("Self-custody. Your keys, your coins.")
                        .color(egui::Color32::GRAY),
                );
                ui.add_space(50.0);
                let subtle_orange =
                    egui::Color32::from_rgba_premultiplied(247, 147, 26, 200);
                let btn = egui::Button::new(
                    egui::RichText::new("Make stable")
                        .color(egui::Color32::WHITE)
                        .strong()
                        .size(18.0),
                )
                .min_size(egui::vec2(200.0, 55.0))
                .fill(subtle_orange)
                .rounding(8.0);
                if ui.add(btn).clicked() {
                    self.status_message =
                        "Getting JIT channel invoice...".to_string();
                    self.get_jit_invoice(ctx);
                }
                if !self.status_message.is_empty() {
                    ui.add_space(20.0);
                    ui.label(self.status_message.clone());
                }
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.label("Node ID: ");
                    let node_id = self.node.node_id().to_string();
                    let node_id_short = format!(
                        "{}...{}",
                        &node_id[0..10],
                        &node_id[node_id.len() - 10..]
                    );
                    ui.monospace(node_id_short);
                    if ui.small_button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = node_id);
                    }
                });
            });
        });
    }

    fn show_main_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(30.0);
                    ui.group(|ui| {
                        ui.add_space(20.0);
                        ui.heading("Your Stable Balance");
                        let sc = self.stable_channel.lock().unwrap();
                        let stable_btc = if sc.is_stable_receiver {
                            sc.stable_receiver_btc
                        } else {
                            sc.stable_provider_btc
                        };
                        let stable_usd = if sc.is_stable_receiver {
                            sc.stable_receiver_usd
                        } else {
                            sc.stable_provider_usd
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("{}", stable_usd))
                                    .size(36.0)
                                    .strong(),
                            ),
                        );
                        ui.label(format!("Agreed Peg USD: {}", sc.expected_usd));
                        ui.label(format!("Bitcoin: {:.8}", stable_btc));
                        ui.add_space(20.0);
                    });
                    ui.add_space(20.0);
                    ui.group(|ui| {
                        let sc = self.stable_channel.lock().unwrap();
                        ui.add_space(20.0);
                        ui.heading("Bitcoin Price");
                        ui.label(format!("${:.2}", sc.latest_price));
                        ui.add_space(20.0);

                        let last_updated = match SystemTime::now().duration_since(UNIX_EPOCH + std::time::Duration::from_secs(sc.timestamp as u64)) {
                            Ok(duration) => duration.as_secs(),
                            Err(_) => 0,
                        };                        
                        ui.add_space(5.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Last updated: {}s ago",
                                last_updated
                            ))
                            .size(12.0)
                            .color(egui::Color32::GRAY),
                        );
                    });
                    ui.add_space(20.0);
                    ui.group(|ui| {
                        ui.heading("Lightning Channels");
                        ui.add_space(5.0);
                        let channels = self.node.list_channels();
                        if channels.is_empty() {
                            ui.label("No channels found.");
                        } else {
                            for ch in channels {
                                ui.label(format!(
                                    "Channel: {} - {} sats",
                                    ch.channel_id, ch.channel_value_sats
                                ));
                            }
                        }
                    });
                    ui.add_space(20.0);
                    if !self.status_message.is_empty() {
                        ui.label(self.status_message.clone());
                        ui.add_space(10.0);
                    }
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
                                ui.output_mut(|o| {
                                    o.copied_text = self.invoice_result.clone()
                                });
                            }
                        }
                    });
                    ui.group(|ui| {
                        ui.label("Pay Invoice");
                        ui.text_edit_multiline(&mut self.invoice_to_pay);
                        if ui.button("Pay Invoice").clicked() {
                            self.pay_invoice();
                        }
                    });
                    if ui.button("Create New Channel").clicked() {
                        self.show_onboarding = true;
                    }
                    if ui.button("Get On-chain Address").clicked() {
                        self.get_address();
                    }
                });
            });
        });
    }
}

#[cfg(feature = "user")]
impl App for UserApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.process_events();
        self.start_background_if_needed();
        if self.waiting_for_payment {
            self.show_waiting_for_payment_screen(ctx);
        } else if self.show_onboarding {
            self.show_onboarding_screen(ctx);
        } else {
            self.show_main_screen(ctx);
        }
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
        Box::new(|_| Ok(Box::new(UserApp::new()))),
    )
    .unwrap();
}
