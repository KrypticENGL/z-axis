//! Client for the Blender addon's JSON-lines socket (see blender_addon/zaxis_bridge).

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

pub const ADDR: &str = "127.0.0.1:9876";

pub fn token_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    base.join("zaxis").join("token")
}

/// Sends one command and returns its `result`, or the addon's error text.
pub fn call(cmd: &str, args: Value) -> Result<Value, String> {
    let token = std::fs::read_to_string(token_path())
        .map_err(|_| "Blender bridge not running (no token). Start Blender with the Z-Axis Bridge add-on.".to_string())?;
    let mut stream = TcpStream::connect_timeout(&ADDR.parse().unwrap(), Duration::from_secs(3))
        .map_err(|e| format!("Cannot reach Blender on {ADDR}: {e}"))?;
    let req = json!({"token": token.trim(), "cmd": cmd, "args": args});
    writeln!(stream, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let mut resp: Value = serde_json::from_str(&line).map_err(|e| format!("Bad reply from Blender: {e}"))?;
    match resp.get("error") {
        Some(err) => Err(err.as_str().unwrap_or_default().to_string()),
        None => Ok(resp["result"].take()),
    }
}
