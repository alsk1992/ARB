#!/usr/bin/env python3
"""
Train ML models on extracted features.

Models:
1. Spread Predictor - Predict when spreads will widen (entry timing)
2. Fill Predictor - Predict fill probability at different price levels
3. Session Classifier - Predict if market conditions are profitable

Uses XGBoost for best performance on tabular data.
"""

import json
import pickle
from pathlib import Path
from typing import Dict, Any, Tuple
import csv

# Check for required packages
try:
    import numpy as np
    from sklearn.model_selection import train_test_split, cross_val_score
    from sklearn.metrics import (
        accuracy_score, precision_score, recall_score, f1_score,
        mean_squared_error, mean_absolute_error, r2_score,
        classification_report, confusion_matrix
    )
    from sklearn.preprocessing import StandardScaler
    import xgboost as xgb
except ImportError:
    print("Required packages not installed. Run:")
    print("  pip install numpy scikit-learn xgboost")
    exit(1)

FEATURES_DIR = Path(__file__).parent / "features"
MODELS_DIR = Path(__file__).parent / "models"


def load_csv(filepath: Path) -> Tuple[list, list]:
    """Load CSV and return headers + rows."""
    with open(filepath) as f:
        reader = csv.DictReader(f)
        rows = list(reader)
    return rows


def prepare_data(rows: list, feature_cols: list, target_col: str) -> Tuple[np.ndarray, np.ndarray]:
    """Prepare feature matrix and target vector."""
    X = []
    y = []

    for row in rows:
        try:
            features = [float(row.get(col, 0) or 0) for col in feature_cols]
            target = float(row.get(target_col, 0) or 0)
            X.append(features)
            y.append(target)
        except (ValueError, TypeError):
            continue

    return np.array(X), np.array(y)


class SpreadPredictor:
    """
    Predict spread changes.

    Classification: Will spread increase by >0.5% in next N updates?
    Regression: What will the spread be?
    """

    def __init__(self):
        self.classifier = None
        self.regressor = None
        self.scaler = StandardScaler()
        self.feature_cols = [
            'spread_now', 'up_ask', 'down_ask', 'combined_ask',
            'seconds_to_resolution', 'minute_of_period',
            'spread_mean_10', 'spread_max_10', 'spread_min_10',
            'spread_volatility_10', 'spread_trend_10',
            'up_trend_10', 'down_trend_10'
        ]

    def train(self, filepath: Path):
        """Train both classifier and regressor."""
        print("Loading spread features...")
        rows = load_csv(filepath)
        print(f"Loaded {len(rows)} samples")

        if len(rows) < 100:
            print("Not enough data to train. Need at least 100 samples.")
            return

        # Prepare data for classification
        X, y_class = prepare_data(rows, self.feature_cols, 'spread_increased')
        X_scaled = self.scaler.fit_transform(X)

        # Split
        X_train, X_test, y_train, y_test = train_test_split(
            X_scaled, y_class, test_size=0.2, random_state=42
        )

        # Train classifier
        print("\nTraining spread increase classifier...")
        self.classifier = xgb.XGBClassifier(
            n_estimators=100,
            max_depth=5,
            learning_rate=0.1,
            random_state=42,
            use_label_encoder=False,
            eval_metric='logloss'
        )
        self.classifier.fit(X_train, y_train)

        # Evaluate classifier
        y_pred = self.classifier.predict(X_test)
        print(f"Accuracy: {accuracy_score(y_test, y_pred):.3f}")
        print(f"Precision: {precision_score(y_test, y_pred, zero_division=0):.3f}")
        print(f"Recall: {recall_score(y_test, y_pred, zero_division=0):.3f}")
        print(f"F1: {f1_score(y_test, y_pred, zero_division=0):.3f}")

        # Feature importance
        print("\nTop features for spread prediction:")
        importances = list(zip(self.feature_cols, self.classifier.feature_importances_))
        importances.sort(key=lambda x: x[1], reverse=True)
        for feat, imp in importances[:5]:
            print(f"  {feat}: {imp:.3f}")

        # Train regressor
        print("\nTraining spread value regressor...")
        _, y_reg = prepare_data(rows, self.feature_cols, 'future_spread')
        X_train_r, X_test_r, y_train_r, y_test_r = train_test_split(
            X_scaled, y_reg, test_size=0.2, random_state=42
        )

        self.regressor = xgb.XGBRegressor(
            n_estimators=100,
            max_depth=5,
            learning_rate=0.1,
            random_state=42
        )
        self.regressor.fit(X_train_r, y_train_r)

        # Evaluate regressor
        y_pred_r = self.regressor.predict(X_test_r)
        print(f"MAE: {mean_absolute_error(y_test_r, y_pred_r):.3f}")
        print(f"RMSE: {np.sqrt(mean_squared_error(y_test_r, y_pred_r)):.3f}")
        print(f"R2: {r2_score(y_test_r, y_pred_r):.3f}")

    def predict(self, features: Dict[str, float]) -> Dict[str, Any]:
        """Make prediction for new data."""
        X = np.array([[features.get(col, 0) for col in self.feature_cols]])
        X_scaled = self.scaler.transform(X)

        return {
            'spread_will_increase': bool(self.classifier.predict(X_scaled)[0]),
            'spread_increase_prob': float(self.classifier.predict_proba(X_scaled)[0][1]),
            'predicted_spread': float(self.regressor.predict(X_scaled)[0])
        }

    def save(self, filepath: Path):
        """Save trained model."""
        with open(filepath, 'wb') as f:
            pickle.dump({
                'classifier': self.classifier,
                'regressor': self.regressor,
                'scaler': self.scaler,
                'feature_cols': self.feature_cols
            }, f)
        print(f"Saved spread predictor to {filepath}")

    def load(self, filepath: Path):
        """Load trained model."""
        with open(filepath, 'rb') as f:
            data = pickle.load(f)
        self.classifier = data['classifier']
        self.regressor = data['regressor']
        self.scaler = data['scaler']
        self.feature_cols = data['feature_cols']


