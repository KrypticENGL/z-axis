# Editing a captured region

The user boxes/lassos part of the model in the Z-Axis viewer. The app has already:
- selected those faces in Blender (object is active and in **Edit Mode, face select**), and
- stored their vertices in the vertex group **`zaxis_region`**.

`blender_get_region` returns `{object, faces: [indices], vertices, image}`; `image` is a crop of what the user saw.

## Rules
- Touch only the region. Geometry outside it must keep its position (bridging edges may move only if the edit needs it, e.g. an extrude).
- Re-select the region yourself before operating; the user may have clicked in Blender since:

```python
import bmesh
obj = bpy.data.objects[REGION_OBJECT]
bpy.context.view_layer.objects.active = obj
if bpy.context.mode != 'EDIT_MESH':
    bpy.ops.object.mode_set(mode='EDIT')
bpy.ops.mesh.select_mode(type='FACE')
bpy.ops.mesh.select_all(action='DESELECT')
bpy.ops.object.vertex_group_set_active(group='zaxis_region')
bpy.ops.object.vertex_group_select()
```

## Recipes (region selected, Edit Mode)
- Extrude along normals: `bpy.ops.mesh.extrude_region_shrink_fatten(TRANSFORM_OT_shrink_fatten={"value": 0.2})`
- Inset: `bpy.ops.mesh.inset(thickness=0.05, depth=0)`
- Scale about region centre: `bpy.ops.transform.resize(value=(1.2, 1.2, 1.2))`
- Move: `bpy.ops.transform.translate(value=(0, 0, 0.1))`
- Subdivide: `bpy.ops.mesh.subdivide(number_cuts=2)`
- Bevel edges of region: `bpy.ops.mesh.bevel(offset=0.02, segments=3)`
- Separate into its own object: `bpy.ops.mesh.separate(type='SELECTED')`
- Material only on region: add slot, set `obj.active_material_index`, `bpy.ops.object.material_slot_assign()`
- Smooth: `bpy.ops.mesh.vertices_smooth(factor=0.5, repeat=5)`

New geometry from extrude/inset is selected afterwards — run
`bpy.ops.object.vertex_group_assign()` so the group still covers the edited region.

Return to Object Mode (`bpy.ops.object.mode_set(mode='OBJECT')`) before `blender_export_glb`.
