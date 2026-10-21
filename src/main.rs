// GUI subsystem: no console window. `zaxis mcp` still gets the pipes its parent agent hands it.
#![windows_subsystem = "windows"]

mod agent;
mod app;
mod blender;
mod glb;
mod mcp;
mod select;
mod skills;
mod sys;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(PathBuf::from);
    let workspace = flag("--workspace").unwrap_or_else(default_workspace);
    if args.get(1).map(String::as_str) == Some("mcp") {
        mcp::run(workspace);
    } else {
        app::run(workspace);
    }
}

fn default_workspace() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    home.join("Documents").join("Z-Axis")
}
