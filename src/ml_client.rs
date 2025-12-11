use anyhow::Result;
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

const ML_SERVER_URL: &str = "http://127.0.0.1:8765";
const TIMEOUT_MS: u64 = 100; // Fast timeout - don't slow down trading

/// ML prediction client
pub struct MlClient {
    client: Client,
    enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpreadFeatures {
    pub spread_now: f64,
    pub up_ask: f64,
    pub down_ask: f64,
    pub combined_ask: f64,
    pub seconds_to_resolution: f64,
    pub minute_of_period: f64,
    pub spread_mean_10: f64,
    pub spread_max_10: f64,
    pub spread_min_10: f64,
    pub spread_volatility_10: f64,
    pub spread_trend_10: f64,
    pub up_trend_10: f64,
    pub down_trend_10: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FillFeatures {
    pub order_price: f64,
    pub best_ask: f64,
    pub price_vs_ask: f64,
    pub price_vs_ask_pct: f64,
    pub spread_pct: f64,
    pub seconds_to_resolution: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpreadPrediction {
    pub spread_will_increase: Option<bool>,
    pub spread_increase_prob: Option<f64>,
    pub predicted_spread: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FillPrediction {
    pub will_fill: Option<bool>,
    pub fill_probability: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimingRecommendation {
    pub best_minute: Option<i32>,
    pub avg_spread: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MlPrediction {
    pub spread: Option<SpreadPrediction>,
    pub fill: Option<FillPrediction>,
    pub timing: Option<TimingRecommendation>,
}

impl MlClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(TIMEOUT_MS))
            .build()
            .unwrap_or_default();

        Self {
            client,
            enabled: true,
        }
    }

    /// Check if ML server is available
    pub async fn health_check(&mut self) -> bool {
        let features = SpreadFeatures {
            spread_now: 3.0,
            up_ask: 0.48,
            down_ask: 0.49,
            combined_ask: 0.97,
            seconds_to_resolution: 600.0,
            minute_of_period: 5.0,
            spread_mean_10: 3.0,
            spread_max_10: 4.0,
            spread_min_10: 2.0,
            spread_volatility_10: 0.5,
            spread_trend_10: 0.0,
            up_trend_10: 0.0,
            down_trend_10: 0.0,
        };

        match self.predict_spread(&features).await {
            Ok(_) => {
                self.enabled = true;
                debug!("ML server connected");
                true
            }
            Err(_) => {
                self.enabled = false;
                debug!("ML server not available, predictions disabled");
                false
            }
        }
    }

    /// Get spread prediction
    pub async fn predict_spread(&self, features: &SpreadFeatures) -> Result<SpreadPrediction> {
        if !self.enabled {
            return Ok(SpreadPrediction {
                spread_will_increase: None,
                spread_increase_prob: None,
                predicted_spread: None,
            });
        }

        let response: MlPrediction = self.client
            .post(ML_SERVER_URL)
            .json(features)
            .send()
            .await?
            .json()
            .await?;

        Ok(response.spread.unwrap_or(SpreadPrediction {
            spread_will_increase: None,
            spread_increase_prob: None,
            predicted_spread: None,
        }))
    }

    /// Get fill prediction for a specific order
    pub async fn predict_fill(&self, features: &FillFeatures) -> Result<FillPrediction> {
        if !self.enabled {
            return Ok(FillPrediction {
                will_fill: None,
                fill_probability: None,
            });
        }

        let response: MlPrediction = self.client
            .post(ML_SERVER_URL)
            .json(features)
            .send()
            .await?
            .json()
            .await?;

        Ok(response.fill.unwrap_or(FillPrediction {
            will_fill: None,
            fill_probability: None,
        }))
    }

    /// Get timing recommendation
    pub async fn get_timing_recommendation(&self) -> Result<Option<TimingRecommendation>> {
        if !self.enabled {
            return Ok(None);
        }

        // Send minimal request just to get timing
        let features = SpreadFeatures {
            spread_now: 0.0,
            up_ask: 0.0,
            down_ask: 0.0,
            combined_ask: 0.0,
            seconds_to_resolution: 0.0,
            minute_of_period: 0.0,
            spread_mean_10: 0.0,
            spread_max_10: 0.0,
            spread_min_10: 0.0,
            spread_volatility_10: 0.0,
            spread_trend_10: 0.0,
            up_trend_10: 0.0,
            down_trend_10: 0.0,
        };

        let response: MlPrediction = self.client
            .post(ML_SERVER_URL)
            .json(&features)
            .send()
            .await?
            .json()
            .await?;

        Ok(response.timing)
    }

    /// Should we enter now based on ML recommendation?
    pub async fn should_enter_now(
        &self,
        current_spread: Decimal,
        seconds_to_resolution: i64,
        spread_history: &[Decimal],
    ) -> bool {
        // If ML not available, use simple heuristic
        if !self.enabled || spread_history.len() < 10 {
            return current_spread >= Decimal::from(3); // 3% minimum
        }

        let minute_of_period = (15 * 60 - seconds_to_resolution) as f64 / 60.0;

        // Check timing recommendation
        if let Ok(Some(timing)) = self.get_timing_recommendation().await {
            if let Some(best_minute) = timing.best_minute {
                let current_minute = minute_of_period as i32;
                // If we're in the optimal window (±2 minutes), go ahead
                if (current_minute - best_minute).abs() <= 2 {
                    debug!("In optimal entry window (minute {})", current_minute);
                    return true;
                }
            }
        }

        // Check spread prediction
        let recent: Vec<f64> = spread_history.iter()
            .rev()
            .take(10)
            .map(|d| d.to_string().parse().unwrap_or(0.0))
            .collect();

        let spread_mean = recent.iter().sum::<f64>() / recent.len() as f64;
        let spread_max = recent.iter().cloned().fold(0.0, f64::max);
        let spread_min = recent.iter().cloned().fold(f64::MAX, f64::min);
        let spread_trend = recent.first().unwrap_or(&0.0) - recent.last().unwrap_or(&0.0);

        let features = SpreadFeatures {
            spread_now: current_spread.to_string().parse().unwrap_or(0.0),
            up_ask: 0.48, // Approximation
            down_ask: 0.49,
            combined_ask: 0.97,
            seconds_to_resolution: seconds_to_resolution as f64,
            minute_of_period,
            spread_mean_10: spread_mean,
            spread_max_10: spread_max,
            spread_min_10: spread_min,
            spread_volatility_10: 0.5, // Approximation
            spread_trend_10: spread_trend,
            up_trend_10: 0.0,
            down_trend_10: 0.0,
        };

        if let Ok(prediction) = self.predict_spread(&features).await {
            // If spread is predicted to increase, wait
            if prediction.spread_will_increase == Some(true) {
                if let Some(prob) = prediction.spread_increase_prob {
                    if prob > 0.7 {
                        debug!("ML predicts spread will increase (prob: {:.2}), waiting...", prob);
                        return false;
                    }
                }
            }
        }

        // Default: enter if spread is decent
        current_spread >= Decimal::from(3)
    }

    /// Get optimal price levels for ladder based on fill predictions
    pub async fn optimize_ladder_prices(
        &self,
        base_prices: Vec<Decimal>,
        best_ask: Decimal,
        spread_pct: Decimal,
        seconds_to_resolution: i64,
    ) -> Vec<Decimal> {
        if !self.enabled {
            return base_prices;
        }

        let mut optimized = Vec::new();
        let ask_f64: f64 = best_ask.to_string().parse().unwrap_or(0.5);
        let spread_f64: f64 = spread_pct.to_string().parse().unwrap_or(0.0);

        for price in &base_prices {
            let price_f64: f64 = price.to_string().parse().unwrap_or(0.0);

            let features = FillFeatures {
                order_price: price_f64,
                best_ask: ask_f64,
                price_vs_ask: price_f64 - ask_f64,
                price_vs_ask_pct: (price_f64 - ask_f64) / ask_f64 * 100.0,
                spread_pct: spread_f64,
                seconds_to_resolution: seconds_to_resolution as f64,
            };

            if let Ok(prediction) = self.predict_fill(&features).await {
                // Only include prices with decent fill probability
                if let Some(prob) = prediction.fill_probability {
                    if prob >= 0.3 {
                        optimized.push(*price);
                    } else {
                        debug!("Skipping price {} - low fill prob {:.2}", price, prob);
                    }
                } else {
                    optimized.push(*price);
                }
            } else {
                optimized.push(*price);
            }
        }

        // Always return at least some prices
        if optimized.is_empty() {
            warn!("ML filtered all prices, using original ladder");
            return base_prices;
        }

        optimized
    }
}

impl Default for MlClient {
    fn default() -> Self {
        Self::new()
    }
}
