use anyhow::{Context, Result};
use ethers::prelude::*;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::storage::TradeStorage;
use crate::types::Trade;

// Polymarket CTF Exchange contract address on Polygon
const CTF_EXCHANGE_ADDRESS: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";

// ABI for the events we care about
abigen!(
    CTFExchange,
    r#"[
        event OrderFilled(bytes32 indexed orderHash, address indexed maker, address indexed taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 fee)
        event OrdersMatched(bytes32 indexed makerOrderHash, bytes32 indexed takerOrderHash, address indexed maker, address taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 makerFee, uint256 takerFee)
    ]"#
);

pub struct PolygonListener {
    provider: Arc<Provider<Ws>>,
    contract: CTFExchange<Provider<Ws>>,
    storage: TradeStorage,
}

impl PolygonListener {
    pub async fn new(rpc_url: &str, storage: TradeStorage) -> Result<Self> {
        let provider = Provider::<Ws>::connect(rpc_url)
            .await
            .context("Failed to connect to Polygon WebSocket")?;

        let contract_address: Address = CTF_EXCHANGE_ADDRESS
            .parse()
            .context("Invalid contract address")?;

        let contract = CTFExchange::new(contract_address, Arc::new(provider.clone()));

        Ok(Self {
            provider: Arc::new(provider),
            contract,
            storage,
        })
    }

    pub async fn start_listening(&self) -> Result<()> {
        info!("📡 Subscribing to CTF Exchange events at {}", CTF_EXCHANGE_ADDRESS);

        // Subscribe to all events from the contract
        let events = self.contract.events();
        let mut stream = events.stream().await?;

        info!("✅ Subscribed! Waiting for trades...");

        let mut trade_count = 0u64;

        while let Some(event) = stream.next().await {
            match event {
                Ok(event) => {
                    trade_count += 1;

                    match event {
                        CTFExchangeEvents::OrderFilledFilter(fill) => {
                            debug!("📦 OrderFilled event detected");
                            if let Err(e) = self.process_order_filled(fill).await {
                                error!("Failed to process OrderFilled: {}", e);
                            }
                        }
                        CTFExchangeEvents::OrdersMatchedFilter(matched) => {
                            debug!("🤝 OrdersMatched event detected");
                            if let Err(e) = self.process_orders_matched(matched).await {
                                error!("Failed to process OrdersMatched: {}", e);
                            }
                        }
                    }

                    if trade_count % 100 == 0 {
                        info!("📊 Processed {} trades", trade_count);
                    }
                }
                Err(e) => {
                    warn!("Error receiving event: {}", e);
                }
            }
        }

        warn!("Event stream ended unexpectedly");
        Ok(())
    }

