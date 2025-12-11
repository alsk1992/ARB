# Deployment Guide

## Recommended: AWS EC2 in us-east-1

### Instance Type
- **t3.small** ($0.02/hr) - Good for testing
- **c6i.large** ($0.085/hr) - Production, compute optimized

### Launch Steps

1. **Create EC2 Instance**
   - Region: us-east-1 (N. Virginia)
   - AMI: Ubuntu 22.04 LTS
   - Instance type: t3.small or c6i.large
   - Storage: 20GB gp3
   - Security group: Allow SSH (port 22) from your IP

2. **Connect to Instance**
   ```bash
   ssh -i your-key.pem ubuntu@<instance-ip>
   ```

3. **Upload Bot Files**
   ```bash
   # From your local machine
   scp -i your-key.pem -r /Users/alsk/poly/btc-arb-bot ubuntu@<instance-ip>:~/
   ```

4. **Run Setup Script**
   ```bash
   cd ~/btc-arb-bot
   chmod +x deploy/setup.sh
   ./deploy/setup.sh
   ```

5. **Configure Credentials**
   ```bash
   nano ~/.env
   ```

   Add your credentials:
   ```
   POLY_API_KEY=your_key
   POLY_API_SECRET=your_secret
   POLY_API_PASSPHRASE=your_passphrase
   POLY_ADDRESS=0x_your_address
   PRIVATE_KEY=your_private_key_no_0x
   DRY_RUN=false
   DISCORD_WEBHOOK=https://discord.com/api/webhooks/...
   ```

6. **Start Bot**
   ```bash
   sudo systemctl start btc-arb-bot
   ```

7. **Monitor**
   ```bash
   journalctl -u btc-arb-bot -f
   ```

## Network Optimization

Add to `/etc/sysctl.conf`:
```
# TCP optimization for low latency
net.ipv4.tcp_fastopen = 3
net.ipv4.tcp_slow_start_after_idle = 0
net.ipv4.tcp_no_metrics_save = 1
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
```

Apply: `sudo sysctl -p`

## Costs

| Instance | $/hour | $/month |
|----------|--------|---------|
| t3.small | $0.02 | ~$15 |
| c6i.large | $0.085 | ~$62 |

## Alternative: Hetzner (Cheaper)

- CX21: 2 vCPU, 4GB RAM - €4.85/month
- Location: Ashburn, VA (near AWS us-east-1)
- Same setup process, just different cloud provider
