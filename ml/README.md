# ML Pipeline for BTC Arb Bot

## Overview

This ML pipeline learns from trading data to optimize:
1. **Entry timing** - When during the 15-min window are spreads widest?
2. **Fill prediction** - Which price levels are most likely to get filled?
3. **Spread prediction** - Is the spread about to widen or tighten?

## Setup

```bash
cd ml
pip install -r requirements.txt
```

## Workflow

### 1. Collect Data

Run the bot in `DRY_RUN=true` mode. It saves data to `./data/`:
- `snapshots_*.jsonl` - Orderbook snapshots every update
- `fills_*.jsonl` - Trade fills
- `orders_*.jsonl` - Orders placed
- `summaries.jsonl` - Session summaries

### 2. Extract Features

```bash
python extract_features.py
```

Creates CSV files in `./features/`:
- `spread_features.csv` - For spread prediction model
- `fill_features.csv` - For fill prediction model
- `timing_features.csv` - For timing analysis
- `session_features.csv` - For session profitability analysis

### 3. Train Models

```bash
python train_models.py
```

Creates models in `./models/`:
- `spread_predictor.pkl` - Predicts spread changes
- `fill_predictor.pkl` - Predicts fill probability
- `timing_recommendations.json` - Optimal entry windows

### 4. Use Predictions

**CLI:**
```bash
python predict.py '{"spread_now": 3.5, "seconds_to_resolution": 600, "spread_mean_10": 3.2}'
```

**HTTP Server (for low-latency):**
```bash
python predict.py --serve 8765
```

Then from bot:
```bash
curl -X POST http://127.0.0.1:8765 -d '{"spread_now": 3.5, ...}'
```

## Models

### Spread Predictor
- **Input**: Current spread, recent spread history, time to resolution
- **Output**:
  - `spread_will_increase` (bool) - Will spread widen in next N updates?
  - `spread_increase_prob` (float) - Probability of increase
  - `predicted_spread` (float) - Predicted spread value

### Fill Predictor
- **Input**: Order price, best ask, spread, time to resolution
- **Output**:
  - `will_fill` (bool) - Will this order get filled?
  - `fill_probability` (float) - Probability of fill

### Timing Optimizer
- **Output**: Which minute (0-14) of the 15-min period has best spreads

## Integration with Bot

The Rust bot can:
1. Call `predict.py` via subprocess for occasional predictions
2. Run `predict.py --serve` as background process for real-time predictions
3. Export models to ONNX for native Rust inference (future)

## Iteration

1. Run bot → Collect data
2. Extract features → Train models
3. Analyze results → Adjust strategy
4. Repeat

The more data you collect, the better the models become.
