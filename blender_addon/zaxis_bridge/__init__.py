"""Z-Axis Bridge: a localhost JSON-lines server that runs agent commands on Blender's main thread."""

import contextlib
import io
import json
import os
import queue
import re
import secrets
import socket
import threading
import time
import traceback

import bpy
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

HOST, PORT = "127.0.0.1", 9876
MAX_LINE = 64 * 1024 * 1024
REGION_GROUP = "zaxis_region"
# glTF is Y-up; Blender is Z-up. Exporter maps (x, y, z)_blender -> (x, z, -y)_gltf.
GLTF_TO_BLENDER = Matrix(((1, 0, 0), (0, 0, -1), (0, 1, 0)))

_jobs = queue.Queue()
_server = None
_token = None


def token_path():
    return os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")), "zaxis", "token")


# ---------------------------------------------------------------- helpers

def _view3d_override():
    """Context override for ops that need a 3D viewport; empty in background mode."""
    wm = bpy.context.window_manager
    for window in wm.windows if wm else []:
        for area in window.screen.areas:
            if area.type == "VIEW_3D":
                region = next(r for r in area.regions if r.type == "WINDOW")
                return dict(window=window, screen=window.screen, area=area, region=region)
    return {}


def _ensure_object_mode():
    if bpy.context.mode != "OBJECT":
        with bpy.context.temp_override(**_view3d_override()):
            bpy.ops.object.mode_set(mode="OBJECT")


# ---------------------------------------------------------------- commands

def cmd_exec(code):
    bpy.ops.ed.undo_push(message="Z-Axis agent exec")
    out = io.StringIO()
    ns = {"bpy": bpy, "__name__": "__zaxis__", "result": None}
    with contextlib.redirect_stdout(out), bpy.context.temp_override(**_view3d_override()):
        exec(compile(code, "<agent>", "exec"), ns)
    return {"stdout": out.getvalue(), "result": repr(ns["result"]) if ns["result"] is not None else None}


def cmd_scene_info():
    objs = []
    for o in bpy.context.scene.objects:
        info = {
            "name": o.name,
            "type": o.type,
            "location": list(o.location),
            "dimensions": list(o.dimensions),
            "modifiers": [m.type for m in o.modifiers],
            "materials": [s.material.name for s in o.material_slots if s.material],
            "selected": o.select_get(),
        }
        if o.type == "MESH":
            info["faces"] = len(o.data.polygons)
            info["vertex_groups"] = [g.name for g in o.vertex_groups]
        objs.append(info)
    active = bpy.context.view_layer.objects.active
    return {
        "file": bpy.data.filepath,
        "mode": bpy.context.mode,
        "active": active.name if active else None,
        "objects": objs,
    }


def cmd_screenshot(path):
    ov = _view3d_override()
    if not ov or bpy.app.background:
        raise RuntimeError("No 3D viewport (Blender is running without UI)")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with bpy.context.temp_override(**ov):
        bpy.ops.screen.screenshot_area(filepath=path)
    return {"path": path}


