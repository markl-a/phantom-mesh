#!/bin/bash
# Phantom Mesh Sandbox Entrypoint
# Starts Xvfb + Mutter + Tint2 + x11vnc + noVNC

set -e

WIDTH=${WIDTH:-1024}
HEIGHT=${HEIGHT:-768}
DEPTH=${DEPTH:-24}

echo "Starting Phantom Mesh Sandbox: ${WIDTH}x${HEIGHT}x${DEPTH}"

# Start virtual display
Xvfb :1 -screen 0 ${WIDTH}x${HEIGHT}x${DEPTH} -ac +extension GLX +render -noreset &
sleep 1

# Start D-Bus session
eval $(dbus-launch --sh-syntax)
export DBUS_SESSION_BUS_ADDRESS

# Start window manager (openbox works in Xvfb; mutter does NOT)
openbox &
sleep 1

# Start panel
tint2 &

# Start VNC server (no password for local use)
x11vnc -display :1 -forever -nopw -shared -rfbport 5900 &

# Start noVNC web client
/usr/share/novnc/utils/novnc_proxy --vnc localhost:5900 --listen 6080 &

echo "Sandbox ready. VNC on :5900, noVNC on :6080"

# Keep container running
wait
