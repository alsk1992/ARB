#!/usr/bin/env python3
"""
Auto-training daemon that watches for new data and retrains models.

Run alongside the bot:
  python auto_train.py

It will:
1. Watch ./data/ for new session summaries
2. Retrain models when enough new data arrives
3. Update predictions automatically
"""

import json
import time
import os
from pathlib import Path
from datetime import datetime
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler

# Import our training modules
from extract_features import FeatureExtractor
from train_models import SpreadPredictor, FillPredictor, TimingOptimizer

DATA_DIR = Path(__file__).parent.parent / "data"
MODELS_DIR = Path(__file__).parent / "models"
FEATURES_DIR = Path(__file__).parent / "features"
STATE_FILE = Path(__file__).parent / ".training_state.json"

# Minimum sessions before training
MIN_SESSIONS_FOR_TRAINING = 5
# Retrain after this many new sessions
RETRAIN_INTERVAL = 3


class TrainingState:
    """Track training state."""

    def __init__(self):
        self.last_trained_sessions = 0
        self.last_trained_time = None
        self.load()

    def load(self):
        if STATE_FILE.exists():
            with open(STATE_FILE) as f:
                data = json.load(f)
                self.last_trained_sessions = data.get('last_trained_sessions', 0)
                self.last_trained_time = data.get('last_trained_time')

    def save(self, sessions_count: int):
        self.last_trained_sessions = sessions_count
        self.last_trained_time = datetime.now().isoformat()
        with open(STATE_FILE, 'w') as f:
            json.dump({
                'last_trained_sessions': self.last_trained_sessions,
                'last_trained_time': self.last_trained_time
            }, f)


class AutoTrainer:
    """Automatically retrain models when new data arrives."""

    def __init__(self):
        self.state = TrainingState()
        self.extractor = FeatureExtractor()
        MODELS_DIR.mkdir(exist_ok=True)
        FEATURES_DIR.mkdir(exist_ok=True)

    def count_sessions(self) -> int:
        """Count total sessions in summaries file."""
        summaries_file = DATA_DIR / "summaries.jsonl"
        if not summaries_file.exists():
            return 0

        count = 0
        with open(summaries_file) as f:
            for line in f:
                if line.strip():
                    count += 1
        return count

    def should_retrain(self) -> bool:
        """Check if we should retrain models."""
        current_sessions = self.count_sessions()

        # Not enough data yet
        if current_sessions < MIN_SESSIONS_FOR_TRAINING:
            print(f"Only {current_sessions} sessions, need {MIN_SESSIONS_FOR_TRAINING} minimum")
            return False

        # Check if enough new sessions since last training
        new_sessions = current_sessions - self.state.last_trained_sessions
        if new_sessions >= RETRAIN_INTERVAL:
            print(f"{new_sessions} new sessions since last training, retraining...")
            return True

        print(f"Only {new_sessions} new sessions, need {RETRAIN_INTERVAL} to retrain")
        return False

    def retrain(self):
        """Run full retraining pipeline."""
        print("\n" + "=" * 60)
        print(f"AUTO-TRAINING STARTED - {datetime.now().isoformat()}")
        print("=" * 60)

        try:
            # Step 1: Extract features
            print("\n[1/3] Extracting features...")
            self.extractor.load_all_sessions()

            spread_features = self.extractor.extract_spread_features()
            self.extractor.save_features(spread_features, "spread_features")

            fill_features = self.extractor.extract_fill_features()
            self.extractor.save_features(fill_features, "fill_features")

            timing_features = self.extractor.extract_timing_features()
            self.extractor.save_features(timing_features, "timing_features")

            session_features = self.extractor.extract_session_features()
            self.extractor.save_features(session_features, "session_features")

            # Step 2: Train models
            print("\n[2/3] Training models...")

            # Spread predictor
            spread_file = FEATURES_DIR / "spread_features.csv"
            if spread_file.exists() and os.path.getsize(spread_file) > 100:
                spread_model = SpreadPredictor()
                spread_model.train(spread_file)
                spread_model.save(MODELS_DIR / "spread_predictor.pkl")

            # Fill predictor
            fill_file = FEATURES_DIR / "fill_features.csv"
            if fill_file.exists() and os.path.getsize(fill_file) > 100:
                fill_model = FillPredictor()
                fill_model.train(fill_file)
                fill_model.save(MODELS_DIR / "fill_predictor.pkl")

            # Timing analysis
            timing_file = FEATURES_DIR / "timing_features.csv"
            if timing_file.exists():
                timing = TimingOptimizer()
                timing.analyze(timing_file)
                with open(MODELS_DIR / "timing_recommendations.json", 'w') as f:
                    json.dump(timing.get_recommendation(), f, indent=2)

            # Step 3: Update state
            print("\n[3/3] Saving state...")
            current_sessions = self.count_sessions()
            self.state.save(current_sessions)

            print("\n" + "=" * 60)
            print("AUTO-TRAINING COMPLETE")
            print(f"Models updated with {current_sessions} sessions of data")
            print("=" * 60 + "\n")

        except Exception as e:
            print(f"\nERROR during training: {e}")
            import traceback
            traceback.print_exc()

    def check_and_train(self):
        """Check if training needed and run if so."""
        if self.should_retrain():
            self.retrain()


class DataEventHandler(FileSystemEventHandler):
    """Watch for new data files."""

    def __init__(self, trainer: AutoTrainer):
        self.trainer = trainer
        self.last_check = 0
        self.check_interval = 60  # Don't check more than once per minute

    def on_modified(self, event):
        if event.src_path.endswith("summaries.jsonl"):
            now = time.time()
            if now - self.last_check > self.check_interval:
                self.last_check = now
                print(f"\nNew session data detected...")
                self.trainer.check_and_train()


def run_daemon():
    """Run the auto-training daemon."""
    print("=" * 60)
    print("AUTO-TRAINING DAEMON STARTED")
    print("=" * 60)
    print(f"Watching: {DATA_DIR}")
    print(f"Models:   {MODELS_DIR}")
    print(f"Min sessions: {MIN_SESSIONS_FOR_TRAINING}")
    print(f"Retrain interval: {RETRAIN_INTERVAL} new sessions")
    print("=" * 60 + "\n")

    trainer = AutoTrainer()

    # Initial check
    trainer.check_and_train()

    # Watch for changes
    event_handler = DataEventHandler(trainer)
    observer = Observer()

    DATA_DIR.mkdir(exist_ok=True)
    observer.schedule(event_handler, str(DATA_DIR), recursive=False)
    observer.start()

    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        observer.stop()
        print("\nAuto-trainer stopped.")

    observer.join()


def run_once():
    """Run training once and exit."""
    trainer = AutoTrainer()
    trainer.retrain()


if __name__ == "__main__":
    import sys

    if len(sys.argv) > 1 and sys.argv[1] == "--once":
        run_once()
    else:
        # Need watchdog for daemon mode
        try:
            from watchdog.observers import Observer
            from watchdog.events import FileSystemEventHandler
            run_daemon()
        except ImportError:
            print("For daemon mode, install watchdog: pip install watchdog")
            print("Running once instead...")
            run_once()
