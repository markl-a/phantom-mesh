# Clawtex Worker Setup Guide (for Claude Code)

## Your Task
Set up and start a Clawtex Worker on this machine, connecting to the Hub.

## Prerequisites
- **Full Worker** (macOS/Windows/Linux with Rust): Needs `cargo` installed
- **Light Worker** (Android/iOS/any Python): Needs `python3` installed

## Steps

### 1. Identify this device
Read the matching config file from `configs/` directory for this machine's name, port, and capabilities.

### 2. Set Hub IP
The Hub runs on the Z13 machine. Replace `<HUB_IP>` below with the Hub's IP address (check with the user).

### 3a. Full Worker Setup (macOS/Windows/Linux)
```bash
# Clone or copy the clawtex-core source
cd clawtex-core

# Build
cargo build --release

# Start worker
./target/release/clawtex-core worker \
  --hub http://<HUB_IP>:7878 \
  --name <DEVICE_NAME> \
  --port <PORT>
```

### 3b. Light Worker Setup (Android/iOS/any Python)
```bash
# Copy clawtex-worker.py from light-worker/ directory
python3 clawtex-worker.py \
  --hub http://<HUB_IP>:7878 \
  --name <DEVICE_NAME> \
  --port <PORT>
```

### 4. Verify
```bash
# Check local health
curl http://localhost:<PORT>/health

# Check Hub can see this worker
curl http://<HUB_IP>:7878/cluster/workers
```

### 5. Run as Background Service (Optional)
```bash
# Linux/macOS: use nohup
nohup ./target/release/clawtex-core worker --hub http://<HUB_IP>:7878 --name <NAME> --port <PORT> > /tmp/clawtex-worker.log 2>&1 &

# Or use screen/tmux for persistent sessions
```

## Hub 連線資訊
- **Hub IP**: `192.168.1.104` (Z13 LAN)
- **Hub Port**: `7878`

## Device Configurations

| Device | Type | Config File | Port |
|--------|------|-------------|------|
| Z13 (Hub) | Hub | — | 7878 |
| M1 Mac | Full Worker | m1-mac.toml | 7879 |
| AYANEO | Full Worker | ayaneo.toml | 7880 |
| Acer | Light Worker | aspire5.toml | 7881 |
| iPhone | Light Worker | iphone.toml | 7882 |
| Android 1 | Light Worker | android-1.toml | 7883 |
| Android 2 | Light Worker | android-2.toml | 7884 |
| iPad | Light Worker | ipad.toml | 7885 |

## Mobile 安裝
詳見 `deploy/mobile/README.md` — Android (Termux) 和 iOS (iSH/a-Shell) 一鍵安裝腳本。

## Troubleshooting
- **Connection refused**: Check Hub IP, firewall, and that the Hub is running
- **Registration failed**: Hub may be overloaded — worker will retry with backoff
- **Build errors**: Ensure Rust toolchain is installed (`rustup update`)
