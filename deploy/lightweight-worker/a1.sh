#!/bin/bash
pkill -f clawtex 2>/dev/null
curl -so w.py http://192.168.1.104:8888/clawtex-worker.py
python w.py --hub http://192.168.1.104:7878 --name android-1 --port 7883
