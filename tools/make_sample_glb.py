"""Writes assets/sample.glb: a low-poly house (brick-textured box, pyramid roof as a child node) on a ground quad.

Stdlib only. Run: python tools/make_sample_glb.py
"""
import json
import math
import struct
import zlib
from pathlib import Path


def png(w, h, pixel):
    rows = b"".join(b"\x00" + bytes(c for x in range(w) for c in pixel(x, y)) for y in range(h))

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))

    header = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)  # 8-bit RGB
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def brick(x, y):
    row = y // 8
    mortar = y % 8 == 0 or (x + (row % 2) * 8) % 16 == 0
    return (205, 195, 175) if mortar else (165, 70 + (x * 7 + y * 13) % 25, 55)


def sub(a, b):
    return [a[i] - b[i] for i in range(3)]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def unit(v):
    n = math.sqrt(sum(c * c for c in v))
    return [c / n for c in v]


def box(sx, sy, sz, tile=2.0):
    """Box standing on y=0. Each face: (normal, u, v) with u x v = normal so quads are CCW from outside."""
    faces = [((1, 0, 0), (0, 0, -1), (0, 1, 0)), ((-1, 0, 0), (0, 0, 1), (0, 1, 0)),
             ((0, 1, 0), (1, 0, 0), (0, 0, -1)), ((0, -1, 0), (1, 0, 0), (0, 0, 1)),
             ((0, 0, 1), (1, 0, 0), (0, 1, 0)), ((0, 0, -1), (-1, 0, 0), (0, 1, 0))]
    half = (sx / 2, sy / 2, sz / 2)
    pos, nrm, uv, idx = [], [], [], []
    for n, u, v in faces:
        base = len(pos)
        for su, sv in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
            p = [(n[i] + su * u[i] + sv * v[i]) * half[i] for i in range(3)]
            p[1] += half[1]
            pos.append(p)
            nrm.append(list(n))
            uv.append([(su + 1) / 2 * tile, (1 - sv) / 2 * tile])  # glTF v points down the image
        idx += [base, base + 1, base + 2, base, base + 2, base + 3]
    return pos, nrm, uv, idx


def pyramid(half, height):
    ring = [(-half, 0, half), (half, 0, half), (half, 0, -half), (-half, 0, -half)]
    apex = (0, height, 0)
    pos, nrm, uv, idx = [], [], [], []
    for i in range(4):
        a, b = ring[i], ring[(i + 1) % 4]
        n = unit(cross(sub(b, a), sub(apex, a)))
        base = len(pos)
        pos += [list(a), list(b), list(apex)]
        nrm += [n, n, n]
        uv += [[0, 1], [1, 1], [0.5, 0]]
        idx += [base, base + 1, base + 2]
    return pos, nrm, uv, idx


def quad(half):
    pos = [[-half, 0, half], [half, 0, half], [half, 0, -half], [-half, 0, -half]]
    return pos, [[0, 1, 0]] * 4, [[0, 1], [1, 1], [1, 0], [0, 0]], [0, 1, 2, 0, 2, 3]


def build():
    blob = bytearray()
    views, accessors = [], []

    def add_view(data, target=None):
        while len(blob) % 4:
            blob.append(0)
        view = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target:
            view["target"] = target
        blob.extend(data)
        views.append(view)
        return len(views) - 1

    def add_accessor(values, kind, comps):
        data = b"".join(struct.pack(f"<{comps}f", *v) for v in values)
        acc = {"bufferView": add_view(data, 34962), "componentType": 5126, "count": len(values), "type": kind}
        if kind == "VEC3":
            acc["min"] = [min(v[i] for v in values) for i in range(3)]
            acc["max"] = [max(v[i] for v in values) for i in range(3)]
        accessors.append(acc)
        return len(accessors) - 1

    def add_mesh(geometry, material):
        pos, nrm, uv, idx = geometry
        attrs = {"POSITION": add_accessor(pos, "VEC3", 3), "NORMAL": add_accessor(nrm, "VEC3", 3),
                 "TEXCOORD_0": add_accessor(uv, "VEC2", 2)}
        accessors.append({"bufferView": add_view(struct.pack(f"<{len(idx)}H", *idx), 34963),
                          "componentType": 5123, "count": len(idx), "type": "SCALAR"})
        return {"primitives": [{"attributes": attrs, "indices": len(accessors) - 1, "material": material}]}

    meshes = [add_mesh(box(2.0, 1.4, 2.0), 0), add_mesh(pyramid(1.25, 0.9), 1), add_mesh(quad(3.0), 2)]
    image_view = add_view(png(64, 64, brick))
    while len(blob) % 4:
        blob.append(0)

    half_angle = math.radians(30) / 2
    gltf = {
        "asset": {"version": "2.0", "generator": "z-axis make_sample_glb.py"},
        "scene": 0,
        "scenes": [{"nodes": [0, 2]}],
        "nodes": [
            {"name": "House", "mesh": 0, "rotation": [0, math.sin(half_angle), 0, math.cos(half_angle)], "children": [1]},
            {"name": "Roof", "mesh": 1, "translation": [0, 1.4, 0]},
            {"name": "Ground", "mesh": 2},
        ],
        "meshes": meshes,
        "materials": [
            {"name": "Brick", "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicFactor": 0}},
            {"name": "Roof", "pbrMetallicRoughness": {"baseColorFactor": [0.55, 0.12, 0.08, 1], "metallicFactor": 0}},
            {"name": "Grass", "pbrMetallicRoughness": {"baseColorFactor": [0.22, 0.42, 0.16, 1], "metallicFactor": 0}},
        ],
        "textures": [{"source": 0, "sampler": 0}],
        "samplers": [{"magFilter": 9729, "minFilter": 9729}],
        "images": [{"bufferView": image_view, "mimeType": "image/png"}],
        "buffers": [{"byteLength": len(blob)}],
        "bufferViews": views,
        "accessors": accessors,
    }

    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    total = 12 + 8 + len(js) + 8 + len(blob)
    return (struct.pack("<4sII", b"glTF", 2, total) + struct.pack("<I4s", len(js), b"JSON") + js
            + struct.pack("<I4s", len(blob), b"BIN\x00") + bytes(blob))


if __name__ == "__main__":
    out = Path(__file__).resolve().parent.parent / "assets" / "sample.glb"
    out.parent.mkdir(exist_ok=True)
    out.write_bytes(build())
    print(f"wrote {out} ({out.stat().st_size} bytes)")
