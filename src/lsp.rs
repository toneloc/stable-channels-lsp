use eframe::{egui, App, Frame};
use ldk_node::{
    bitcoin::{Network, Address, secp256k1::PublicKey},
    lightning_invoice::{Bolt11Invoice, Description, Bolt11InvoiceDescription},
    lightning::ln::{msgs::SocketAddress},
    config::ChannelConfig,
    Builder, Node, Event, liquidity::LSPS2ServiceConfig
};
use std::time::{Duration, Instant};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use serde::{Serialize, Deserialize};
use std::fs;
use hex;

use crate::types::*;
use crate::stable;
use crate::price_feeds::get_cached_price;

const LSP_DATA_DIR: &str = "data/lsp";
const LSP_NODE_ALIAS: &str = "lsp";
const LSP_PORT: u16 = 9737;

const EXCHANGE_DATA_DIR: &str = "data/exchange";
const EXCHANGE_NODE_ALIAS: &str = "exchange";
const EXCHANGE_PORT: u16 = 9735;

const DEFAULT_NETWORK: &str = "signet";
const DEFAULT_CHAIN_SOURCE_URL: &str = "https://mutinynet.com/api/";
const EXPECTED_USD: f64 = 15.0;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct StableChannelEntry {
    channel_id: String,
    expected_usd: f64,
    native_btc: f64,
}

#[cfg(feature = "lsp")]
pub struct LspApp {
    node: Arc<Node>,
    btc_price: f64,
    status_message: String,
    last_update: Instant,
    last_stability_check: Instant,
    lightning_balance_btc: f64,
    onchain_balance_btc: f64,
    lightning_balance_usd: f64,
    onchain_balance_usd: f64,
    total_balance_btc: f64,
    total_balance_usd: f64,
    invoice_amount: String,
    invoice_result: String,
    invoice_to_pay: String,
    on_chain_address: String,
    on_chain_amount: String,
    channel_id_to_close: String,
    stable_channels: Vec<StableChannel>,
    selected_channel_id: String,
    stable_channel_amount: String,
    open_channel_node_id: String,
    open_channel_address: String,
    open_channel_amount: String,
    channel_info: String,
}

#[cfg(feature = "lsp")]
impl LspApp {
    pub fn new_with_mode(mode: &str) -> Self {
        let (data_dir, node_alias, port) = match mode.to_lowercase().as_str() {
            "exchange" => (EXCHANGE_DATA_DIR, EXCHANGE_NODE_ALIAS, EXCHANGE_PORT),
            "lsp" => (LSP_DATA_DIR, LSP_NODE_ALIAS, LSP_PORT),
            _ => panic!("Invalid mode"),
        };

        let mut builder = Builder::new();

        let network = match DEFAULT_NETWORK.to_lowercase().as_str() {
            "signet" => Network::Signet,
            "testnet" => Network::Testnet,
            "bitcoin" => Network::Bitcoin,
            _ => {
                println!("Warning: Unknown network in config, defaulting to Signet");
                Network::Signet
            }
        };

        builder.set_network(network);
        builder.set_chain_source_esplora(DEFAULT_CHAIN_SOURCE_URL.to_string(), None);
        builder.set_storage_dir_path(data_dir.to_string());

        let listen_addr = format!("127.0.0.1:{}", port).parse().unwrap();
        builder.set_listening_addresses(vec![listen_addr]).unwrap();
        let _ = builder.set_node_alias(node_alias.to_string()).ok();

        if node_alias == LSP_NODE_ALIAS {
            let service_config = LSPS2ServiceConfig {
                require_token: None,
                advertise_service: true,
                channel_opening_fee_ppm: 0,
                channel_over_provisioning_ppm: 1_000_000,
                min_channel_opening_fee_msat: 0,
                min_channel_lifetime: 100,
                max_client_to_self_delay: 1024,
                min_payment_size_msat: 0,
                max_payment_size_msat: 100_000_000_000,
            };
            builder.set_liquidity_provider_lsps2(service_config);
        }

        let node = Arc::new(builder.build().expect("Failed to build node"));
        node.start().expect("Failed to start node");

        let btc_price = get_cached_price();

        let mut app = Self {
            node,
            btc_price,
            status_message: String::new(),
            last_update: Instant::now(),
            last_stability_check: Instant::now(),
            lightning_balance_btc: 0.0,
            onchain_balance_btc: 0.0,
            lightning_balance_usd: 0.0,
            onchain_balance_usd: 0.0,
            total_balance_btc: 0.0,
            total_balance_usd: 0.0,
            invoice_amount: "1000".into(),
            invoice_result: String::new(),
            invoice_to_pay: String::new(),
            on_chain_address: String::new(),
            on_chain_amount: "10000".into(),
            channel_id_to_close: String::new(),
            stable_channels: Vec::new(),
            selected_channel_id: String::new(),
            stable_channel_amount: EXPECTED_USD.to_string(),
            open_channel_node_id: String::new(),
            open_channel_address: "127.0.0.1:9737".into(),
            open_channel_amount: "100000".into(),
            channel_info: String::new(),
        };

        app.update_balances();
        app.update_channel_info();

        if node_alias == LSP_NODE_ALIAS {
            app.load_stable_channels();
        }

        app
    }

