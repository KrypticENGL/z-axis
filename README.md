# Z-Axis

Native Windows app (Rust + Win32 + D3D11) that drives a live Blender 5.x session through an AI agent:
Claude Code, Codex CLI, Gemini CLI, or local models via Ollama / LM Studio (run through `codex --oss`).

## Run
1. `cargo build --release`
2. Start `target/release/zaxis.exe` → **File → Install Blender add-on** (or install `blender_addon/zaxis_bridge` yourself) → **File → Launch Blender**.
3. Pick an agent, type an instruction, press Enter. Exports appear under **Versions** and in the viewer.

Workspace (exports, references, region data, agent skill/MCP config): `Documents\Z-Axis` (`--workspace <dir>` to change).

## Using it
- **Viewer**: left-drag orbit, right/middle or Shift+left drag pan, wheel zoom, `F` frame.
- **Compare**: View → Compare versions. Left = pinned version, right = whatever you click in the list.
- **Maps**: View → Base Color / Normal / Metal-Rough / AO / Emissive / UV layout, each its own window. Click a map window to cycle materials.
- **Region edit**: toolbar **Capture** (Lasso → Box → Off), drag over the model (Shift add, Ctrl remove). Blender selects those faces into vertex group `zaxis_region`; your next prompt is limited to them.
- **References**: Add… or drag images onto the window; selected references are attached to the next prompt.

## How it fits together
```
zaxis.exe (GUI) ──spawns──► agent CLI ──stdio MCP──► zaxis.exe mcp ──TCP 127.0.0.1:9876 + token──► Blender add-on
```
On agent select, `skills/blender-zaxis` is written into the workspace in that agent's format
(`.claude/skills` + `.mcp.json`, `.agents/skills` + `AGENTS.md`, `.gemini/` + `GEMINI.md`).

## Tests
- `cargo test`: protocol, parsers, region picking, skill install.
- `blender --background --factory-startup --python tests/test_addon.py`: the add-on over its real socket.
- Live agent: run `blender --background --factory-startup --python tests/serve_blender.py -- 900`, then
  `ZAXIS_E2E=claude cargo test e2e -- --ignored --nocapture` (`codex`, `ollama`, `lmstudio`, `gemini`; `ZAXIS_E2E_MODEL` for a model).