def cmd_export_glb(dir, note=""):
    _ensure_object_mode()
    os.makedirs(dir, exist_ok=True)
    nums = [int(m.group(1)) for f in os.listdir(dir) if (m := re.fullmatch(r"v(\d+)\.glb", f))]
    stem = os.path.join(dir, "v%03d" % (max(nums, default=0) + 1))
    bpy.ops.export_scene.gltf(filepath=stem + ".glb", export_format="GLB", export_apply=True)
    meta = {"time": time.strftime("%Y-%m-%d %H:%M:%S"), "note": note, "blend": bpy.data.filepath}
    with open(stem + ".json", "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
    return {"path": stem + ".glb"}


def cmd_select_region(regions, space="gltf"):
    """regions: [{object, points: [[x,y,z], ...]}] world-space face centres (from the app's GLB view).

    Maps each point to the nearest face of the object's *original* mesh (not the evaluated one,
    so face indices stay stable for the agent), selects those faces and stores them in REGION_GROUP.
    """
    _ensure_object_mode()
    for o in bpy.context.view_layer.objects:
        o.select_set(False)
    report = []
    for reg in regions:
        obj = bpy.data.objects.get(reg["object"])
        if obj is None or obj.type != "MESH":
            raise ValueError("No mesh object named %r" % reg["object"])
        mesh = obj.data
        tree = BVHTree.FromPolygons([v.co for v in mesh.vertices], [p.vertices for p in mesh.polygons])
        to_local = obj.matrix_world.inverted()
        faces = set()
        for p in reg["points"]:
            world = Vector(p)
            if space == "gltf":
                world = GLTF_TO_BLENDER @ world
            hit = tree.find_nearest(to_local @ world)
            if hit[2] is not None:
                faces.add(hit[2])
        for poly in mesh.polygons:
            poly.select = poly.index in faces
        verts = sorted({v for i in faces for v in mesh.polygons[i].vertices})
        group = obj.vertex_groups.get(REGION_GROUP) or obj.vertex_groups.new(name=REGION_GROUP)
        group.remove(range(len(mesh.vertices)))
        group.add(verts, 1.0, "REPLACE")
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        report.append({"object": obj.name, "faces": sorted(faces), "vertices": len(verts)})
    if report:
        with bpy.context.temp_override(**_view3d_override()):
            bpy.ops.object.mode_set(mode="EDIT")
            bpy.ops.mesh.select_mode(type="FACE")
    return {"group": REGION_GROUP, "regions": report}


COMMANDS = {
    "exec": cmd_exec,
    "scene_info": cmd_scene_info,
    "screenshot": cmd_screenshot,
    "export_glb": cmd_export_glb,
    "select_region": cmd_select_region,
}


# ---------------------------------------------------------------- main-thread pump

def pump():
    """Run queued jobs on the main thread. Registered as a timer; tests call it directly."""
    while True:
        try:
            req, box, done = _jobs.get_nowait()
        except queue.Empty:
            return 0.05
        try:
            box["result"] = COMMANDS[req["cmd"]](**req.get("args", {}))
        except Exception:
            box["error"] = traceback.format_exc()
        done.set()


# ---------------------------------------------------------------- socket server

def _handle(conn):
    with conn, conn.makefile("rb") as rf:
        for line in rf:
            if len(line) > MAX_LINE:
                return
            try:
                req = json.loads(line)
            except ValueError:
                return
            if not secrets.compare_digest(str(req.get("token", "")), _token):
                conn.sendall(b'{"error": "bad token"}\n')
                return
            if req.get("cmd") not in COMMANDS:
                resp = {"error": "unknown command %r" % req.get("cmd")}
            else:
                box, done = {}, threading.Event()
                _jobs.put((req, box, done))
                # Blender may be busy (modal op, long exec); wait instead of failing the agent.
                resp = box if done.wait(600) else {"error": "timed out waiting for Blender main thread"}
            conn.sendall(json.dumps(resp, default=str).encode() + b"\n")


def _serve(sock):
    while True:
        try:
            conn, _ = sock.accept()
        except OSError:
            return  # socket closed by stop()
        threading.Thread(target=_handle, args=(conn,), daemon=True).start()


def start():
    global _server, _token
    if _server:
        return
    _token = secrets.token_hex(24)
    os.makedirs(os.path.dirname(token_path()), exist_ok=True)
    with open(token_path(), "w") as f:
        f.write(_token)
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.bind((HOST, PORT))
    sock.listen(8)
    _server = sock
    threading.Thread(target=_serve, args=(sock,), daemon=True).start()
    if not bpy.app.timers.is_registered(pump):
        bpy.app.timers.register(pump, persistent=True)


def stop():
    global _server
    if _server:
        _server.close()
        _server = None
    if bpy.app.timers.is_registered(pump):
        bpy.app.timers.unregister(pump)


# ---------------------------------------------------------------- UI

class ZAXIS_OT_toggle(bpy.types.Operator):
    bl_idname = "zaxis.toggle_server"
    bl_label = "Start/Stop Z-Axis Bridge"

    def execute(self, context):
        if _server:
            stop()
        else:
            try:
                start()
            except OSError as e:
                self.report({"ERROR"}, "Port %d busy: %s" % (PORT, e))
                return {"CANCELLED"}
        return {"FINISHED"}


class ZAXIS_PT_panel(bpy.types.Panel):
    bl_label = "Z-Axis"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Z-Axis"

    def draw(self, context):
        col = self.layout.column()
        col.label(text=("Running on %s:%d" % (HOST, PORT)) if _server else "Stopped",
                  icon="CHECKMARK" if _server else "X")
        col.operator(ZAXIS_OT_toggle.bl_idname, text="Stop" if _server else "Start")


_classes = (ZAXIS_OT_toggle, ZAXIS_PT_panel)


def register():
    for c in _classes:
        bpy.utils.register_class(c)
    if bpy.app.background:
        return  # e.g. the app enabling the add-on headlessly; don't steal the live session's token
    try:
        start()
    except OSError as e:
        print("Z-Axis Bridge: could not start server:", e)


def unregister():
    stop()
    for c in reversed(_classes):
        bpy.utils.unregister_class(c)