    pub fn new() -> Self {
        Self::new_with_mode("lsp")
    }
}

#[cfg(feature = "lsp")]
impl LspApp {
    pub fn update_balances(&mut self) {
        let current_price = get_cached_price();
        if current_price > 0.0 {
            self.btc_price = current_price;
        }

        let balances = self.node.list_balances();
        self.lightning_balance_btc = balances.total_lightning_balance_sats as f64 / 100_000_000.0;
        self.onchain_balance_btc = balances.total_onchain_balance_sats as f64 / 100_000_000.0;
        self.lightning_balance_usd = self.lightning_balance_btc * self.btc_price;
        self.onchain_balance_usd = self.onchain_balance_btc * self.btc_price;
        self.total_balance_btc = self.lightning_balance_btc + self.onchain_balance_btc;
        self.total_balance_usd = self.lightning_balance_usd + self.onchain_balance_usd;
    }

    pub fn check_and_update_stable_channels(&mut self) {
        let current_price = get_cached_price();
        if current_price > 0.0 {
            self.btc_price = current_price;
        }
    
        let mut channels_updated = false;
        for sc in &mut self.stable_channels {
            if !stable::channel_exists(&self.node, &sc.channel_id) {
                continue;
            }
    
            sc.latest_price = current_price;
            stable::check_stability(&self.node, sc, current_price);
    
            if sc.payment_made {
                channels_updated = true;
            }
        }
    
        if channels_updated {
            self.save_stable_channels();
        }
    }

