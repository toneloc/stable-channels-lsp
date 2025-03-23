use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::{
    lightning::ln::types::ChannelId, Node,
};
use ureq::Agent;

/// Represents the action to take after a stability check
#[derive(Debug, Clone)]
pub enum StabilityAction {
    /// No action needed, channel is stable enough
    DoNothing,
    /// Wait for payment from counterparty
    Wait,
    /// Make a payment to maintain stability
    Pay(u64), // amount in msats
    /// High risk situation detected
    HighRisk(u32), // risk level
    /// Channel not properly initialized or not found
    NotInitialized,
}

/// Get the latest BTC/USD price from available price feeds
pub fn get_latest_price(agent: &Agent) -> f64 {
    match crate::price_feeds::get_latest_price(agent) {
        Ok(price) => price,
        Err(_) => 84000.0 // TODO
    }
}

/// Check if the given channel exists in the node's channel list
pub fn channel_exists(node: &Node, channel_id: &ChannelId) -> bool {
    let channels = node.list_channels();
    channels.iter().any(|c| c.channel_id == *channel_id)
}

/// Update stable channel balances based on current channel state
pub fn update_balances(node: &Node, mut sc: StableChannel) -> (bool, StableChannel) {
    // Get current price if we don't have it
    if sc.latest_price == 0.0 {
        let agent = Agent::new();
        sc.latest_price = get_latest_price(&agent);
    }
    
    // First check if we're using the default channel ID (all zeros)
    let is_default = sc.channel_id == ChannelId::from_bytes([0; 32]);
    let mut matching_channel_found = false;
    
    // If it's a default channel ID, try to find any available channel
    if is_default {
        if let Some(channel) = node.list_channels().first() {
            sc.channel_id = channel.channel_id;
            println!("Set active channel ID to: {}", sc.channel_id);
            matching_channel_found = true;
            
            // Update the channel balances
            let (our_balance, their_balance) = {
                let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
                let our_balance_sats =
                    (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
                let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                (our_balance_sats, their_balance_sats)
            };
            
            // Update balances based on whether we're the stable receiver or provider
            if sc.is_stable_receiver {
                sc.stable_receiver_btc = Bitcoin::from_sats(our_balance);
                sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                sc.stable_provider_btc = Bitcoin::from_sats(their_balance);
                sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
            } else {
                sc.stable_provider_btc = Bitcoin::from_sats(our_balance);
                sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                sc.stable_receiver_btc = Bitcoin::from_sats(their_balance);
                sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
            }
        }
    } else {
        // Otherwise, look for a channel matching our stored ID
        for channel in node.list_channels() {
            if channel.channel_id == sc.channel_id {
                matching_channel_found = true;
                
                // Update the channel balances
                let (our_balance, their_balance) = {
                    let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
                    let our_balance_sats =
                        (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
                    let their_balance_sats = channel.channel_value_sats - our_balance_sats;
                    (our_balance_sats, their_balance_sats)
                };
                
                // Update balances based on whether we're the stable receiver or provider
                if sc.is_stable_receiver {
                    sc.stable_receiver_btc = Bitcoin::from_sats(our_balance);
                    sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                    sc.stable_provider_btc = Bitcoin::from_sats(their_balance);
                    sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                } else {
                    sc.stable_provider_btc = Bitcoin::from_sats(our_balance);
                    sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
                    sc.stable_receiver_btc = Bitcoin::from_sats(their_balance);
                    sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
                }
                
                break;
            }
        }
    }
    
    if !matching_channel_found {
        println!("No matching channel found for ID: {}", sc.channel_id);
    }
    
    (matching_channel_found, sc)
}

/// Initialize a stable channel with the given parameters
pub fn initialize_stable_channel(
    node: &Node,
    mut sc: StableChannel,
    channel_id_str: &str,
    is_stable_receiver: bool,
    expected_dollar_amount: f64,
    native_amount_sats: f64,
) -> Result<StableChannel, Box<dyn std::error::Error>> {
    // Check if the channel_id is provided as hex string or full channel id
    let channel_id = if channel_id_str.len() == 64 { // It's a hex string
        let channel_id_bytes: [u8; 32] = hex::decode(channel_id_str)?
            .try_into()
            .map_err(|_| "Decoded channel ID has incorrect length")?;
        ChannelId::from_bytes(channel_id_bytes)
    } else { // It's already a formatted channel id
        from_str_channel_id(channel_id_str)?
    };

    // Find the counterparty node ID from the channel list
    let mut counterparty = None;
    for channel in node.list_channels() {
        if channel.channel_id.to_string() == channel_id.to_string() {
            counterparty = Some(channel.counterparty_node_id);
            break;
        }
    }

    let counterparty = counterparty.ok_or("Failed to find channel with the specified ID")?;

    // Update the stable channel state
    sc.channel_id = channel_id;
    sc.is_stable_receiver = is_stable_receiver;
    sc.counterparty = counterparty;
    sc.expected_usd = USD::from_f64(expected_dollar_amount);
    sc.expected_btc = Bitcoin::from_btc(native_amount_sats);
    
    // Get initial price
    let agent = Agent::new();
    let latest_price = get_latest_price(&agent);
    sc.latest_price = latest_price;

    // Update balances
    let (_, updated_sc) = update_balances(node, sc);

    Ok(updated_sc)
}

/// Check if the stable channel is in balance and determine what action to take
pub fn check_stability(node: &Node, sc: &mut StableChannel) {
    let (success, updated_sc) = update_balances(node, sc.clone());
    
    // If update was successful, copy the updated values back to our mutable reference
    if success {
        *sc = updated_sc;
    }

    // Calculate stability
    let dollars_from_par: USD = sc.stable_receiver_usd - sc.expected_usd;
    let percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();

    println!("{:<25} {:>15}", "Expected USD:", sc.expected_usd);
    println!("{:<25} {:>15}", "User USD:", sc.stable_receiver_usd);
    println!("{:<25} {:>5}", "Percent from par:", format!("{:.2}%\n", percent_from_par));

    println!("{:<25} {:>15}", "User BTC:", sc.stable_receiver_btc);
    println!("{:<25} {:>15}", "LSP USD:", sc.stable_provider_usd);

    enum Action {
        Wait,
        Pay,
        DoNothing,
        HighRisk,
    }

    let action = if percent_from_par < 0.1 {
        Action::DoNothing
    } else {
        let is_receiver_below_expected: bool = sc.stable_receiver_usd < sc.expected_usd;

        match (sc.is_stable_receiver, is_receiver_below_expected, sc.risk_level > 100) {
            (_, _, true) => Action::HighRisk, // High risk scenario
            (true, true, false) => Action::Wait,   // We are User and below peg, wait for payment
            (true, false, false) => Action::Pay,   // We are User and above peg, need to pay
            (false, true, false) => Action::Pay,   // We are LSP and below peg, need to pay
            (false, false, false) => Action::Wait, // We are LSP and above peg, wait for payment
        }
    };

    match action {
        Action::DoNothing => {
            println!("\nDifference from par less than 0.1%. Doing nothing.");
            // We could set a flag here to indicate stability
        }
        Action::Wait => {
            println!("\nWaiting for payment...");
            // Update some state to indicate we're waiting
        }
        Action::Pay => {
            println!("\nPaying the difference...\n");
            println!("Paying msats to counterparty: {}", sc.counterparty);


            let amt = USD::to_msats(dollars_from_par, sc.latest_price);

            // Perform payment logic
            let result = node
                .spontaneous_payment()
                .send(amt, sc.counterparty, None);
                
            match result {
                Ok(payment_id) => {
                    println!("Payment sent successfully with payment ID: {}", payment_id);
                    sc.payment_made = true;  // Set flag to indicate payment was made
                },
                Err(e) => println!("Failed to send payment: {}", e),
            }
        }
        Action::HighRisk => {
            println!("Risk level high. Current risk level: {}", sc.risk_level);
            // Update some state to indicate high risk
        }
    }
}

/// Execute a payment to maintain stability
pub fn execute_payment(node: &Node, amount_msats: u64, sc: &StableChannel) -> Result<String, Box<dyn std::error::Error>> {
    // Avoid sending zero-amount payments which could trigger assertion failures
    if amount_msats == 0 {
        return Err("Cannot send a payment with zero amount".into());
    }
    
    // Get channel details
    let channels = node.list_channels();
    let channel = channels.iter().find(|c| c.channel_id == sc.channel_id);
    
    // Verify channel exists
    if channel.is_none() {
        return Err("Channel not found".into());
    }
    
    let channel = channel.unwrap();
    
    // Check if channel is ready
    if !channel.is_channel_ready {
        return Err("Channel is not ready for payments".into());
    }
    
    // Check if we have sufficient outbound capacity
    if channel.outbound_capacity_msat < amount_msats {
        return Err(format!("Insufficient outbound capacity: have {}msat, need {}msat", 
                          channel.outbound_capacity_msat, amount_msats).into());
    }
    
    // Verify the counterparty exists and matches our stable channel
    if channel.counterparty_node_id != sc.counterparty {
        return Err("Counterparty mismatch".into());
    }
    
    // Perform the payment
    let result = node
        .spontaneous_payment()
        .send(amount_msats, sc.counterparty, None)?;
        
    Ok(result.to_string())
}

/// Function to parse a ChannelId from a string (helper function)
fn from_str_channel_id(s: &str) -> Result<ChannelId, Box<dyn std::error::Error>> {
    // Simplified parsing - may need to be expanded based on the actual string format
    let clean_str = s.trim();
    
    if clean_str.len() >= 64 {
        // It's likely a hex string
        let hex_part = if clean_str.len() > 64 {
            // Extract just the 64 hex chars if there's extra formatting
            let start = clean_str.find(|c: char| c.is_ascii_hexdigit())
                .ok_or("No hex digits found in channel ID string")?;
            &clean_str[start..(start+64)]
        } else {
            clean_str
        };
        
        let bytes = hex::decode(hex_part)?;
        if bytes.len() != 32 {
            return Err(format!("Expected 32 bytes, got {}", bytes.len()).into());
        }
        
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(ChannelId::from_bytes(arr))
    } else {
        Err("Channel ID string is too short".into())
    }
}