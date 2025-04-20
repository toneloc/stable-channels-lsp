use crate::types::{Bitcoin, StableChannel, USD};
use ldk_node::{
    lightning::ln::types::ChannelId, Node,
};
use ureq::Agent;
use crate::price_feeds::get_cached_price;

/// Get the current BTC/USD price, preferring cached value when available
pub fn get_current_price(agent: &Agent) -> f64 {
    // First try the cached price
    let cached_price = get_cached_price();
    
    // Use the cached price if valid
    if cached_price > 0.0 {
        return cached_price;
    }
    
    // Fall back to fetching a new price
    match crate::price_feeds::get_latest_price(agent) {
        Ok(price) => price,
        Err(_) => 0.0 
    }
}

/// Check if the given channel exists in the node's channel list
pub fn channel_exists(node: &Node, channel_id: &ChannelId) -> bool {
    let channels = node.list_channels();
    channels.iter().any(|c| c.channel_id == *channel_id)
}

// Can run in backgound
pub fn update_balances<'update_balance_lifetime>(
    node: &Node,
    sc: &'update_balance_lifetime mut StableChannel,
) -> (bool, &'update_balance_lifetime mut StableChannel) {
    if sc.latest_price == 0.0 {
        sc.latest_price = get_cached_price();
        
        if sc.latest_price == 0.0 {
            let agent = Agent::new();
            sc.latest_price = get_current_price(&agent);
        }
    }
    
    let channels = node.list_channels();
    let matching_channel = if sc.channel_id == ChannelId::from_bytes([0; 32]) {
        channels.first()
    } else {
        channels.iter().find(|c| c.channel_id == sc.channel_id)
    };
    
    if let Some(channel) = matching_channel {
        if sc.channel_id == ChannelId::from_bytes([0; 32]) {
            sc.channel_id = channel.channel_id;
            println!("Set active channel ID to: {}", sc.channel_id);
        }
        
        let unspendable_punishment_sats = channel.unspendable_punishment_reserve.unwrap_or(0);
        let our_balance_sats = (channel.outbound_capacity_msat / 1000) + unspendable_punishment_sats;
        let their_balance_sats = channel.channel_value_sats - our_balance_sats;
        
        if sc.is_stable_receiver {
            sc.stable_receiver_btc = Bitcoin::from_sats(our_balance_sats);
            sc.stable_provider_btc = Bitcoin::from_sats(their_balance_sats);
        } else {
            sc.stable_provider_btc = Bitcoin::from_sats(our_balance_sats);
            sc.stable_receiver_btc = Bitcoin::from_sats(their_balance_sats);
        }
        
        sc.stable_receiver_usd = USD::from_bitcoin(sc.stable_receiver_btc, sc.latest_price);
        sc.stable_provider_usd = USD::from_bitcoin(sc.stable_provider_btc, sc.latest_price);
        
        return (true, sc);
    }
    
    println!("No matching channel found for ID: {}", sc.channel_id);
    (true, sc)
}

pub fn check_stability(node: &Node, sc: &mut StableChannel, price: f64) {
    println!("\n=== CHECKING CHANNEL STABILITY ===");
    
    let current_price = if price > 0.0 {
        price
    } else {
        // Otherwise use cached price
        let cached_price = get_cached_price();
        if cached_price > 0.0 {
            cached_price
        } else {
            println!("Skipping stability check: No valid price available");
            return;
        }
    };
    
    // Update the price in the stable channel
    sc.latest_price = current_price;

    // Get updated balances with the current price
    let (success, _) = update_balances(node, sc);
    
    if success {
        println!("Channel balances updated successfully");
    } else {
        println!("Failed to update channel balances");
    }
    
    // Calculate stability
    let dollars_from_par = sc.stable_receiver_usd - sc.expected_usd;
    let percent_from_par = ((dollars_from_par / sc.expected_usd) * 100.0).abs();
    
    println!("Channel status:");
    println!("  Expected USD:      {}", sc.expected_usd);
    println!("  Current user USD:  {}", sc.stable_receiver_usd);
    println!("  Difference:        ${:.2}", dollars_from_par.0);
    println!("  Percent from par:  {:.2}%", percent_from_par);
    println!("  User BTC:          {}", sc.stable_receiver_btc);
    println!("  LSP USD:           {}", sc.stable_provider_usd);
    println!("  BTC price:         ${:.2}", sc.latest_price);
    
    // Determine action based on criteria
    let is_receiver_below_expected = sc.stable_receiver_usd < sc.expected_usd;
    
    if percent_from_par < 0.1 {
        println!("\n✓ STABLE: Difference from par less than 0.1%. No action needed.");
        return;
    } else if sc.risk_level > 100 {
        println!("\n⚠ HIGH RISK: Risk level ({}) exceeds threshold. Action suspended.", sc.risk_level);
        return;
    } else if (sc.is_stable_receiver && is_receiver_below_expected) || 
              (!sc.is_stable_receiver && !is_receiver_below_expected) {
        println!("\n⏱ CHECKING: Balance conditions indicate we should check for payment from counterparty.");
        if sc.is_stable_receiver {
            println!("  We are the stable receiver and our balance is below expected.");
        } else {
            println!("  We are the stable provider and receiver balance is above expected.");
        }
        return;
    }
    
    // Only payment action remains
    println!("\n💸 PAYING: Sending payment to maintain stability.");
    if sc.is_stable_receiver {
        println!("  We are the stable receiver and our balance is above expected.");
    } else {
        println!("  We are the stable provider and receiver balance is below expected.");
    }
    
    let amt = USD::to_msats(dollars_from_par, sc.latest_price);
    println!("  Amount to pay:     {} msats (${:.2})", amt, dollars_from_par.0.abs());
    println!("  Counterparty:      {}", sc.counterparty);
    
    match node.spontaneous_payment().send(amt, sc.counterparty, None) {
        Ok(payment_id) => {
            println!("✓ Payment sent successfully!");
            println!("  Payment ID: {}", payment_id);
            sc.payment_made = true;
        },
        Err(e) => println!("✗ Failed to send payment: {}", e),
    }
    
    println!("=== STABILITY CHECK COMPLETE ===");
}

// For backward compatibility with other code
pub fn check_stability_with_price(node: &Node, sc: &mut StableChannel, price: f64) {
    // Only use provided price if it's valid
    if price > 0.0 {
        sc.latest_price = price;
    } else {
        // Otherwise use cached price
        let cached_price = get_cached_price();
        if cached_price > 0.0 {
            sc.latest_price = cached_price;
        }
    }
    
    // Call the main implementation
    check_stability(node, sc, sc.latest_price);
}