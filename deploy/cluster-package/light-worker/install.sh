#!/bin/bash
# Light Worker Installation Script
# Run this on Android (Termux), iOS (iSH/a-Shell), or any device with Python 3.

set -e

echo "=== Clawtex Light Worker Setup ==="

# Check Python
if ! command -v python3 &> /dev/null; then
    echo "ERROR: python3 not found. Install Python 3 first."
    echo "  Android (Termux): pkg install python"
    echo "  iOS (iSH): apk add python3"
    exit 1
fi

echo "Python version: $(python3 --version)"

# Check dependencies (only stdlib needed)
python3 -c "import http.server, json, urllib.request" 2>/dev/null
if [ $? -ne 0 ]; then
    echo "ERROR: Python stdlib modules missing. Ensure full Python 3 is installed."
    exit 1
fi

echo "Dependencies OK (stdlib only)"
echo ""
echo "Start with:"
echo "  python3 clawtex-worker.py --hub http://<HUB_IP>:7878 --name <NAME> --port 7880"
