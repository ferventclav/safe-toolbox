//! Reads the GeForce driver status from the NVIDIA app's local MCP server (http://127.0.0.1:13508/mcp).
//! The NVIDIA app does not let other programs install drivers, so a newer driver is shown with a link to the app.

use std::time::Duration;

use serde_json::{Value, json};

use super::model::{ItemKind, SourceResult, UpdateItem, UpdateSource};
use super::sources::str_of;
use super::versions;
use crate::proc::{self, Cancel};
use crate::tools::{contains_ic, program_files};

const URL: &str = "http://127.0.0.1:13508/mcp";

#[derive(Clone, Debug, PartialEq)]
pub struct DriverStatus {
    pub installed: String,
    pub channel: String,
    pub recommended: Vec<(String, String)>,
}

const NOT_OPEN: &str = "The NVIDIA app isn't open (or isn't installed), so the graphics driver wasn't checked. Open it and check again.";

pub fn check(cancel: &Cancel) -> SourceResult {
    // No NVIDIA card: nothing to show. (AMD and Intel drivers come through Windows Update.)
    let gpu = proc::powershell("(Get-CimInstance Win32_VideoController).Name -join '|'", Duration::from_secs(40), Some(cancel));
    if gpu.ok() && !contains_ic(&gpu.output, "NVIDIA") {
        return SourceResult::absent(UpdateSource::Nvidia);
    }
    let Some(body) = fetch_status() else { return SourceResult::message(UpdateSource::Nvidia, NOT_OPEN) };
    let Some(status) = parse_driver_status(&body) else {
        return SourceResult::message(UpdateSource::Nvidia, "The NVIDIA app answered, but not with a driver status.");
    };
    let items: Vec<UpdateItem> = status
        .recommended
        .iter()
        .filter(|(v, _)| versions::compare(v, &status.installed).is_gt())
        .map(|(v, name)| {
            let mut i = UpdateItem::new(UpdateSource::Nvidia, format!("nvidia-driver-{v}"), if name.is_empty() { "GeForce driver" } else { name })
                .versions(&status.installed, v);
            i.kind = ItemKind::Info;
            i.selectable = false;
            i
        })
        .collect();
    let when_empty = format!("Driver {} ({}) is the newest", status.installed, pretty(&status.channel));
    SourceResult::items(UpdateSource::Nvidia, items, &when_empty)
}

fn fetch_status() -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
    let post = |session: Option<&str>, payload: Value| -> Option<(String, Option<String>)> {
        let mut req = agent
            .post(URL)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(s) = session {
            req = req.header("Mcp-Session-Id", s);
        }
        let mut res = req.send(payload.to_string()).ok()?;
        let id = res.headers().get("Mcp-Session-Id").and_then(|v| v.to_str().ok()).map(str::to_string).or(session.map(str::to_string));
        let body = res.body_mut().read_to_string().unwrap_or_default();
        Some((body, id))
    };
    let (_, session) = post(
        None,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "SafeUpdater", "version": "0.1.0"}}}),
    )?;
    post(session.as_deref(), json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
    let (body, _) = post(
        session.as_deref(),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "nvapp_client_get_driver_status", "arguments": {}}}),
    )?;
    Some(body)
}

/// The JSON-RPC message in a response body, plain or as server-sent events ("data: {...}").
pub fn extract_json(body: &str) -> Option<String> {
    let t = body.trim();
    if t.starts_with('{') {
        return Some(t.to_string());
    }
    body.split('\n')
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .last()
        .map(str::to_string)
}

pub fn parse_driver_status(body: &str) -> Option<DriverStatus> {
    let rpc: Value = serde_json::from_str(&extract_json(body)?).ok()?;
    let result = rpc.get("result")?;
    // The NVIDIA app sends the data in structuredContent; its text content is a readable summary, not JSON.
    if let Some(s) = result.get("structuredContent").filter(|v| v.is_object()) {
        return from_status(s);
    }
    let text = result.get("content")?.as_array()?.first()?.get("text")?.as_str()?;
    from_status(&serde_json::from_str(text).ok()?)
}

fn from_status(root: &Value) -> Option<DriverStatus> {
    let inst = root.get("installedDriver")?;
    let recommended = root
        .get("recommendedDrivers")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|r| (str_of(r, "version"), str_of(r, "name"))).collect())
        .unwrap_or_default();
    Some(DriverStatus { installed: str_of(inst, "version"), channel: str_of(inst, "channel"), recommended })
}

/// Starts the NVIDIA app if it is installed in the usual place.
pub fn open_app() -> bool {
    let pf = program_files();
    for exe in [pf.join(r"NVIDIA Corporation\NVIDIA App\CEF\NVIDIA App.exe"), pf.join(r"NVIDIA Corporation\NVIDIA app\CEF\NVIDIA app.exe")] {
        if exe.is_file() {
            return crate::shell::open(&exe.to_string_lossy());
        }
    }
    false
}

fn pretty(channel: &str) -> String {
    match channel.replace('_', " ").to_lowercase().as_str() {
        "game ready" => "Game Ready".into(),
        "studio" => "Studio".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_structured_and_sse_bodies() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"structuredContent\":{\"installedDriver\":{\"version\":\"621.24\",\"channel\":\"GAME_READY\"},\"recommendedDrivers\":[{\"version\":\"622.10\",\"name\":\"GeForce Game Ready Driver\"}]}}}\n\n";
        let s = parse_driver_status(body).unwrap();
        assert_eq!(s.installed, "621.24");
        assert_eq!(s.recommended[0].0, "622.10");
        assert_eq!(pretty(&s.channel), "Game Ready");
        let text = r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"installedDriver\":{\"version\":\"1.0\",\"channel\":\"studio\"}}"}]}}"#;
        assert_eq!(parse_driver_status(text).unwrap().channel, "studio");
        assert!(parse_driver_status("garbage").is_none());
    }
}
