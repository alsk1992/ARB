use anyhow::{Context, Result};
use ethers::prelude::*;
use ethers::types::transaction::eip712::{Eip712, TypedData};
use ethers::utils::hex;
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;
use uuid::Uuid;

use crate::types::{Order, SignedOrder, OrderType, Side};

// Polymarket Exchange contract addresses on Polygon
// Regular CTF Exchange for non-negRisk markets
const CTF_EXCHANGE_ADDRESS: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
// NegRisk CTF Exchange for negRisk markets (BTC 15-min markets use this)
const NEG_RISK_CTF_EXCHANGE_ADDRESS: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";
const CHAIN_ID: u64 = 137; // Polygon mainnet

/// EIP-712 Order Signer for Polymarket
pub struct OrderSigner {
    wallet: LocalWallet,
    address: Address,
    funder: Address,
}

impl OrderSigner {
    pub fn new(private_key: &str, funder_address: &str) -> Result<Self> {
        let wallet = private_key
            .parse::<LocalWallet>()
            .context("Invalid private key")?
            .with_chain_id(CHAIN_ID);

        let address = wallet.address();
        let funder = Address::from_str(funder_address).context("Invalid funder address")?;

        Ok(Self {
            wallet,
            address,
            funder,
        })
    }

    /// Create and sign an order
    pub async fn create_order(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        side: Side,
        tick_size: Decimal,
        neg_risk: bool,
    ) -> Result<Order> {
        // Calculate amounts based on side (from Polymarket's official clob-client)
        // USDC uses 6 decimals, conditional tokens use 6 decimals
        //
        // For BUY orders:
        //   - makerAmount = size * price (USDC cost - what you PAY)
        //   - takerAmount = size (shares - what you GET)
        //
        // For SELL orders:
        //   - makerAmount = size (shares - what you SELL)
        //   - takerAmount = size * price (USDC - what you GET)
        let (maker_amount, taker_amount) = match side {
            Side::Buy => {
                // BUY: You pay USDC (maker), you get shares (taker)
                let cost = size * price;
                let cost_raw = (cost * Decimal::from(1_000_000)).round().to_string();
                let shares_raw = (size * Decimal::from(1_000_000)).round().to_string();
                (cost_raw, shares_raw) // maker=cost, taker=shares
            }
            Side::Sell => {
                // SELL: You give shares (maker), you get USDC (taker)
                let cost = size * price;
                let shares_raw = (size * Decimal::from(1_000_000)).round().to_string();
                let cost_raw = (cost * Decimal::from(1_000_000)).round().to_string();
                (shares_raw, cost_raw) // maker=shares, taker=cost
            }
        };

        // Generate random salt
        let salt = Uuid::new_v4().as_u128().to_string();

        // Set expiration to 1 hour from now
        let expiration = (chrono::Utc::now().timestamp() + 3600).to_string();

        // Create order struct for signing
        let order_data = json!({
            "salt": salt,
            "maker": format!("{:?}", self.funder),
            "signer": format!("{:?}", self.address),
            "taker": "0x0000000000000000000000000000000000000000",
            "tokenId": token_id,
            "makerAmount": maker_amount,
            "takerAmount": taker_amount,
            "expiration": expiration,
            "nonce": "0",
            "feeRateBps": "0",
            "side": if matches!(side, Side::Buy) { 0 } else { 1 },
            "signatureType": 0 // EOA
        });

        // Sign the order using EIP-712 (use correct exchange contract)
        let signature = self.sign_order(&order_data, neg_risk).await?;

        let signed_order = SignedOrder {
            salt,
            maker: format!("{:?}", self.funder),
            signer: format!("{:?}", self.address),
            taker: "0x0000000000000000000000000000000000000000".to_string(),
            token_id: token_id.to_string(),
            maker_amount,
            taker_amount,
            expiration,
            nonce: "0".to_string(),
            fee_rate_bps: "0".to_string(),
            side,
            signature_type: 0,
            signature,
        };

        Ok(Order {
            order: signed_order,
            owner: format!("{:?}", self.funder),
            order_type: OrderType::Gtc,
        })
    }

    /// Sign order using EIP-712
    async fn sign_order(&self, order: &serde_json::Value, neg_risk: bool) -> Result<String> {
        // Select correct exchange contract based on market type
        let exchange_address = if neg_risk {
            NEG_RISK_CTF_EXCHANGE_ADDRESS
        } else {
            CTF_EXCHANGE_ADDRESS
        };

        // EIP-712 domain
        let domain = json!({
            "name": "Polymarket CTF Exchange",
            "version": "1",
            "chainId": CHAIN_ID,
            "verifyingContract": exchange_address
        });

        // EIP-712 types
        let types = json!({
            "EIP712Domain": [
                {"name": "name", "type": "string"},
                {"name": "version", "type": "string"},
                {"name": "chainId", "type": "uint256"},
                {"name": "verifyingContract", "type": "address"}
            ],
            "Order": [
                {"name": "salt", "type": "uint256"},
                {"name": "maker", "type": "address"},
                {"name": "signer", "type": "address"},
                {"name": "taker", "type": "address"},
                {"name": "tokenId", "type": "uint256"},
                {"name": "makerAmount", "type": "uint256"},
                {"name": "takerAmount", "type": "uint256"},
                {"name": "expiration", "type": "uint256"},
                {"name": "nonce", "type": "uint256"},
                {"name": "feeRateBps", "type": "uint256"},
                {"name": "side", "type": "uint8"},
                {"name": "signatureType", "type": "uint8"}
            ]
        });

        // Create typed data
        let typed_data = json!({
            "types": types,
            "primaryType": "Order",
            "domain": domain,
            "message": order
        });

        // Parse as TypedData and sign
        let typed_data: TypedData = serde_json::from_value(typed_data)?;

        // Hash and sign
        let hash = typed_data.encode_eip712()?;
        let signature = self.wallet.sign_hash(H256::from(hash))?;

        // Return hex signature
        Ok(format!("0x{}", hex::encode(signature.to_vec())))
    }

    /// Create a ladder of orders at different price levels
    pub async fn create_ladder_orders(
        &self,
        token_id: &str,
        base_price: Decimal,
        total_size: Decimal,
        levels: u32,
        spacing: Decimal,
        side: Side,
        tick_size: Decimal,
        neg_risk: bool,
    ) -> Result<Vec<Order>> {
        let size_per_level = total_size / Decimal::from(levels);
        let mut orders = Vec::with_capacity(levels as usize);

        for i in 0..levels {
            // For BUY orders, ladder DOWN from base price
            // For SELL orders, ladder UP from base price
            let price_offset = spacing * Decimal::from(i);
            let price = match side {
                Side::Buy => base_price - price_offset,
                Side::Sell => base_price + price_offset,
            };

            // Round to tick size
            let price = (price / tick_size).round() * tick_size;

            // Skip if price is invalid
            if price <= Decimal::ZERO || price >= Decimal::ONE {
                continue;
            }

            let order = self.create_order(
                token_id,
                price,
                size_per_level,
                side,
                tick_size,
                neg_risk,
            ).await?;

            orders.push(order);
        }

        Ok(orders)
    }
}
