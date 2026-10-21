"""Headless Blender serving the bridge for N seconds (default 300): for MCP / agent end-to-end runs.
Run: blender --background --factory-startup --python tests/serve_blender.py -- 300
"""
import os, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "blender_addon"))
import zaxis_bridge as zb

secs = float(sys.argv[sys.argv.index("--") + 1]) if "--" in sys.argv else 300
zb.start()
print("serving", flush=True)
end = time.time() + secs
while time.time() < end:
    zb.pump()
    time.sleep(0.02)
zb.stop()
