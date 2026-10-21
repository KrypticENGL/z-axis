# Materials, UVs, textures, export

The Z-Axis app shows the GLB's base colour, normal, metallic/roughness, occlusion and emissive maps and the
`TEXCOORD_0` UV layout. Only what glTF can carry shows up there, so build materials the glTF exporter understands.

## glTF-friendly materials
Principled BSDF → Material Output, with Image Texture nodes plugged straight in:
```python
mat = bpy.data.materials.new("Wood")
nt = mat.node_tree; bsdf = nt.nodes["Principled BSDF"]
bsdf.inputs["Base Color"].default_value = (0.55, 0.35, 0.2, 1)
bsdf.inputs["Roughness"].default_value = 0.6
img = nt.nodes.new("ShaderNodeTexImage"); img.image = bpy.data.images.load(path)
nt.links.new(img.outputs["Color"], bsdf.inputs["Base Color"])
obj.data.materials.append(mat)
```
- Normal map: Image (Non-Color) → `Normal Map` node → BSDF `Normal`.
- Metal/rough: one Image (Non-Color) → `Separate Color`; G → Roughness, B → Metallic.
- Procedural textures (Noise, Voronoi...) do **not** export. Bake them to images first if the user wants to see them in the app.

## UVs
```python
bpy.ops.object.mode_set(mode='EDIT'); bpy.ops.mesh.select_all(action='SELECT')
bpy.ops.uv.smart_project(angle_limit=1.15, island_margin=0.02)   # radians
bpy.ops.object.mode_set(mode='OBJECT')
```
Use `uv.unwrap(method='ANGLE_BASED')` after marking seams for organic shapes. Keep one UV map (`TEXCOORD_0`).

## Baking (Cycles)
Set `scene.render.engine = 'CYCLES'`, add an Image Texture node with a new image selected (active) in the material,
`bpy.ops.object.bake(type='DIFFUSE', pass_filter={'COLOR'})`, then `image.save_render(path)` or pack it
(`image.pack()`) so it embeds in the GLB.

## Export
Always use `blender_export_glb` (it numbers versions and records your `note`). Don't call
`bpy.ops.export_scene.gltf` yourself — the app only watches the versions folder.
Modifiers are applied in the export; the .blend keeps them live.