    pub fn poll_events(&mut self) {
        while let Some(event) = self.node.next_event() {
            match event {
                Event::ChannelReady { channel_id, .. } => {
                    self.status_message = format!("Channel {} is now ready", channel_id);
                    self.update_balances();
                }

                Event::PaymentSuccessful { payment_id: _, payment_hash, payment_preimage: _, fee_paid_msat: _ } => {
                    self.status_message = format!("Sent payment {}", payment_hash);
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

                _ => {}
            }
            let _ = self.node.event_handled();
        }
    }

    pub fn generate_invoice(&mut self) -> bool {
        if let Ok(amount) = self.invoice_amount.parse::<u64>() {
            let msats = amount * 1000;
            match self.node.bolt11_payment().receive(
                msats,
                &Bolt11InvoiceDescription::Direct(Description::new("Invoice".to_string()).unwrap()),
                3600,
            ) {
                Ok(invoice) => {
                    self.invoice_result = invoice.to_string();
                    self.status_message = "Invoice generated".to_string();
                    true
                }
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
            Ok(invoice) => match self.node.bolt11_payment().send(&invoice, None) {
                Ok(payment_id) => {
                    self.status_message = format!("Payment sent, ID: {}", payment_id);
                    self.invoice_to_pay.clear();
                    self.update_balances();
                    true
                }
                Err(e) => {
                    self.status_message = format!("Payment error: {}", e);
                    false
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
            }
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
                    Ok(valid_addr) => match self.node.onchain_payment().send_to_address(&valid_addr, amount, None) {
                        Ok(txid) => {
                            self.status_message = format!("Transaction sent: {}", txid);
                            self.update_balances();
                            true
                        }
                        Err(e) => {
                            self.status_message = format!("Transaction error: {}", e);
                            false
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

    pub fn show_balance_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Balances");
            ui.add_space(5.0);

            ui.horizontal(|ui| {
                ui.label("Lightning:");
                ui.monospace(format!("{:.8} BTC", self.lightning_balance_btc));
                ui.monospace(format!("(${:.2})", self.lightning_balance_usd));
            });

            ui.horizontal(|ui| {
                ui.label("On-chain:  ");
                ui.monospace(format!("{:.8} BTC", self.onchain_balance_btc));
                ui.monospace(format!("(${:.2})", self.onchain_balance_usd));
            });

            ui.horizontal(|ui| {
                ui.label("Total:     ");
                ui.strong(format!("{:.8} BTC", self.total_balance_btc));
                ui.strong(format!("(${:.2})", self.total_balance_usd));
            });

            ui.add_space(5.0);
            ui.label(format!(
                "Price: ${:.2} | Updated: {} seconds ago",
                self.btc_price,
                self.last_update.elapsed().as_secs()
            ));
        });
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

    pub fn show_node_info_section(&mut self, ui: &mut egui::Ui, port: u16) {
        ui.group(|ui| {
            ui.label(format!("Node ID: {}", self.node.node_id()));
            ui.label(format!("Listening on: 127.0.0.1:{}", port));
        });
    }

    pub fn show_channels_section(&mut self, ui: &mut egui::Ui, channel_info: &mut String) {
        ui.group(|ui| {
            ui.heading("Lightning Channels");
            if ui.button("Refresh Channel List").clicked() {
                *channel_info = self.update_channel_info();
            }
            ui.text_edit_multiline(channel_info);
        });
    }

    pub fn update_channel_info(&mut self) -> String {
        let channels = self.node.list_channels();
        if channels.is_empty() {
            return "No channels found.".to_string();
        } else {
            let mut info = String::new();
            for (i, channel) in channels.iter().enumerate() {
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
            info
        }
    }

    pub fn open_channel(&mut self) -> bool {
        match PublicKey::from_str(&self.open_channel_node_id) {
            Ok(node_id) => match SocketAddress::from_str(&self.open_channel_address) {
                Ok(net_address) => match self.open_channel_amount.parse::<u64>() {
                    Ok(sats) => {
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
                                self.status_message = format!("Channel opening initiated with {} for {} sats", node_id, sats);
                                true
                            }
                            Err(e) => {
                                self.status_message = format!("Error opening channel: {}", e);
                                false
                            }
                        }
                    }
                    Err(_) => {
                        self.status_message = "Invalid amount format".to_string();
                        false
                    }
                },
                Err(_) => {
                    self.status_message = "Invalid network address format".to_string();
                    false
                }
            },
            Err(_) => {
                self.status_message = "Invalid node ID format".to_string();
                false
            }
        }
    }

    pub fn close_specific_channel(&mut self) {
        if self.channel_id_to_close.is_empty() {
            self.status_message = "Please enter a channel ID to close".to_string();
            return;
        }

        let input = self.channel_id_to_close.trim();
        if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(bytes) = hex::decode(input) {
                for channel in self.node.list_channels() {
                    if channel.channel_id.0.to_vec() == bytes {
                        let result = self.node.close_channel(&channel.user_channel_id, channel.counterparty_node_id);
                        self.status_message = match result {
                            Ok(_) => format!("Closing channel: {}", input),
                            Err(e) => format!("Error closing channel: {}", e),
                        };
                        self.channel_id_to_close.clear();
                        return;
                    }
                }
            }
            self.status_message = "Channel ID not found.".to_string();
        } else {
            for channel in self.node.list_channels() {
                if channel.channel_id.to_string() == input {
                    let result = self.node.close_channel(&channel.user_channel_id, channel.counterparty_node_id);
                    self.status_message = match result {
                        Ok(_) => format!("Closing channel: {}", input),
                        Err(e) => format!("Error closing channel: {}", e),
                    };
                    self.channel_id_to_close.clear();
                    return;
                }
            }
            self.status_message = "Channel not found.".to_string();
        }
    }

    pub fn designate_stable_channel(&mut self) {
        if self.selected_channel_id.is_empty() {
            self.status_message = "Please select a channel ID".to_string();
            return;
        }

        let amount = match self.stable_channel_amount.parse::<f64>() {
            Ok(val) => val,
            Err(_) => {
                self.status_message = "Invalid amount format".to_string();
                return;
            }
        };

        let channel_id_str = self.selected_channel_id.trim().to_string();

        for channel in self.node.list_channels() {
            if channel.channel_id.to_string() == channel_id_str {
                let expected_usd = USD::from_f64(amount);
                let expected_btc = Bitcoin::from_usd(expected_usd, self.btc_price);

                let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
                let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
                let their_balance_sats = channel.channel_value_sats - our_balance_sats;

                let stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
                let stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
                let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, self.btc_price);
                let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, self.btc_price);

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
                    latest_price: self.btc_price,
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

                self.save_stable_channels();

                self.status_message = format!(
                    "Channel {} designated as stable with target ${}",
                    channel_id_str, amount
                );
                self.selected_channel_id.clear();
                self.stable_channel_amount = EXPECTED_USD.to_string();
                return;
            }
        }

        self.status_message = format!("No channel found matching: {}", self.selected_channel_id);
    }

    pub fn show_lsp_screen(&mut self, ctx: &egui::Context) {
        let mut channel_info = self.update_channel_info();

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Lightning Service Provider");
                ui.add_space(10.0);

                self.show_node_info_section(ui, LSP_PORT);
                ui.add_space(10.0);
                self.show_balance_section(ui);
                ui.add_space(10.0);

                ui.group(|ui| {
                    ui.heading("Open Channel");
                    ui.horizontal(|ui| {
                        ui.label("Node ID:");
                        ui.text_edit_singleline(&mut self.open_channel_node_id);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Net Address:");
                        ui.text_edit_singleline(&mut self.open_channel_address);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Amount (sats):");
                        ui.text_edit_singleline(&mut self.open_channel_amount);
                    });
                    if ui.button("Open Channel").clicked() {
                        if self.open_channel() {
                            self.open_channel_node_id.clear();
                            self.open_channel_amount = "100000".to_string();
                        }
                    }
                });

                ui.add_space(10.0);

                ui.group(|ui| {
                    ui.heading("Stable Channels");
                    if self.stable_channels.is_empty() {
                        ui.label("No stable channels configured");
                    } else {
                        for (i, sc) in self.stable_channels.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}. Channel: {}", i + 1, sc.channel_id));
                                ui.label(format!("Target: ${:.2}", sc.expected_usd.0));
                            });
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

                    ui.label("Designate Stable Channel:");
                    ui.horizontal(|ui| {
                        ui.label("Channel ID:");
                        ui.text_edit_singleline(&mut self.selected_channel_id);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Target USD amount:");
                        ui.text_edit_singleline(&mut self.stable_channel_amount);
                    });
                    if ui.button("Designate as Stable").clicked() {
                        self.designate_stable_channel();
                    }
                });

