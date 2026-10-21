//! `zaxis mcp --workspace <dir>`: stdio MCP server that forwards tool calls to the Blender addon.
//! Hand-rolled JSON-RPC: MCP stdio is newline-delimited JSON and we only need 4 methods.

use crate::blender;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub fn run(workspace: PathBuf) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(resp) = handle(&workspace, &line) else { continue };
        if writeln!(stdout, "{resp}").and_then(|_| stdout.flush()).is_err() {
            break;
        }
    }
}

/// Returns the response line, or None for notifications.
pub fn handle(ws: &Path, line: &str) -> Option<Value> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})),
    };
    let id = req.get("id")?.clone();
    let params = &req["params"];
    let result = match req["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "zaxis", "version": env!("CARGO_PKG_VERSION")},
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => call_tool(ws, params["name"].as_str().unwrap_or_default(), &params["arguments"]),
        m => {
            return Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unknown method {m}")}}));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn tools() -> Value {
    let no_args = json!({"type": "object", "properties": {}});
    json!([
        {"name": "blender_scene_info", "description": "List objects in the live Blender scene: type, transform, modifiers, materials, face count, vertex groups, current mode.", "inputSchema": no_args},
        {"name": "blender_exec", "description": "Run Python in the live Blender session (bpy imported). Returns print() output and the value of a `result` variable. One undo step per call.",
         "inputSchema": {"type": "object", "properties": {"code": {"type": "string"}}, "required": ["code"]}},
        {"name": "blender_screenshot", "description": "PNG screenshot of the Blender 3D viewport.", "inputSchema": no_args},
        {"name": "blender_export_glb", "description": "Export the scene as the next GLB version so the Z-Axis app shows it. Call after every completed change.",
         "inputSchema": {"type": "object", "properties": {"note": {"type": "string", "description": "Short description of the change"}}}},
        {"name": "blender_get_region", "description": "The model region the user captured in the Z-Axis app: object, face indices, vertex group name and a crop image path. Empty if none.", "inputSchema": no_args},
    ])
}

fn call_tool(ws: &Path, name: &str, args: &Value) -> Value {
    let out = match name {
        "blender_scene_info" => blender::call("scene_info", json!({})).map(text),
        "blender_exec" => blender::call("exec", json!({"code": args["code"]})).map(|r| {
            let mut s = r["stdout"].as_str().unwrap_or_default().to_string();
            if let Some(v) = r["result"].as_str() {
                s.push_str(&format!("\nresult = {v}"));
            }
            text_str(if s.trim().is_empty() { "ok (no output)".into() } else { s })
        }),
        "blender_screenshot" => {
            let path = ws.join("shots").join("viewport.png");
            blender::call("screenshot", json!({"path": path}))
                .and_then(|_| std::fs::read(&path).map_err(|e| e.to_string()))
                .map(|png| json!([{"type": "image", "mimeType": "image/png", "data": base64(&png)}]))
        }
        "blender_export_glb" => blender::call("export_glb", json!({"dir": ws.join("versions"), "note": args["note"].as_str().unwrap_or_default()})).map(text),
        "blender_get_region" => Ok(text_str(std::fs::read_to_string(ws.join("region.json")).unwrap_or_else(|_| "No active region.".into()))),
        _ => Err(format!("unknown tool {name}")),
    };
    match out {
        Ok(content) => json!({"content": content}),
        Err(e) => json!({"content": text_str(e), "isError": true}),
    }
}

fn text(v: Value) -> Value {
    text_str(serde_json::to_string_pretty(&v).unwrap_or_default())
}

fn text_str(s: String) -> Value {
    json!([{"type": "text", "text": s}])
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = u32::from(c[0]) << 16 | u32::from(*c.get(1).unwrap_or(&0)) << 8 | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            s.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn protocol_basics() {
        let ws = Path::new(".");
        let init = handle(ws, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(handle(ws, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        let list = handle(ws, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 5);
        let bad = handle(ws, r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
    }
}
