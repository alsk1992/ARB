/// BTC 15-min Arbitrage Bot for Polymarket
///
/// This library provides components for arbitrage trading on Polymarket's
/// BTC 15-minute binary markets. The strategy involves buying both UP and
/// DOWN outcomes such that the combined cost is less than $1, guaranteeing
/// profit regardless of which outcome wins.

pub mod alerts;
pub mod auth;
pub mod clob;
pub mod config;
pub mod datalog;
pub mod market;
pub mod ml_client;
pub mod multi_strategy;
pub mod orderbook;
pub mod position;
pub mod presign;
pub mod retry;
pub mod signer;
pub mod strategies;
pub mod strategy;
pub mod types;
pub mod websocket;