    async fn process_order_filled(&self, event: OrderFilledFilter) -> Result<()> {
        // Generate base ID from event data
        let base_id = format!(
            "{}_{:?}_{:?}",
            hex::encode(&event.order_hash),
            event.maker,
            event.taker
        );
        let block_number = 0u64;  // Will be filled from actual block data later
        let timestamp = chrono::Utc::now();

        // Create maker trade with unique tx_hash (includes maker address)
        let maker_tx_hash = format!("0x{}_{:?}", &base_id[..60].chars().take(60).collect::<String>(), event.maker);
        let mut maker_trade = Trade::new(
            maker_tx_hash,
            block_number,
            timestamp,
            format!("{:?}", event.maker),
            true, // is_maker
        );
        maker_trade.token_id = event.maker_asset_id.to_string();
        maker_trade.size = self.wei_to_decimal(event.maker_amount_filled);
        maker_trade.fee_paid = Some(self.wei_to_decimal(event.fee));
        maker_trade.order_hash = Some(format!("0x{}", hex::encode(event.order_hash)));
        maker_trade.side = "SELL".to_string(); // Maker is selling their asset

        // Calculate price: taker_amount / maker_amount
        if event.maker_amount_filled > U256::zero() {
            maker_trade.price = event.taker_amount_filled.as_u128() as f64
                / event.maker_amount_filled.as_u128() as f64;
        }

        // Create taker trade with unique tx_hash (includes taker address)
        let taker_tx_hash = format!("0x{}_{:?}", &base_id[..60].chars().take(60).collect::<String>(), event.taker);
        let mut taker_trade = Trade::new(
            taker_tx_hash,
            block_number,
            timestamp,
            format!("{:?}", event.taker),
            false, // is_maker
        );
        taker_trade.token_id = event.taker_asset_id.to_string();
        taker_trade.size = self.wei_to_decimal(event.taker_amount_filled);
        taker_trade.order_hash = Some(format!("0x{}", hex::encode(event.order_hash)));
        taker_trade.side = "BUY".to_string(); // Taker is buying
        taker_trade.price = maker_trade.price;

        // Store both trades
        self.storage.save_trade(&maker_trade).await?;
        self.storage.save_trade(&taker_trade).await?;

        info!(
            "💸 Trade: {} bought from {} | Size: {} @ {}",
            taker_trade.wallet_address,
            maker_trade.wallet_address,
            taker_trade.size,
            taker_trade.price
        );

        Ok(())
    }

    async fn process_orders_matched(&self, event: OrdersMatchedFilter) -> Result<()> {
        // Generate base ID from event data
        let base_id = format!(
            "{}_{}_{}",
            hex::encode(&event.maker_order_hash),
            hex::encode(&event.taker_order_hash),
            format!("{:?}{:?}", event.maker, event.taker)
        );
        let block_number = 0u64;
        let timestamp = chrono::Utc::now();

        // Maker trade with unique tx_hash
        let maker_tx_hash = format!("0x{}_{:?}", &base_id[..60].chars().take(60).collect::<String>(), event.maker);
        let mut maker_trade = Trade::new(
            maker_tx_hash,
            block_number,
            timestamp,
            format!("{:?}", event.maker),
            true,
        );
        maker_trade.token_id = event.maker_asset_id.to_string();
        maker_trade.size = self.wei_to_decimal(event.maker_amount_filled);
        maker_trade.fee_paid = Some(self.wei_to_decimal(event.maker_fee));
        maker_trade.order_hash = Some(format!("0x{}", hex::encode(event.maker_order_hash)));
        maker_trade.side = "SELL".to_string();

        if event.maker_amount_filled > U256::zero() {
            maker_trade.price = event.taker_amount_filled.as_u128() as f64
                / event.maker_amount_filled.as_u128() as f64;
        }

        // Taker trade with unique tx_hash
        let taker_tx_hash = format!("0x{}_{:?}", &base_id[..60].chars().take(60).collect::<String>(), event.taker);
        let mut taker_trade = Trade::new(
            taker_tx_hash,
            block_number,
            timestamp,
            format!("{:?}", event.taker),
            false,
        );
        taker_trade.token_id = event.taker_asset_id.to_string();
        taker_trade.size = self.wei_to_decimal(event.taker_amount_filled);
        taker_trade.fee_paid = Some(self.wei_to_decimal(event.taker_fee));
        taker_trade.order_hash = Some(format!("0x{}", hex::encode(event.taker_order_hash)));
        taker_trade.side = "BUY".to_string();
        taker_trade.price = maker_trade.price;

        // Store both trades
        self.storage.save_trade(&maker_trade).await?;
        self.storage.save_trade(&taker_trade).await?;

        info!(
            "🤝 Matched: {} ↔️ {} | Size: {} @ {}",
            maker_trade.wallet_address,
            taker_trade.wallet_address,
            maker_trade.size,
            maker_trade.price
        );

        Ok(())
    }

    fn wei_to_decimal(&self, wei: U256) -> f64 {
        // Convert from wei (18 decimals) to f64
        wei.as_u128() as f64 / 1e18
    }
}
