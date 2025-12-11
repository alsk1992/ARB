#!/usr/bin/env python3
"""
Feature extraction pipeline for ML model training.

Reads raw data logs and extracts features suitable for training models to:
1. Predict optimal entry timing
2. Predict fill probability at different price levels
3. Predict spread movements
4. Optimize ladder parameters

Output: CSV files ready for training with scikit-learn, XGBoost, or PyTorch
"""

import json
import os
from datetime import datetime, timedelta
from pathlib import Path
from typing import List, Dict, Any, Optional
import csv

# Data directory
DATA_DIR = Path(__file__).parent.parent / "data"
OUTPUT_DIR = Path(__file__).parent / "features"


def load_jsonl(filepath: Path) -> List[Dict]:
    """Load JSONL file into list of dicts."""
    if not filepath.exists():
        return []

    data = []
    with open(filepath) as f:
        for line in f:
            if line.strip():
                data.append(json.loads(line))
    return data


def parse_timestamp(ts: str) -> datetime:
    """Parse ISO timestamp."""
    # Handle various formats
    ts = ts.replace("Z", "+00:00")
    if "." in ts:
        # Truncate microseconds if too long
        parts = ts.split(".")
        if "+" in parts[1]:
            micro, tz = parts[1].split("+")
            ts = f"{parts[0]}.{micro[:6]}+{tz}"
        elif "-" in parts[1]:
            micro, tz = parts[1].split("-")
            ts = f"{parts[0]}.{micro[:6]}-{tz}"

    try:
        return datetime.fromisoformat(ts)
    except:
        return datetime.now()