class FillPredictor:
    """
    Predict probability of order fill.

    Given price level relative to best ask, predict if order will fill.
    """

    def __init__(self):
        self.model = None
        self.scaler = StandardScaler()
        self.feature_cols = [
            'order_price', 'best_ask', 'price_vs_ask', 'price_vs_ask_pct',
            'spread_pct', 'seconds_to_resolution'
        ]

    def train(self, filepath: Path):
        """Train fill predictor."""
        print("Loading fill features...")
        rows = load_csv(filepath)
        print(f"Loaded {len(rows)} samples")

        if len(rows) < 50:
            print("Not enough data to train. Need at least 50 samples.")
            return

        X, y = prepare_data(rows, self.feature_cols, 'was_filled')
        X_scaled = self.scaler.fit_transform(X)

        X_train, X_test, y_train, y_test = train_test_split(
            X_scaled, y, test_size=0.2, random_state=42
        )

        print("\nTraining fill predictor...")
        self.model = xgb.XGBClassifier(
            n_estimators=100,
            max_depth=4,
            learning_rate=0.1,
            random_state=42,
            use_label_encoder=False,
            eval_metric='logloss'
        )
        self.model.fit(X_train, y_train)

        y_pred = self.model.predict(X_test)
        print(f"Accuracy: {accuracy_score(y_test, y_pred):.3f}")
        print(f"Precision: {precision_score(y_test, y_pred, zero_division=0):.3f}")
        print(f"Recall: {recall_score(y_test, y_pred, zero_division=0):.3f}")

        print("\nFeature importance:")
        importances = list(zip(self.feature_cols, self.model.feature_importances_))
        importances.sort(key=lambda x: x[1], reverse=True)
        for feat, imp in importances:
            print(f"  {feat}: {imp:.3f}")

    def predict(self, features: Dict[str, float]) -> Dict[str, Any]:
        """Predict fill probability."""
        X = np.array([[features.get(col, 0) for col in self.feature_cols]])
        X_scaled = self.scaler.transform(X)

        return {
            'will_fill': bool(self.model.predict(X_scaled)[0]),
            'fill_probability': float(self.model.predict_proba(X_scaled)[0][1])
        }

    def save(self, filepath: Path):
        with open(filepath, 'wb') as f:
            pickle.dump({
                'model': self.model,
                'scaler': self.scaler,
                'feature_cols': self.feature_cols
            }, f)
        print(f"Saved fill predictor to {filepath}")

    def load(self, filepath: Path):
        with open(filepath, 'rb') as f:
            data = pickle.load(f)
        self.model = data['model']
        self.scaler = data['scaler']
        self.feature_cols = data['feature_cols']