                ui.add_space(10.0);
                self.show_invoice_section(ui);
                ui.add_space(10.0);
                self.show_pay_invoice_section(ui);
                ui.add_space(10.0);
                self.show_onchain_address_section(ui);
                ui.add_space(10.0);
                self.show_onchain_send_section(ui);
                ui.add_space(10.0);

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
                self.show_channels_section(ui, &mut channel_info);
                ui.add_space(10.0);

                if !self.status_message.is_empty() {
                    ui.label(self.status_message.clone());
                }
            });
        });
    }

    pub fn save_stable_channels(&mut self) {
        let entries: Vec<StableChannelEntry> = self.stable_channels.iter().map(|sc| StableChannelEntry {
            channel_id: sc.channel_id.to_string(),
            expected_usd: sc.expected_usd.0,
            native_btc: sc.expected_btc.to_btc(),
        }).collect();

        let file_path = Path::new(LSP_DATA_DIR).join("stablechannels.json");

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|e| {
                eprintln!("Failed to create directory: {}", e);
            });
        }

        match serde_json::to_string_pretty(&entries) {
            Ok(json) => {
                match fs::write(&file_path, json) {
                    Ok(_) => {
                        println!("Saved stable channels to {}", file_path.display());
                        self.status_message = "Stable channels saved successfully".to_string();
                    }
                    Err(e) => {
                        eprintln!("Error writing stable channels file: {}", e);
                        self.status_message = format!("Failed to save stable channels: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error serializing stable channels: {}", e);
                self.status_message = format!("Failed to serialize stable channels: {}", e);
            }
        }
    }

    pub fn load_stable_channels(&mut self) {
        let file_path = Path::new(LSP_DATA_DIR).join("stablechannels.json");

        if !file_path.exists() {
            println!("No existing stable channels file found.");
            return;
        }

        match fs::read_to_string(&file_path) {
            Ok(contents) => {
                match serde_json::from_str::<Vec<StableChannelEntry>>(&contents) {
                    Ok(entries) => {
                        self.stable_channels.clear();

                        for entry in entries {
                            for channel in self.node.list_channels() {
                                if channel.channel_id.to_string() == entry.channel_id {
                                    let unspendable = channel.unspendable_punishment_reserve.unwrap_or(0);
                                    let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable;
                                    let their_balance_sats = channel.channel_value_sats - our_balance_sats;

                                    let stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
                                    let stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
                                    let stable_provider_usd = USD::from_bitcoin(stable_provider_btc, self.btc_price);
                                    let stable_receiver_usd = USD::from_bitcoin(stable_receiver_btc, self.btc_price);

                                    let stable_channel = StableChannel {
                                        channel_id: channel.channel_id,
                                        counterparty: channel.counterparty_node_id,
                                        is_stable_receiver: false,
                                        expected_usd: USD::from_f64(entry.expected_usd),
                                        expected_btc: Bitcoin::from_btc(entry.native_btc),
                                        stable_receiver_btc,
                                        stable_receiver_usd,
                                        stable_provider_btc,
                                        stable_provider_usd,
                                        latest_price: self.btc_price,
                                        risk_level: 0,
                                        payment_made: false,
                                        timestamp: 0,
                                        formatted_datetime: "".to_string(),
                                        sc_dir: LSP_DATA_DIR.to_string(),
                                        prices: "".to_string(),
                                    };

                                    self.stable_channels.push(stable_channel);
                                    break;
                                }
                            }
                        }

                        println!("Loaded {} stable channels", self.stable_channels.len());
                        self.status_message = format!("Loaded {} stable channels", self.stable_channels.len());
                    }
                    Err(e) => {
                        eprintln!("Error parsing stable channels file: {}", e);
                        self.status_message = format!("Failed to parse stable channels: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Error reading stable channels file: {}", e);
                self.status_message = format!("Failed to read stable channels file: {}", e);
            }
        }
    }
}

#[cfg(feature = "lsp")]
impl App for LspApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.poll_events();

        if self.last_update.elapsed() > Duration::from_secs(30) {
            let current_price = get_cached_price();
            if current_price > 0.0 {
                self.btc_price = current_price;
            }
            self.update_balances();
            self.last_update = Instant::now();
        }

        if self.last_stability_check.elapsed() > Duration::from_secs(30) {
            self.check_and_update_stable_channels();
            self.last_stability_check = Instant::now();
        }

        self.show_lsp_screen(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(feature = "lsp")]
pub fn run() {
    println!("Starting LSP Interface...");

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([500.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Lightning Service Provider",
        native_options,
        Box::new(|_cc| Ok(Box::new(LspApp::new()))),
    )
    .unwrap_or_else(|e| {
        eprintln!("Error starting the application: {:?}", e);
    });
}