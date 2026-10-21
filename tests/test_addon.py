"""Run: blender --background --factory-startup --python tests/test_addon.py

Drives the bridge over its real socket while the main thread pumps jobs (timers don't run in -b scripts).
"""

import json
import os
import socket
import sys
import tempfile
import threading
import traceback

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "blender_addon"))
import bpy  # noqa: E402
import zaxis_bridge as zb  # noqa: E402

failures = []
finished = threading.Event()


def call(sock_file, sock, cmd, **args):
    sock.sendall(json.dumps({"token": zb._token, "cmd": cmd, "args": args}).encode() + b"\n")
    return json.loads(sock_file.readline())


def client():
    try:
        out_dir = tempfile.mkdtemp()
        s = socket.create_connection((zb.HOST, zb.PORT))
        f = s.makefile("rb")

        r = call(f, s, "exec", code="for o in list(bpy.data.objects): bpy.data.objects.remove(o)\n"
                                    "bpy.ops.mesh.primitive_cube_add(size=2)\n"
                                    "bpy.context.object.name = 'Box'\nprint('made')")
        assert "error" not in r, r
        assert r["result"]["stdout"].strip() == "made", r

        r = call(f, s, "scene_info")
        names = [o["name"] for o in r["result"]["objects"]]
        assert names == ["Box"], names

        r = call(f, s, "export_glb", dir=out_dir, note="t")
        assert os.path.exists(r["result"]["path"]) and r["result"]["path"].endswith("v001.glb"), r
        r = call(f, s, "export_glb", dir=out_dir)
        assert r["result"]["path"].endswith("v002.glb"), r

        # Blender +Z top face centre (0,0,1) is (0,1,0) in glTF space; slightly offset points still hit it.
        r = call(f, s, "select_region", regions=[{"object": "Box", "points": [[0, 1.0, 0], [0.3, 1.01, 0.2]]}])
        reg = r["result"]["regions"][0]
        assert len(reg["faces"]) == 1 and reg["vertices"] == 4, r

        r = call(f, s, "exec", code="import mathutils\n"
                                    "o = bpy.data.objects['Box']\n"
                                    "p = o.data.polygons[%d]\n"
                                    "result = tuple(round(c, 3) for c in p.center)" % reg["faces"][0])
        assert r["result"]["result"] == "(0.0, 0.0, 1.0)", r

        r = call(f, s, "exec", code="raise ValueError('boom')")
        assert "boom" in r["error"], r

        s.sendall(b'{"token": "wrong", "cmd": "scene_info"}\n')
        assert json.loads(f.readline())["error"] == "bad token"
    except Exception:
        failures.append(traceback.format_exc())
    finally:
        finished.set()


zb.start()
threading.Thread(target=client, daemon=True).start()
while not finished.wait(0.02):
    zb.pump()
zb.stop()

if failures:
    print("FAIL\n" + failures[0])
    sys.exit(1)
print("ADDON TESTS PASSED")
sys.exit(0)
