---
name: blender-zaxis
description: Model, edit and export 3D assets in a live Blender 5.x session through the zaxis MCP tools (blender_exec, blender_scene_info, blender_screenshot, blender_export_glb, blender_get_region). Use for any request about the Blender scene, meshes, materials, UVs, textures, modifiers, or editing a captured region of the model.
---

# Blender via Z-Axis

You are driving a **live** Blender 5.x session that the user is watching. Every change is real and visible.
The Z-Axis app shows the latest exported GLB, so the user only sees your work after you export.

## Tools
| Tool | Use |
|---|---|
| `blender_scene_info` | Objects, mode, modifiers, materials, vertex groups. **Call first, every task.** |
| `blender_exec` | Run Python with `bpy` preloaded. `print()` output and a `result` variable come back. Each call is one undo step. |
| `blender_screenshot` | Image of the 3D viewport. Use to check your work visually. |
| `blender_export_glb` | Export the scene as the next version (`versions/v###.glb`). Pass a short `note` describing the change. |
| `blender_get_region` | The region the user captured in the app (object + faces), or nothing. |

## Workflow
1. `blender_scene_info`. If the prompt says **Active region**, also call `blender_get_region`.
2. Plan in small steps. One `blender_exec` per logical step (create, then modify, then material) so errors are easy to locate and undo is granular.
3. After each step, check the output; on a traceback, fix the cause, don't retry blindly.
4. Look at a `blender_screenshot` before you declare a visual change done (skip if it errors with "without UI").
5. `blender_export_glb` once the requested change is complete. Always export — the user can't see it otherwise.
6. Reply briefly: what changed, object names, anything you couldn't do.

## Rules
- Never delete or rename objects the user didn't mention. Never `read_factory_settings` / open other files / quit Blender.
- Don't save the .blend unless asked.
- Use real-world units: 1 Blender unit = 1 metre.
- Name what you create (`obj.name = "Chair_Leg_FL"`), never leave `Cube.003`.
- Prefer non-destructive work (modifiers) unless the user asks to apply.
- With an active region: change **only** the faces in vertex group `zaxis_region`. See [references/region-edit.md](references/region-edit.md).
- References (image paths in the prompt): look at them before modelling and match proportions/colours.

## More
- Blender 5.x API pitfalls and recipes: [references/bpy-5x-notes.md](references/bpy-5x-notes.md)
- Region edits: [references/region-edit.md](references/region-edit.md)
- Materials, UVs, textures and export: [references/export.md](references/export.md)
