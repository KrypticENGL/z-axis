# z-axis

A Windows-native 3D viewer with an embedded Claude Code agent panel.

## Stack

- C++17, CMake, vcpkg, Qt 6 Widgets (UI), Direct3D 11 (rendering), DirectXMath.

## Target: low-end Windows PCs

- Create the D3D11 device with feature level fallback down to 10_0. If no hardware device works, fall back to the WARP software rasterizer.
- Compile shaders for vs_4_0/ps_4_0. No compute shaders, no tessellation.
- Rendering budget: low-poly meshes, baked lighting, simple materials, optional FPS cap. No post-processing.

## Architecture

- The renderer sits behind an abstract `IRenderer` interface. A macOS Metal/SceneKit renderer will be added later.
- Scene logic, glTF loading, scene.json parsing and the agent bridge must contain NO Direct3D or Win32 code.

## Data

- Interchange format: glTF 2.0 (.glb).
- Scene description: `scene.json` (models, transforms, lights, camera) in a watched folder. The app hot-reloads it.

## Workflow

- Keep each phase small, buildable and committed.
- Run the build after every change.

<!-- code-review-graph MCP tools -->
## MCP Tools: code-review-graph

**IMPORTANT: This project has a knowledge graph. ALWAYS use the
code-review-graph MCP tools BEFORE using Grep/Glob/Read to explore
the codebase.** The graph is faster, cheaper (fewer tokens), and gives
you structural context (callers, dependents, test coverage) that file
scanning cannot.

### When to use graph tools FIRST

- **Exploring code**: `semantic_search_nodes` or `query_graph` instead of Grep
- **Understanding impact**: `get_impact_radius` instead of manually tracing imports
- **Code review**: `detect_changes` + `get_review_context` instead of reading entire files
- **Finding relationships**: `query_graph` with callers_of/callees_of/imports_of/tests_for
- **Architecture questions**: `get_architecture_overview` + `list_communities`

Fall back to Grep/Glob/Read **only** when the graph doesn't cover what you need.

### Key Tools

| Tool | Use when |
|------|----------|
| `detect_changes` | Reviewing code changes — gives risk-scored analysis |
| `get_review_context` | Need source snippets for review — token-efficient |
| `get_impact_radius` | Understanding blast radius of a change |
| `get_affected_flows` | Finding which execution paths are impacted |
| `query_graph` | Tracing callers, callees, imports, tests, dependencies |
| `semantic_search_nodes` | Finding functions/classes by name or keyword |
| `get_architecture_overview` | Understanding high-level codebase structure |
| `refactor_tool` | Planning renames, finding dead code |

### Workflow

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes` for code review.
3. Use `get_affected_flows` to understand impact.
4. Use `query_graph` pattern="tests_for" to check coverage.
