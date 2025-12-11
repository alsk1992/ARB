#!/usr/bin/env python3
"""
Prediction server for the trading bot.

Can be called as:
1. CLI: python predict.py '{"spread_now": 3.5, "seconds_to_resolution": 600, ...}'
2. HTTP server: python predict.py --serve (for low-latency predictions)

Returns JSON with predictions.
"""

import json
import sys
import pickle
from pathlib import Path
from typing import Dict, Any, Optional

MODELS_DIR = Path(__file__).parent / "models"


class PredictionEngine:
    """Load models and make predictions."""

    def __init__(self):
        self.spread_predictor = None
        self.fill_predictor = None
        self.timing_recommendations = None
        self._load_models()

    def _load_models(self):
        """Load all trained models."""
        # Spread predictor
        spread_path = MODELS_DIR / "spread_predictor.pkl"
        if spread_path.exists():
            with open(spread_path, 'rb') as f:
                data = pickle.load(f)
            self.spread_predictor = data
            print(f"Loaded spread predictor", file=sys.stderr)

        # Fill predictor
        fill_path = MODELS_DIR / "fill_predictor.pkl"
        if fill_path.exists():
            with open(fill_path, 'rb') as f:
                data = pickle.load(f)
            self.fill_predictor = data
            print(f"Loaded fill predictor", file=sys.stderr)

        # Timing recommendations
        timing_path = MODELS_DIR / "timing_recommendations.json"
        if timing_path.exists():
            with open(timing_path) as f:
                self.timing_recommendations = json.load(f)
            print(f"Loaded timing recommendations", file=sys.stderr)

    def predict_spread(self, features: Dict[str, float]) -> Optional[Dict[str, Any]]:
        """Predict spread movement."""
        if not self.spread_predictor:
            return None

        try:
            import numpy as np

            feature_cols = self.spread_predictor['feature_cols']
            X = np.array([[features.get(col, 0) for col in feature_cols]])
            X_scaled = self.spread_predictor['scaler'].transform(X)

            classifier = self.spread_predictor['classifier']
            regressor = self.spread_predictor['regressor']

            return {
                'spread_will_increase': bool(classifier.predict(X_scaled)[0]),
                'spread_increase_prob': float(classifier.predict_proba(X_scaled)[0][1]),
                'predicted_spread': float(regressor.predict(X_scaled)[0])
            }
        except Exception as e:
            return {'error': str(e)}

    def predict_fill(self, features: Dict[str, float]) -> Optional[Dict[str, Any]]:
        """Predict fill probability."""
        if not self.fill_predictor:
            return None

        try:
            import numpy as np

            feature_cols = self.fill_predictor['feature_cols']
            X = np.array([[features.get(col, 0) for col in feature_cols]])
            X_scaled = self.fill_predictor['scaler'].transform(X)

            model = self.fill_predictor['model']

            return {
                'will_fill': bool(model.predict(X_scaled)[0]),
                'fill_probability': float(model.predict_proba(X_scaled)[0][1])
            }
        except Exception as e:
            return {'error': str(e)}

    def get_timing_recommendation(self) -> Optional[Dict[str, Any]]:
        """Get optimal entry timing."""
        return self.timing_recommendations

    def predict_all(self, features: Dict[str, float]) -> Dict[str, Any]:
        """Run all predictions."""
        result = {
            'spread': self.predict_spread(features),
            'timing': self.get_timing_recommendation()
        }

        # Only predict fill if we have order-specific features
        if 'order_price' in features:
            result['fill'] = self.predict_fill(features)

        return result


def run_cli(input_json: str):
    """Run prediction from CLI."""
    try:
        features = json.loads(input_json)
    except json.JSONDecodeError as e:
        print(json.dumps({'error': f'Invalid JSON: {e}'}))
        return

    engine = PredictionEngine()
    result = engine.predict_all(features)
    print(json.dumps(result, indent=2))


def run_server(port: int = 8765):
    """Run HTTP prediction server for low-latency predictions."""
    try:
        from http.server import HTTPServer, BaseHTTPRequestHandler
    except ImportError:
        print("http.server not available", file=sys.stderr)
        return

    engine = PredictionEngine()

    class PredictHandler(BaseHTTPRequestHandler):
        def do_POST(self):
            content_length = int(self.headers['Content-Length'])
            body = self.rfile.read(content_length)

            try:
                features = json.loads(body)
                result = engine.predict_all(features)
                response = json.dumps(result)
                self.send_response(200)
            except Exception as e:
                response = json.dumps({'error': str(e)})
                self.send_response(400)

            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            self.wfile.write(response.encode())

        def log_message(self, format, *args):
            pass  # Suppress logging for speed

    print(f"Starting prediction server on port {port}...", file=sys.stderr)
    server = HTTPServer(('127.0.0.1', port), PredictHandler)
    server.serve_forever()


def main():
    if len(sys.argv) < 2:
        print("Usage:")
        print("  python predict.py '{\"spread_now\": 3.5, ...}'  # CLI prediction")
        print("  python predict.py --serve                       # Start HTTP server")
        print("  python predict.py --serve 8888                  # Custom port")
        return

    if sys.argv[1] == '--serve':
        port = int(sys.argv[2]) if len(sys.argv) > 2 else 8765
        run_server(port)
    else:
        run_cli(sys.argv[1])


if __name__ == "__main__":
    main()
