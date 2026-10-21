# Blender 5.x bpy notes

## Context
`blender_exec` runs inside a 3D-viewport context override when Blender has a UI, so most `bpy.ops` work.
Still prefer data API over operators: it's faster, has no context requirements and is deterministic.

```python
import bmesh
obj = bpy.data.objects["Table"]
bpy.context.view_layer.objects.active = obj   # many ops act on the active object
```

## Modes
- Check `bpy.context.mode` (`'OBJECT'`, `'EDIT_MESH'`, ...). Switch with `bpy.ops.object.mode_set(mode='EDIT')`.
- In edit mode, `obj.data` is stale. Use `bmesh.from_edit_mesh(obj.data)` and finish with `bmesh.update_edit_mesh(obj.data)`.
- In object mode use `bm = bmesh.new(); bm.from_mesh(me); ...; bm.to_mesh(me); bm.free(); me.update()`.

## Creating geometry
```python
bpy.ops.mesh.primitive_cube_add(size=1, location=(0, 0, 0.5))
obj = bpy.context.active_object; obj.name = "Crate"
```
Or build data directly:
```python
me = bpy.data.meshes.new("Panel"); me.from_pydata(verts, [], faces); me.update()
obj = bpy.data.objects.new("Panel", me); bpy.context.scene.collection.objects.link(obj)
```

## 4.x → 5.x changes that break old snippets
- Material/World/Light node trees: `use_nodes` is always on (setting it is deprecated). Use `mat.node_tree` directly.
- `Principled BSDF` inputs: `"Specular IOR Level"`, `"Emission Color"`, `"Subsurface Weight"`, `"Coat Weight"`, `"Transmission Weight"`. `"Specular"`, `"Emission"` and `"Subsurface"` no longer exist.
- `Mesh.use_auto_smooth` and `auto_smooth_angle` are gone. Use `bpy.ops.object.shade_auto_smooth()` or the "Smooth by Angle" modifier; `obj.data.shade_smooth()` for plain smooth shading.
- `bpy.context.scene.render.engine` values: `'BLENDER_EEVEE'` (EEVEE Next), `'CYCLES'`, `'BLENDER_WORKBENCH'`.
- Mesh attributes: use `me.attributes` (`FLOAT_COLOR`, `FLOAT2` for UV maps). `me.vertex_colors` is gone; use `me.color_attributes`.
- Geometry nodes modifier inputs: `mod["Socket_2"]` keys — read identifiers from `mod.node_group.interface.items_tree`.
- `bpy.ops.export_scene.gltf` uses `export_apply` for modifiers; `export_format='GLB'`.

## Modifiers
```python
m = obj.modifiers.new("Bevel", 'BEVEL'); m.width = 0.02; m.segments = 3; m.limit_method = 'ANGLE'
bpy.ops.object.modifier_apply(modifier=m.name)   # only when asked to apply
```

## Debugging
- `print()` anything you need to see; set `result = ...` for a value.
- Undo: each `blender_exec` is one undo step; to revert your last step run `bpy.ops.ed.undo()`.
