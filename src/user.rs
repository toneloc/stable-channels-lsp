// src/user.rs
use eframe::{egui, App, Frame};
use ldk_node::Builder;
use ldk_node::{
    bitcoin::secp256k1::PublicKey,
    lightning::ln::msgs::SocketAddress,
};
use ureq::Agent;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use image::{GrayImage, Luma};
use qrcode::{QrCode, Color};
use egui::TextureOptions;

use crate::base::AppState;
use crate::stable::update_balances;
use crate::types::*;

const USER_DATA_DIR: &str = "data/user";
const USER_NODE_ALIAS: &str = "user";
const USER_PORT: u16 = 9736;
const DEFAULT_LSP_PUBKEY: &str =
    "036f452075412c2d4c12864200ef8a75341c2b4e7d19a5ed55835fe5a46a10e5ae";
const DEFAULT_LSP_ADDRESS: &str = "127.0.0.1:9737";
const EXPECTED_USD: f64 = 8.0;
const DEFAULT_GATEWAY_PUBKEY: &str =
    "03809c504e5b078daeaa0052a1b10bd3f48f4d6547fcf7d689965de299b76988f2";

#[cfg(feature = "user")]
pub struct UserApp {
    base: AppState,
    show_onboarding: bool,
    qr_texture: Option<egui::TextureHandle>,
    waiting_for_payment: bool,
    stable_channel: Arc<Mutex<StableChannel>>,
    is_stable_channel_initialized: bool,
    last_stability_check: Instant,
    background_started: bool,
}