class TimingOptimizer:
    """
    Analyze timing data to find optimal entry windows.

    Not ML per se, but statistical analysis of when spreads are best.
    """

    def __init__(self):
        self.timing_stats = {}

    def analyze(self, filepath: Path):
        """Analyze timing features."""
        print("Loading timing features...")
        rows = load_csv(filepath)

        if not rows:
            print("No timing data available.")
            return

        print("\nOptimal Entry Timing Analysis")
        print("=" * 50)
        print(f"{'Minute':<10} {'Avg Spread':<12} {'Max Spread':<12} {'>4%':<10} {'>5%':<10}")
        print("-" * 50)

        best_minute = None
        best_spread = 0

        for row in sorted(rows, key=lambda x: int(x.get('minute', 0))):
            minute = int(row.get('minute', 0))
            avg_spread = float(row.get('spread_mean', 0))
            max_spread = float(row.get('spread_max', 0))
            above_4 = float(row.get('spreads_above_4pct', 0)) * 100
            above_5 = float(row.get('spreads_above_5pct', 0)) * 100

            print(f"{minute:<10} {avg_spread:<12.2f} {max_spread:<12.2f} {above_4:<10.1f}% {above_5:<10.1f}%")

            if avg_spread > best_spread:
                best_spread = avg_spread
                best_minute = minute

            self.timing_stats[minute] = {
                'avg_spread': avg_spread,
                'max_spread': max_spread,
                'above_4pct': above_4,
                'above_5pct': above_5
            }

        print("-" * 50)
        if best_minute is not None:
            print(f"\nBest entry window: Minute {best_minute} (avg spread: {best_spread:.2f}%)")
            print(f"Recommendation: Place orders around minute {best_minute} of the 15-min period")

    def get_recommendation(self) -> Dict[str, Any]:
        """Get timing recommendation."""
        if not self.timing_stats:
            return {'recommendation': 'Not enough data'}

        best_minute = max(self.timing_stats.keys(), key=lambda m: self.timing_stats[m]['avg_spread'])
        return {
            'best_minute': best_minute,
            'avg_spread': self.timing_stats[best_minute]['avg_spread'],
            'stats': self.timing_stats
        }


def train_all_models():
    """Train all models."""
    MODELS_DIR.mkdir(exist_ok=True)

    # Spread predictor
    print("\n" + "=" * 60)
    print("SPREAD PREDICTOR")
    print("=" * 60)
    spread_file = FEATURES_DIR / "spread_features.csv"
    if spread_file.exists():
        spread_model = SpreadPredictor()
        spread_model.train(spread_file)
        spread_model.save(MODELS_DIR / "spread_predictor.pkl")
    else:
        print(f"No spread features found at {spread_file}")

    # Fill predictor
    print("\n" + "=" * 60)
    print("FILL PREDICTOR")
    print("=" * 60)
    fill_file = FEATURES_DIR / "fill_features.csv"
    if fill_file.exists():
        fill_model = FillPredictor()
        fill_model.train(fill_file)
        fill_model.save(MODELS_DIR / "fill_predictor.pkl")
    else:
        print(f"No fill features found at {fill_file}")

    # Timing analysis
    print("\n" + "=" * 60)
    print("TIMING ANALYSIS")
    print("=" * 60)
    timing_file = FEATURES_DIR / "timing_features.csv"
    if timing_file.exists():
        timing = TimingOptimizer()
        timing.analyze(timing_file)

        # Save timing recommendations
        with open(MODELS_DIR / "timing_recommendations.json", 'w') as f:
            json.dump(timing.get_recommendation(), f, indent=2)
        print(f"Saved timing recommendations to {MODELS_DIR / 'timing_recommendations.json'}")
    else:
        print(f"No timing features found at {timing_file}")

    print("\n" + "=" * 60)
    print("TRAINING COMPLETE")
    print("=" * 60)
    print(f"\nModels saved to: {MODELS_DIR}/")
    print("\nTo use in bot, the Rust code can call these Python models via subprocess")
    print("or we can export to ONNX format for native Rust inference.")


if __name__ == "__main__":
    train_all_models()
