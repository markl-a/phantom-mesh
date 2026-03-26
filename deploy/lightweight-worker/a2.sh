#!/bin/bash
pkill -f phantom-mesh 2>/dev/null
curl -so w.py http://10.0.1.1:8888/phantom-mesh-worker.py
python w.py --hub http://10.0.1.1:7878 --name android-2 --port 7884
