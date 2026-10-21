//! Launches agent CLIs headless and maps their JSON-lines output to chat events.

use crate::skills;
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Agent {
    Claude,
    Codex,
    Gemini,
    Ollama,
    LmStudio,
}

impl Agent {
    pub const ALL: [Agent; 5] = [Agent::Claude, Agent::Codex, Agent::Gemini, Agent::Ollama, Agent::LmStudio];

    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex CLI",
            Agent::Gemini => "Gemini CLI",
            Agent::Ollama => "Ollama (local, via Codex)",
            Agent::LmStudio => "LM Studio (local, via Codex)",
        }
    }

    fn cli(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Gemini => "gemini",
            _ => "codex",
        }
    }

    /// Model used when the model box is empty; local providers need one.
    pub fn default_model(self) -> &'static str {
        match self {
            Agent::Ollama => "gpt-oss:20b",
            Agent::LmStudio => "openai/gpt-oss-20b",
            _ => "",
        }
    }

    /// Full path to the CLI, or None if it isn't installed. `where` also finds npm's .cmd shims.
    pub fn find_exe(self) -> Option<PathBuf> {
        let out = Command::new("where.exe").arg(self.cli()).creation_flags(CREATE_NO_WINDOW).output().ok()?;
        String::from_utf8_lossy(&out.stdout).lines().map(PathBuf::from).find(|p| {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("cmd"))
        })
    }
}

pub struct Run<'a> {
    pub agent: Agent,
    pub cli: &'a Path,
    pub model: &'a str,
    pub session: Option<&'a str>,
    pub images: &'a [PathBuf],
    pub workspace: &'a Path,
    pub zaxis_exe: &'a Path,
}

