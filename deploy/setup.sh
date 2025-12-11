#!/bin/bash
# AWS EC2 Setup Script for BTC Arb Bot
# Run this on a fresh Ubuntu 22.04 instance in us-east-1

set -e

echo "==================================="
echo "  BTC Arb Bot - Server Setup"
echo "==================================="

# Update system
echo "[1/9] Updating system..."
sudo apt-get update -y
sudo apt-get upgrade -y

# Install dependencies
echo "[2/9] Installing dependencies..."
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    curl \
    git \
    htop \
    tmux \
    python3 \
    python3-pip \
    python3-venv

# Install Rust
echo "[3/9] Installing Rust..."
if ! command -v rustc &> /dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
fi

# Clone or update repo
echo "[4/9] Setting up bot code..."
BOT_DIR="$HOME/btc-arb-bot"
if [ -d "$BOT_DIR" ]; then
    cd $BOT_DIR
    git pull 2>/dev/null || true
else
    mkdir -p $BOT_DIR
    echo "Please copy bot files to $BOT_DIR"
fi

# Build release
echo "[5/9] Building release binary..."
cd $BOT_DIR
source $HOME/.cargo/env
cargo build --release

# Setup Python ML environment
echo "[6/9] Setting up ML environment..."
cd $BOT_DIR/ml
python3 -m venv venv
source venv/bin/activate
pip install --upgrade pip
pip install -r requirements.txt
deactivate

# Setup bot systemd service
echo "[7/9] Setting up bot systemd service..."
sudo tee /etc/systemd/system/btc-arb-bot.service > /dev/null <<EOF
[Unit]
Description=BTC Arbitrage Bot
After=network.target btc-ml-trainer.service
Wants=btc-ml-trainer.service btc-ml-predictor.service

[Service]
Type=simple
User=$USER
WorkingDirectory=$BOT_DIR
ExecStart=$BOT_DIR/target/release/btc-arb-bot
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# Setup ML auto-trainer service
echo "[8/9] Setting up ML auto-trainer service..."
sudo tee /etc/systemd/system/btc-ml-trainer.service > /dev/null <<EOF
[Unit]
Description=BTC Bot ML Auto-Trainer
After=network.target

[Service]
Type=simple
User=$USER
WorkingDirectory=$BOT_DIR/ml
ExecStart=$BOT_DIR/ml/venv/bin/python auto_train.py
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

# Setup ML prediction server service
sudo tee /etc/systemd/system/btc-ml-predictor.service > /dev/null <<EOF
[Unit]
Description=BTC Bot ML Prediction Server
After=network.target

[Service]
Type=simple
User=$USER
WorkingDirectory=$BOT_DIR/ml
ExecStart=$BOT_DIR/ml/venv/bin/python predict.py --serve 8765
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable btc-arb-bot
sudo systemctl enable btc-ml-trainer
sudo systemctl enable btc-ml-predictor

# Setup .env if not exists
echo "[9/9] Checking configuration..."
if [ ! -f "$BOT_DIR/.env" ]; then
    echo "WARNING: .env file not found!"
    echo "Please create $BOT_DIR/.env with your credentials"
    cp $BOT_DIR/.env.example $BOT_DIR/.env 2>/dev/null || true
fi

echo ""
echo "==================================="
echo "  Setup Complete!"
echo "==================================="
echo ""
echo "Next steps:"
echo "1. Edit $BOT_DIR/.env with your credentials"
echo "2. Start all services: sudo systemctl start btc-arb-bot btc-ml-trainer btc-ml-predictor"
echo "3. Check logs: journalctl -u btc-arb-bot -f"
echo ""
echo "Commands:"
echo "  Start all:  sudo systemctl start btc-arb-bot btc-ml-trainer btc-ml-predictor"
echo "  Stop all:   sudo systemctl stop btc-arb-bot btc-ml-trainer btc-ml-predictor"
echo "  Bot logs:   journalctl -u btc-arb-bot -f"
echo "  ML logs:    journalctl -u btc-ml-trainer -f"
echo ""
echo "ML Pipeline:"
echo "  - Auto-trainer watches for new data and retrains models"
echo "  - Prediction server runs on http://127.0.0.1:8765"
echo "  - Models update automatically every 3 sessions"
