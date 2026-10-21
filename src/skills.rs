//! Installs the blender-zaxis skill + MCP config into the workspace in each agent's native format.

use crate::agent::Agent;
use serde_json::json;
use std::path::Path;

const SKILL: &str = include_str!("../skills/blender-zaxis/SKILL.md");
const REFS: [(&str, &str); 3] = [
    ("bpy-5x-notes.md", include_str!("../skills/blender-zaxis/references/bpy-5x-notes.md")),
    ("region-edit.md", include_str!("../skills/blender-zaxis/references/region-edit.md")),
    ("export.md", include_str!("../skills/blender-zaxis/references/export.md")),
];

pub fn mcp_args(ws: &Path) -> Vec<String> {
    vec!["mcp".into(), "--workspace".into(), ws.display().to_string()]
}

pub fn install(agent: Agent, ws: &Path, exe: &Path) -> std::io::Result<()> {
    for d in ["versions", "references", "shots"] {
        std::fs::create_dir_all(ws.join(d))?;
    }
    let server = json!({"command": exe, "args": mcp_args(ws)});
    match agent {
        Agent::Claude => {
            write_skill(&ws.join(".claude/skills/blender-zaxis"))?;
            write(ws, ".mcp.json", &pretty(json!({"mcpServers": {"zaxis": server}})))?;
            write(ws, "CLAUDE.md", "Blender work: use the `blender-zaxis` skill and the `zaxis` MCP tools. Export after every change.\n")
        }
        // Codex gets its MCP server via -c flags at launch (see agent.rs), so no global config edits.
        Agent::Codex | Agent::Ollama | Agent::LmStudio => {
            write_skill(&ws.join(".agents/skills/blender-zaxis"))?;
            write(ws, "AGENTS.md", &inline_skill(".agents/skills/blender-zaxis"))
        }
        Agent::Gemini => {
            write_skill(&ws.join(".gemini/skills/blender-zaxis"))?;
            let mut server = server;
            server["trust"] = json!(true);
            write(ws, ".gemini/settings.json", &pretty(json!({"mcpServers": {"zaxis": server}})))?;
            write(ws, "GEMINI.md", &inline_skill(".gemini/skills/blender-zaxis"))
        }
    }
}

fn write_skill(dir: &Path) -> std::io::Result<()> {
    write(dir, "SKILL.md", SKILL)?;
    for (name, body) in REFS {
        write(&dir.join("references"), name, body)?;
    }
    Ok(())
}

/// Context-file form for agents without skill loading: the skill body minus frontmatter, links rebased.
fn inline_skill(skill_dir: &str) -> String {
    let body = SKILL.splitn(3, "---").nth(2).unwrap_or(SKILL).trim_start();
    body.replace("](references/", &format!("]({skill_dir}/references/"))
}

fn write(dir: &Path, name: &str, body: &str) -> std::io::Result<()> {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap_or(dir))?;
    std::fs::write(path, body)
}

fn pretty(v: serde_json::Value) -> String {
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_skill_strips_frontmatter_and_rebases_links() {
        let s = inline_skill(".agents/skills/blender-zaxis");
        assert!(s.starts_with("# Blender via Z-Axis"));
        assert!(s.contains("](.agents/skills/blender-zaxis/references/region-edit.md)"));
        assert!(!s.contains("](references/"));
    }
}