/// The prompt is written to stdin by the caller (avoids Windows quoting limits on .cmd shims).
pub fn command(r: &Run) -> Command {
    let mut c = Command::new(r.cli);
    c.current_dir(r.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let model = if r.model.is_empty() { r.agent.default_model() } else { r.model };
    match r.agent {
        Agent::Claude => {
            c.args(["-p", "--output-format", "stream-json", "--verbose", "--strict-mcp-config", "--mcp-config"]);
            c.arg(r.workspace.join(".mcp.json"));
            c.args(["--allowedTools", "mcp__zaxis__*,Read,Skill"]);
            if let Some(s) = r.session {
                c.args(["--resume", s]);
            }
            if !model.is_empty() {
                c.args(["--model", model]);
            }
        }
        Agent::Codex | Agent::Ollama | Agent::LmStudio => {
            c.arg("exec");
            if let Some(s) = r.session {
                c.args(["resume", s]);
            }
            c.args(["--json", "--skip-git-repo-check"]);
            // TOML literal strings ('...') so Windows backslashes need no escaping.
            c.arg("-c").arg(format!("mcp_servers.zaxis.command='{}'", r.zaxis_exe.display()));
            let args: Vec<String> = skills::mcp_args(r.workspace).iter().map(|a| format!("'{a}'")).collect();
            c.arg("-c").arg(format!("mcp_servers.zaxis.args=[{}]", args.join(",")));
            c.arg("-c").arg("mcp_servers.zaxis.default_tools_approval_mode='approve'");
            match r.agent {
                Agent::Ollama => c.args(["--oss", "--local-provider", "ollama"]),
                Agent::LmStudio => c.args(["--oss", "--local-provider", "lmstudio"]),
                _ => &mut c,
            };
            if !model.is_empty() {
                c.args(["-m", model]);
            }
            for img in r.images {
                c.arg("-i").arg(img);
            }
            c.arg("-");
        }
        Agent::Gemini => {
            c.args(["--output-format", "stream-json", "--allowed-mcp-server-names", "zaxis", "-p", " "]);
            if !model.is_empty() {
                c.args(["-m", model]);
            }
        }
    }
    c
}

/// Builds the text sent on stdin: region note + reference paths + user text.
pub fn prompt(agent: Agent, user: &str, region: Option<&str>, refs: &[PathBuf]) -> String {
    let mut p = String::new();
    if let Some(r) = region {
        p.push_str(&format!("Active region: {r}. Call blender_get_region and only modify faces in vertex group `zaxis_region`.\n\n"));
    }
    if !refs.is_empty() {
        p.push_str("Reference images (look at them first):\n");
        for f in refs {
            // Gemini inlines files written as @path; the others read paths with their own tools.
            let at = if agent == Agent::Gemini { "@" } else { "" };
            p.push_str(&format!("- {at}{}\n", f.display()));
        }
        p.push('\n');
    }
    p.push_str(user);
    p
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Text(String),
    Tool(String),
    Session(String),
    Error(String),
    Done,
}

pub fn parse(agent: Agent, line: &str) -> Vec<Event> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let mut ev = vec![];
    match agent {
        Agent::Claude => match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => ev.push(Event::Session(s(&v["session_id"]))),
            Some("assistant") => {
                for c in v["message"]["content"].as_array().into_iter().flatten() {
                    match c["type"].as_str() {
                        Some("text") => ev.push(Event::Text(s(&c["text"]) + "\n")),
                        Some("tool_use") => ev.push(Event::Tool(s(&c["name"]))),
                        _ => {}
                    }
                }
            }
            Some("result") => {
                if v["is_error"] == true {
                    ev.push(Event::Error(s(&v["result"])));
                }
                ev.push(Event::Done);
            }
            _ => {}
        },
        Agent::Codex | Agent::Ollama | Agent::LmStudio => match v["type"].as_str() {
            Some("thread.started") => ev.push(Event::Session(s(&v["thread_id"]))),
            Some("item.started") if v["item"]["type"] == "mcp_tool_call" => ev.push(Event::Tool(s(&v["item"]["tool"]))),
            Some("item.started") if v["item"]["type"] == "command_execution" => ev.push(Event::Tool(s(&v["item"]["command"]))),
            Some("item.completed") if v["item"]["type"] == "agent_message" => ev.push(Event::Text(s(&v["item"]["text"]) + "\n")),
            Some("turn.failed") => ev.extend([Event::Error(s(&v["error"]["message"])), Event::Done]),
            Some("error") => ev.push(Event::Error(s(&v["message"]))),
            Some("turn.completed") => ev.push(Event::Done),
            _ => {}
        },
        Agent::Gemini => match v["type"].as_str() {
            Some("init") => ev.push(Event::Session(s(&v["session_id"]))),
            Some("message") if v["role"] == "assistant" => ev.push(Event::Text(s(&v["content"]))),
            Some("tool_use") => ev.push(Event::Tool(s(&v["tool_name"]))),
            Some("result") => {
                if v["status"] == "error" {
                    ev.push(Event::Error(s(&v["error"]["message"])));
                }
                ev.push(Event::Done);
            }
            _ => {}
        },
    }
    ev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_stream() {
        let lines = include_str!("../tests/fixtures/claude.jsonl");
        let ev: Vec<Event> = lines.lines().flat_map(|l| parse(Agent::Claude, l)).collect();
        assert_eq!(ev[0], Event::Session("abc".into()));
        assert!(ev.contains(&Event::Tool("mcp__zaxis__blender_scene_info".into())));
        assert!(ev.contains(&Event::Text("Done.\n".into())));
        assert_eq!(ev.last(), Some(&Event::Done));
    }

    #[test]
    fn parses_codex_stream() {
        let lines = include_str!("../tests/fixtures/codex.jsonl");
        let ev: Vec<Event> = lines.lines().flat_map(|l| parse(Agent::Codex, l)).collect();
        assert_eq!(ev[0], Event::Session("t1".into()));
        assert!(ev.contains(&Event::Tool("blender_scene_info".into())));
        assert!(ev.contains(&Event::Text("Done.\n".into())));
        assert_eq!(ev.last(), Some(&Event::Done));
    }

    #[test]
    fn prompt_includes_region_and_refs() {
        let p = prompt(Agent::Gemini, "make it red", Some("object Box, 3 faces"), &[PathBuf::from("C:/r/a.png")]);
        assert!(p.contains("Active region: object Box, 3 faces"));
        assert!(p.contains("- @C:/r/a.png"));
        assert!(p.ends_with("make it red"));
    }

    /// Live run against a real CLI + Blender bridge (tests/serve_blender.py). Opt-in:
    /// `ZAXIS_E2E=claude cargo test e2e -- --ignored --nocapture` (also codex, ollama, lmstudio, gemini).
    #[test]
    #[ignore]
    fn e2e_agent_exports_a_version() {
        use std::io::{BufRead, BufReader, Write};
        let name = std::env::var("ZAXIS_E2E").unwrap_or_else(|_| "claude".into());
        let agent = *Agent::ALL.iter().find(|a| format!("{a:?}").eq_ignore_ascii_case(&name)).expect("unknown agent");
        let ws = std::env::temp_dir().join(format!("zaxis-e2e-{name}"));
        let _ = std::fs::remove_dir_all(&ws);
        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/zaxis.exe");
        skills::install(agent, &ws, &exe).unwrap();
        let cli = agent.find_exe().expect("CLI not installed");
        let model = std::env::var("ZAXIS_E2E_MODEL").unwrap_or_default();
        let mut child = command(&Run { agent, cli: &cli, model: &model, session: None, images: &[], workspace: &ws, zaxis_exe: &exe }).spawn().unwrap();
        child.stdin.take().unwrap().write_all(prompt(agent, "Add a UV sphere named Ball at x=3 and export.", None, &[]).as_bytes()).unwrap();
        let mut events = vec![];
        for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
            let ev = parse(agent, &line);
            if ev.is_empty() {
                eprintln!("  (unparsed) {}", &line[..line.len().min(160)]);
            }
            for e in ev {
                eprintln!("{e:?}");
                events.push(e);
            }
        }
        let mut err = String::new();
        let _ = std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err);
        eprintln!("exit: {:?}
stderr tail: {}", child.wait(), &err[err.len().saturating_sub(800)..]);
        assert!(events.iter().any(|e| matches!(e, Event::Session(_))));
        assert!(events.iter().any(|e| matches!(e, Event::Tool(t) if t.contains("blender_"))));
        assert!(ws.join("versions/v001.glb").exists(), "agent did not export");
    }
}
