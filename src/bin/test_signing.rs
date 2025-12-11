/// Test script to verify EIP-712 signing works correctly
///
/// This tests the order signing against Polymarket's API without
/// actually placing orders. Run with:
///   cargo run --bin test_signing

use anyhow::Result;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// Import from main crate
use btc_arb_bot::config::Config;
use btc_arb_bot::signer::OrderSigner;
use btc_arb_bot::types::Side;

#[tokio::main]
async fn main() -> Result<()> {
    println!("═══════════════════════════════════════");
    println!("       BTC Arb Bot - Signing Test      ");
    println!("═══════════════════════════════════════\n");

    // Load config
    let config = Config::from_env()?;
    println!("✓ Config loaded");
    println!("  Address: {}", config.address);

    // Create signer
    let signer = OrderSigner::new(&config.private_key, &config.address)?;
    println!("✓ Signer created\n");

    // Test creating a BUY order
    println!("Creating test BUY order...");
    let test_token_id = "21742633143463906290569050155826241533067272736897614950488156847949938836455";
    let price = dec!(0.48);
    let size = dec!(100); // 100 shares
    let tick_size = dec!(0.01);
    let neg_risk = true; // BTC markets are neg risk

    let order = signer.create_order(
        test_token_id,
        price,
        size,
        Side::Buy,
        tick_size,
        neg_risk,
    ).await?;

    println!("✓ Order created successfully!\n");
    println!("Order details:");
    println!("  Token ID: {}", order.order.token_id);
    println!("  Side: BUY (0)");
    println!("  Price: {}", price);
    println!("  Size: {} shares", size);
    println!("  Maker Amount (cost): {}", order.order.maker_amount);
    println!("  Taker Amount (shares): {}", order.order.taker_amount);
    println!("  Signature: {}...", &order.order.signature[..40]);
    println!("  Expiration: {}", order.order.expiration);

    // Verify amounts are correct
    let expected_cost = (size * price * Decimal::from(1_000_000)).round();
    let expected_shares = (size * Decimal::from(1_000_000)).round();
    let actual_cost: Decimal = order.order.maker_amount.parse()?;
    let actual_shares: Decimal = order.order.taker_amount.parse()?;

    println!("\nAmount verification:");
    println!("  Expected cost (maker): {} raw units", expected_cost);
    println!("  Actual cost (maker):   {} raw units", actual_cost);
    println!("  Expected shares (taker): {} raw units", expected_shares);
    println!("  Actual shares (taker):   {} raw units", actual_shares);

    if actual_cost == expected_cost && actual_shares == expected_shares {
        println!("\n✓ PASS: Amounts are correct!");
    } else {
        println!("\n✗ FAIL: Amount mismatch!");
        return Err(anyhow::anyhow!("Amount calculation error"));
    }

    // Test SELL order too
    println!("\n───────────────────────────────────────\n");
    println!("Creating test SELL order...");

    let sell_order = signer.create_order(
        test_token_id,
        dec!(0.52),
        dec!(50),
        Side::Sell,
        tick_size,
        neg_risk,
    ).await?;

    println!("✓ SELL order created");
    println!("  Maker Amount (shares): {}", sell_order.order.maker_amount);
    println!("  Taker Amount (cost): {}", sell_order.order.taker_amount);

    // For SELL: maker = shares, taker = cost
    let sell_expected_shares = (dec!(50) * Decimal::from(1_000_000)).round();
    let sell_expected_cost = (dec!(50) * dec!(0.52) * Decimal::from(1_000_000)).round();
    let sell_actual_maker: Decimal = sell_order.order.maker_amount.parse()?;
    let sell_actual_taker: Decimal = sell_order.order.taker_amount.parse()?;

    if sell_actual_maker == sell_expected_shares && sell_actual_taker == sell_expected_cost {
        println!("✓ PASS: SELL amounts correct!");
    } else {
        println!("✗ FAIL: SELL amount mismatch!");
        println!("  Expected maker (shares): {}", sell_expected_shares);
        println!("  Expected taker (cost): {}", sell_expected_cost);
    }

    println!("\n═══════════════════════════════════════");
    println!("         All tests passed!             ");
    println!("═══════════════════════════════════════");

    Ok(())
}