#[cfg(feature = "user")]
impl UserApp {
    fn new() -> Self {
        println!("Initializing user node...");

        let lsp_pubkey = PublicKey::from_str(DEFAULT_LSP_PUBKEY).unwrap();

        let mut builder = Builder::new();
        println!(
            "Setting LSP with address: {} and pubkey: {}",
            DEFAULT_LSP_ADDRESS, DEFAULT_LSP_PUBKEY
        );
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

        let mut base = AppState::new(builder, USER_DATA_DIR, USER_NODE_ALIAS, USER_PORT);

        if let Ok(pubkey) = PublicKey::from_str(DEFAULT_GATEWAY_PUBKEY) {
            let socket_addr = SocketAddress::from_str("127.0.0.1:9735").unwrap();
            let _ = base.node.connect(pubkey, socket_addr, true);
        }

        if let Ok(pubkey) = PublicKey::from_str(DEFAULT_LSP_ADDRESS) {
            let socket_addr = SocketAddress::from_str("127.0.0.1:9737").unwrap();
            let _ = base.node.connect(pubkey, socket_addr, true);
        }

        if base.btc_price <= 0.0 {
            if let Ok(price) = crate::price_feeds::get_latest_price(&Agent::new()) {
                base.btc_price = price;
            }
        }

        let sc_init = StableChannel {
            channel_id: ldk_node::lightning::ln::types::ChannelId::from_bytes([0; 32]),
            counterparty: lsp_pubkey,
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
            prices: String::new(),
        };
        let stable_channel = Arc::new(Mutex::new(sc_init));

        let show_onboarding = base.node.list_channels().is_empty();

        let mut app = Self {
            base,
            show_onboarding,
            qr_texture: None,
            waiting_for_payment: false,
            stable_channel,
            is_stable_channel_initialized: true,
            last_stability_check: Instant::now(),
            background_started: false,
        };

        {
            let current_price = app.base.btc_price;
            let mut sc = app.stable_channel.lock().unwrap();
            crate::stable::check_stability(&app.base.node, &mut sc, current_price);
            update_balances(&app.base.node, &mut sc);
        }

        let node_arc = Arc::clone(&app.base.node);
        let sc_arc = Arc::clone(&app.stable_channel);

        std::thread::spawn(move || {
            use std::{thread::sleep, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
        
            fn current_unix_time() -> i64 {
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().try_into().unwrap()
            }
        
            let mut last = Instant::now();
        
            loop {
                sleep(Duration::from_secs(1));
        
                if last.elapsed() >= Duration::from_secs(30)
                    && !node_arc.list_channels().is_empty()
                {
                    let price = crate::price_feeds::get_cached_price();
                    if price > 0.0 {
                        let mut sc = sc_arc.lock().unwrap();
                        crate::stable::check_stability(&*node_arc, &mut sc, price);
                        update_balances(&*node_arc, &mut sc);
                        sc.latest_price = price;
                        sc.timestamp = current_unix_time();
                    }
        
                    last = Instant::now();
                }
            }
        });

        app
    }

    fn start_background_if_needed(&mut self) {
        if self.background_started {
            return;
        }
        let node_arc = Arc::clone(&self.base.node);
        let sc_arc = Arc::clone(&self.stable_channel);
        std::thread::spawn(move || {
            use std::{thread::sleep, time::{Duration, Instant}};
            let mut last = Instant::now();
            loop {
                sleep(Duration::from_secs(1));
                if last.elapsed() >= Duration::from_secs(30)
                    && !node_arc.list_channels().is_empty()
                {
                    let price = crate::price_feeds::get_cached_price();
                    if price > 0.0 {
                        let mut sc = sc_arc.lock().unwrap();
                        crate::stable::check_stability(&*node_arc, &mut sc, price);
                        update_balances(&*node_arc, &mut sc);
                    }
                    last = Instant::now();
                }
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
        let result = self.base.node.bolt11_payment().receive_via_jit_channel(
            USD::to_msats(USD::from_f64(EXPECTED_USD), latest_price),
            &description,
            3600,
            Some(10_000_000),
        );
        match result {
            Ok(invoice) => {
                self.base.invoice_result = invoice.to_string();
                let code = QrCode::new(&self.base.invoice_result).unwrap();
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
                self.base.status_message =
                    "Invoice generated. Pay it to create a JIT channel.".to_string();
                self.waiting_for_payment = true;
            }
            Err(e) => {
                self.base.invoice_result = format!("Error: {e:?}");
                self.base.status_message = format!("Failed to generate invoice: {}", e);
            }
        }
    }

    fn get_lsps1_channel(&mut self) {
        let lsp_balance_sat = 10_000;
        let client_balance_sat = 10_000;
        let lsps1 = self.base.node.lsps1_liquidity();
        match lsps1.request_channel(lsp_balance_sat, client_balance_sat, 2016, false) {
            Ok(status) => {
                self.base.status_message =
                    format!("LSPS1 channel order initiated! Status: {status:?}");
            }
            Err(e) => {
                self.base.status_message = format!("LSPS1 channel request failed: {e:?}");
            }
        }
    }

    fn process_events(&mut self) {
        while let Some(event) = self.base.node.next_event() {
            match event {
                ldk_node::Event::ChannelReady { channel_id, .. } => {
                    self.base.status_message =
                        format!("Channel {channel_id} is now ready");
                    self.show_onboarding = false;
                    self.waiting_for_payment = false;
                }
                ldk_node::Event::PaymentReceived { amount_msat, .. } => {
                    self.base.status_message = format!("Received payment of {} msats", amount_msat);
                    let mut sc = self.stable_channel.lock().unwrap();
                    update_balances(&self.base.node, &mut sc);
                    self.show_onboarding = false;
                    self.waiting_for_payment = false;
                }
                ldk_node::Event::PaymentSuccessful { payment_id, payment_hash, payment_preimage, fee_paid_msat } => {
                    self.base.status_message = format!("Sent payment {}", payment_hash);
                    let mut sc = self.stable_channel.lock().unwrap();
                    update_balances(&self.base.node, &mut sc);
                }
                ldk_node::Event::ChannelClosed { channel_id, .. } => {
                    self.base.status_message =
                        format!("Channel {channel_id} has been closed");
                    if self.base.node.list_channels().is_empty() {
                        self.show_onboarding = true;
                        self.waiting_for_payment = false;
                    }
                }
                _ => {}
            }
            self.base.node.event_handled();
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
                    egui::TextEdit::multiline(&mut self.base.invoice_result)
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
                    ui.output_mut(|o| o.copied_text = self.base.invoice_result.clone());
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
                    self.base.status_message =
                        "Getting JIT channel invoice...".to_string();
                    self.get_jit_invoice(ctx);
                }
                if !self.base.status_message.is_empty() {
                    ui.add_space(20.0);
                    ui.label(self.base.status_message.clone());
                }
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.label("Node ID: ");
                    let node_id = self.base.node.node_id().to_string();
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
                        };                        ui.add_space(5.0);
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
                        let channels = self.base.node.list_channels();
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
                    if !self.base.status_message.is_empty() {
                        ui.label(self.base.status_message.clone());
                        ui.add_space(10.0);
                    }
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
                                ui.output_mut(|o| {
                                    o.copied_text = self.base.invoice_result.clone()
                                });
                            }
                        }
                    });
                    ui.group(|ui| {
                        ui.label("Pay Invoice");
                        ui.text_edit_multiline(&mut self.base.invoice_to_pay);
                        if ui.button("Pay Invoice").clicked() {
                            self.base.pay_invoice();
                        }
                    });
                    if ui.button("Create New Channel").clicked() {
                        self.show_onboarding = true;
                    }
                    if ui.button("Get On-chain Address").clicked() {
                        self.base.get_address();
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