class FeatureExtractor:
    """Extract ML features from raw trading data."""

    def __init__(self, data_dir: Path = DATA_DIR):
        self.data_dir = data_dir
        self.snapshots: List[Dict] = []
        self.fills: List[Dict] = []
        self.orders: List[Dict] = []
        self.summaries: List[Dict] = []

    def load_session(self, session_id: str):
        """Load all data for a session."""
        self.snapshots = load_jsonl(self.data_dir / f"snapshots_{session_id}.jsonl")
        self.fills = load_jsonl(self.data_dir / f"fills_{session_id}.jsonl")
        self.orders = load_jsonl(self.data_dir / f"orders_{session_id}.jsonl")
        print(f"Loaded session {session_id}: {len(self.snapshots)} snapshots, {len(self.fills)} fills, {len(self.orders)} orders")

    def load_all_sessions(self):
        """Load all available sessions."""
        self.summaries = load_jsonl(self.data_dir / "summaries.jsonl")

        all_snapshots = []
        all_fills = []
        all_orders = []

        # Find all session files
        for f in self.data_dir.glob("snapshots_*.jsonl"):
            session_id = f.stem.replace("snapshots_", "")
            self.load_session(session_id)
            all_snapshots.extend(self.snapshots)
            all_fills.extend(self.fills)
            all_orders.extend(self.orders)

        self.snapshots = all_snapshots
        self.fills = all_fills
        self.orders = all_orders

        print(f"Total: {len(self.snapshots)} snapshots, {len(self.fills)} fills, {len(self.summaries)} sessions")

    def extract_spread_features(self) -> List[Dict]:
        """
        Extract features for spread prediction model.

        Target: predict spread_pct N seconds in the future
        Features: current spread, recent spread history, time to resolution, etc.
        """
        features = []

        # Sort by timestamp
        sorted_snaps = sorted(self.snapshots, key=lambda x: x.get('timestamp', ''))

        # Group by market
        markets = {}
        for snap in sorted_snaps:
            mid = snap.get('market_id', 'unknown')
            if mid not in markets:
                markets[mid] = []
            markets[mid].append(snap)

        # Extract features for each market
        for market_id, snaps in markets.items():
            if len(snaps) < 10:
                continue

            for i in range(10, len(snaps) - 5):
                current = snaps[i]

                # Current state
                spread_now = float(current.get('spread_pct', 0) or 0)
                up_ask = float(current.get('up_best_ask', 0.5) or 0.5)
                down_ask = float(current.get('down_best_ask', 0.5) or 0.5)
                combined = float(current.get('combined_ask', 1.0) or 1.0)

                # Time features
                ts = parse_timestamp(current.get('timestamp', ''))
                end_time = parse_timestamp(current.get('end_time', ''))
                seconds_to_resolution = (end_time - ts).total_seconds()
                minute_of_period = (15 * 60 - seconds_to_resolution) / 60  # 0-15

                # Historical features (last 10 snapshots)
                recent_spreads = [float(snaps[j].get('spread_pct', 0) or 0) for j in range(i-10, i)]
                spread_mean = sum(recent_spreads) / len(recent_spreads)
                spread_max = max(recent_spreads)
                spread_min = min(recent_spreads)
                spread_volatility = (sum((s - spread_mean)**2 for s in recent_spreads) / len(recent_spreads)) ** 0.5
                spread_trend = recent_spreads[-1] - recent_spreads[0]  # positive = increasing

                # Price movement
                recent_up_asks = [float(snaps[j].get('up_best_ask', 0.5) or 0.5) for j in range(i-10, i)]
                recent_down_asks = [float(snaps[j].get('down_best_ask', 0.5) or 0.5) for j in range(i-10, i)]
                up_trend = recent_up_asks[-1] - recent_up_asks[0]
                down_trend = recent_down_asks[-1] - recent_down_asks[0]

                # Target: spread 5 snapshots ahead
                future_spread = float(snaps[i + 5].get('spread_pct', 0) or 0)
                spread_change = future_spread - spread_now
                spread_increased = 1 if spread_change > 0.5 else 0  # Binary classification

                features.append({
                    # Identifiers
                    'market_id': market_id,
                    'timestamp': current.get('timestamp'),

                    # Current state features
                    'spread_now': spread_now,
                    'up_ask': up_ask,
                    'down_ask': down_ask,
                    'combined_ask': combined,

                    # Time features
                    'seconds_to_resolution': seconds_to_resolution,
                    'minute_of_period': minute_of_period,

                    # Historical features
                    'spread_mean_10': spread_mean,
                    'spread_max_10': spread_max,
                    'spread_min_10': spread_min,
                    'spread_volatility_10': spread_volatility,
                    'spread_trend_10': spread_trend,
                    'up_trend_10': up_trend,
                    'down_trend_10': down_trend,

                    # Targets
                    'future_spread': future_spread,
                    'spread_change': spread_change,
                    'spread_increased': spread_increased,  # For classification
                })

        return features

    def extract_fill_features(self) -> List[Dict]:
        """
        Extract features for fill prediction model.

        Target: did an order at this price level get filled?
        Features: price relative to best ask, spread, time, etc.
        """
        features = []

        for order in self.orders:
            ts = parse_timestamp(order.get('timestamp', ''))
            market_id = order.get('market_id', '')
            price = float(order.get('price', 0))
            side = order.get('side', '')  # UP or DOWN

            # Find nearest snapshot
            nearest_snap = None
            min_diff = float('inf')
            for snap in self.snapshots:
                if snap.get('market_id') != market_id:
                    continue
                snap_ts = parse_timestamp(snap.get('timestamp', ''))
                diff = abs((snap_ts - ts).total_seconds())
                if diff < min_diff:
                    min_diff = diff
                    nearest_snap = snap

            if not nearest_snap or min_diff > 60:
                continue

            # Get market state at order time
            if side == 'UP':
                best_ask = float(nearest_snap.get('up_best_ask', 0.5) or 0.5)
            else:
                best_ask = float(nearest_snap.get('down_best_ask', 0.5) or 0.5)

            spread_pct = float(nearest_snap.get('spread_pct', 0) or 0)

            # Calculate features
            price_vs_ask = price - best_ask  # Negative = below ask (more likely to fill)
            price_vs_ask_pct = (price_vs_ask / best_ask) * 100 if best_ask > 0 else 0

            end_time = parse_timestamp(nearest_snap.get('end_time', ''))
            seconds_to_resolution = (end_time - ts).total_seconds()

            # Check if this order was filled
            was_filled = 0
            for fill in self.fills:
                if fill.get('market_id') == market_id and fill.get('side') == side:
                    fill_price = float(fill.get('price', 0))
                    if abs(fill_price - price) < 0.01:  # Same price level
                        was_filled = 1
                        break

            features.append({
                'market_id': market_id,
                'timestamp': order.get('timestamp'),
                'side': side,
                'order_price': price,
                'best_ask': best_ask,
                'price_vs_ask': price_vs_ask,
                'price_vs_ask_pct': price_vs_ask_pct,
                'spread_pct': spread_pct,
                'seconds_to_resolution': seconds_to_resolution,
                'was_filled': was_filled,  # Target
            })

        return features

    def extract_session_features(self) -> List[Dict]:
        """
        Extract features for session profitability prediction.

        Target: was this session profitable?
        Features: entry spread, time of day, volatility, etc.
        """
        features = []

        for summary in self.summaries:
            profit = float(summary.get('locked_profit', 0) or 0)
            profit_pct = float(summary.get('profit_pct', 0) or 0)
            total_cost = float(summary.get('total_cost', 0) or 0)

            if total_cost == 0:
                continue

            # Time features
            start = parse_timestamp(summary.get('start_time', ''))
            hour_of_day = start.hour
            day_of_week = start.weekday()

            # Position balance
            up_shares = float(summary.get('total_up_shares', 0) or 0)
            down_shares = float(summary.get('total_down_shares', 0) or 0)
            balance_ratio = min(up_shares, down_shares) / max(up_shares, down_shares) if max(up_shares, down_shares) > 0 else 0

            # Activity
            orders_placed = int(summary.get('orders_placed', 0) or 0)
            fills_received = int(summary.get('fills_received', 0) or 0)
            fill_rate = fills_received / orders_placed if orders_placed > 0 else 0

            features.append({
                'session_id': summary.get('session_id'),
                'market_id': summary.get('market_id'),
                'hour_of_day': hour_of_day,
                'day_of_week': day_of_week,
                'total_cost': total_cost,
                'up_shares': up_shares,
                'down_shares': down_shares,
                'balance_ratio': balance_ratio,
                'orders_placed': orders_placed,
                'fills_received': fills_received,
                'fill_rate': fill_rate,
                'is_dry_run': 1 if summary.get('is_dry_run') else 0,

                # Targets
                'profit': profit,
                'profit_pct': profit_pct,
                'is_profitable': 1 if profit > 0 else 0,
            })

        return features

    def extract_timing_features(self) -> List[Dict]:
        """
        Extract features for optimal entry timing.

        Target: what's the best minute (0-15) to enter?
        Features: historical spread patterns by minute
        """
        features = []

        # Group snapshots by minute within the 15-min period
        minute_data = {i: [] for i in range(16)}

        for snap in self.snapshots:
            ts = parse_timestamp(snap.get('timestamp', ''))
            end_time = parse_timestamp(snap.get('end_time', ''))
            seconds_to_resolution = (end_time - ts).total_seconds()

            if seconds_to_resolution < 0 or seconds_to_resolution > 15 * 60:
                continue

            minute = int((15 * 60 - seconds_to_resolution) / 60)
            minute = max(0, min(14, minute))

            spread = float(snap.get('spread_pct', 0) or 0)
            minute_data[minute].append(spread)

        # Calculate stats per minute
        for minute, spreads in minute_data.items():
            if len(spreads) < 5:
                continue

            features.append({
                'minute': minute,
                'sample_count': len(spreads),
                'spread_mean': sum(spreads) / len(spreads),
                'spread_max': max(spreads),
                'spread_min': min(spreads),
                'spread_std': (sum((s - sum(spreads)/len(spreads))**2 for s in spreads) / len(spreads)) ** 0.5,
                'spreads_above_4pct': sum(1 for s in spreads if s >= 4) / len(spreads),
                'spreads_above_5pct': sum(1 for s in spreads if s >= 5) / len(spreads),
            })

        return features

    def save_features(self, features: List[Dict], name: str):
        """Save features to CSV."""
        if not features:
            print(f"No features to save for {name}")
            return

        OUTPUT_DIR.mkdir(exist_ok=True)
        filepath = OUTPUT_DIR / f"{name}.csv"

        fieldnames = list(features[0].keys())
        with open(filepath, 'w', newline='') as f:
            writer = csv.DictWriter(f, fieldnames=fieldnames)
            writer.writeheader()
            writer.writerows(features)

        print(f"Saved {len(features)} rows to {filepath}")

    def run_full_extraction(self):
        """Run all feature extractions."""
        print("Loading all sessions...")
        self.load_all_sessions()

        print("\nExtracting spread prediction features...")
        spread_features = self.extract_spread_features()
        self.save_features(spread_features, "spread_features")

        print("\nExtracting fill prediction features...")
        fill_features = self.extract_fill_features()
        self.save_features(fill_features, "fill_features")

        print("\nExtracting session profitability features...")
        session_features = self.extract_session_features()
        self.save_features(session_features, "session_features")

        print("\nExtracting timing features...")
        timing_features = self.extract_timing_features()
        self.save_features(timing_features, "timing_features")

        print("\nDone! Feature files saved to ml/features/")


if __name__ == "__main__":
    extractor = FeatureExtractor()
    extractor.run_full_extraction()
