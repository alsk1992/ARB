#!/bin/bash
set -e

echo "🚀 Deploying Order Flow System to AWS"

# Check if .env exists
if [ ! -f .env ]; then
    echo "Creating .env file from .env.example..."
    cp .env.example .env
    echo "⚠️  Please edit .env with your settings before running again!"
    exit 1
fi

# Stop existing containers
echo "Stopping existing containers..."
docker-compose down

# Pull latest code
echo "Pulling latest code..."
git pull origin orderflow-trading

# Build images
echo "Building Docker images (this will take 5-10 minutes)..."
docker-compose build --no-cache

# Start services
echo "Starting services..."
docker-compose up -d

# Wait for postgres
echo "Waiting for PostgreSQL to be ready..."
sleep 10

# Show status
echo ""
echo "✅ Deployment complete!"
echo ""
echo "📊 Service Status:"
docker-compose ps

echo ""
echo "📝 View Logs:"
echo "  All services:    docker-compose logs -f"
echo "  Listener only:   docker-compose logs -f listener"
echo "  Reputation only: docker-compose logs -f reputation"
echo "  Executor only:   docker-compose logs -f executor"

echo ""
echo "🔍 Check Database:"
echo "  docker-compose exec postgres psql -U orderflow -d orderflow -c 'SELECT COUNT(*) FROM orderflow_trades;'"

echo ""
echo "🛑 Stop All:"
echo "  docker-compose down"
