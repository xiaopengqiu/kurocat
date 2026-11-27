use crate::cats::{
    all_cats, cat_exists, cat_schema_guide, cats_path, delete_custom_cat, patch_custom_cat,
    sanitize_cat_id, upsert_custom_cat, CatSkinConfig,
};
use crate::config::{load_settings, save_settings, set_windows_autostart};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SERVER_NAME: &str = "kurocat";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentErrorKind {
    Tool,
    Task,
}

/// Real-time AI Coding Companion / pet interaction payload broadcast to both the desktop
/// pet window (`kuro-agent-event`) and the OBS Browser Source SSE stream (`agentEvent`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCompanionEvent {
    /// "status" | "action" | "bubble"
    pub kind: String,
    /// "working" | "thinking" | "waiting" | "done" | "error" | "idle"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// "happy" | "pet" | "angry" | "curious" | "sleep" | "wake" | "tail_pat"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_is_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<AgentErrorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_type: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<AgentTurnContext>,
    #[serde(default)]
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub start: bool,
    #[serde(default)]
    pub subagent: bool,
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn clean_short_text(raw: &str, max_chars: usize) -> Option<String> {
    let collapsed: String = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if collapsed.is_empty() {
        return None;
    }
    let count = collapsed.chars().count();
    if count <= max_chars {
        Some(collapsed)
    } else {
        let mut s: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        Some(s)
    }
}

fn normalize_status(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "working" | "coding" | "running" | "busy" | "typing" | "tool" => Some("working"),
        "thinking" | "planning" | "reasoning" | "prompt" => Some("thinking"),
        "waiting" | "waiting_approval" | "permission" | "approval" | "confirm" | "input" => {
            Some("waiting")
        }
        "done" | "complete" | "completed" | "finished" | "success" | "stop" => Some("done"),
        "error" | "failed" | "failure" | "fail" => Some("error"),
        "idle" | "clear" | "off" | "reset" | "cancel" | "canceled" | "cancelled" => Some("idle"),
        _ => None,
    }
}

fn normalize_action(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "happy" | "celebrate" | "cheer" | "smile" => Some("happy"),
        "pet" | "purr" | "pat" => Some("pet"),
        "angry" | "cross" | "mad" | "huff" | "grumpy" => Some("angry"),
        "curious" | "alert" | "look" => Some("curious"),
        "sleep" | "nap" | "zzz" => Some("sleep"),
        "wake" | "wakeup" => Some("wake"),
        "tail_pat" | "tail" | "slap" => Some("tail_pat"),
        _ => None,
    }
}

fn label_for_tool(tool_name: &str) -> (&'static str, bool) {
    let lower = tool_name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return ("处理任务中…", true);
    }
    if lower.contains("edit")
        || lower.contains("write")
        || lower.contains("patch")
        || lower.contains("replace")
        || lower.contains("notebook")
    {
        ("写代码中…", true)
    } else if lower.contains("command")
        || lower.contains("bash")
        || lower.contains("shell")
        || lower.contains("exec")
        || lower.contains("terminal")
    {
        ("运行命令中…", true)
    } else if lower.contains("browser") || lower.contains("web") || lower.contains("fetch") {
        ("查阅资料中…", true)
    } else if lower.contains("read")
        || lower.contains("grep")
        || lower.contains("search")
        || lower.contains("find")
        || lower.contains("list")
        || lower.contains("glob")
        || lower.contains("view")
    {
        ("阅读代码中…", true)
    } else if lower.contains("subagent") || lower.contains("agent") || lower.contains("task") {
        ("协作处理中…", true)
    } else {
        ("调用工具中…", true)
    }
}

fn detect_hook_source(raw: &Value, source_hint: Option<&str>) -> String {
    if let Some(s) = source_hint.and_then(|v| clean_short_text(v, 24)) {
        return s;
    }
    if let Some(s) = raw
        .get("source")
        .and_then(|v| v.as_str())
        .and_then(|v| clean_short_text(v, 24))
    {
        return s;
    }
    let tp = raw
        .get("transcriptPath")
        .or_else(|| raw.get("transcript_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if tp.contains("doubao") {
        return "豆包工作版".to_string();
    }
    if tp.contains("workbuddy") || tp.contains("codebuddy") {
        return "WorkBuddy".to_string();
    }
    if tp.contains("deepseek") || tp.contains("/.dsh/") || tp.contains("\\.dsh\\") {
        return "DeepSeek".to_string();
    }
    if tp.contains("codex")
        || raw
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t == "agent-turn-complete")
    {
        return "Codex".to_string();
    }
    if tp.contains("qwen") {
        return "千问".to_string();
    }
    if tp.contains("antigravity") || raw.get("hookEventName").is_some() {
        return "Antigravity".to_string();
    }
    if tp.contains("cursor") {
        return "Cursor".to_string();
    }
    if tp.contains("claude") || raw.get("hook_event_name").is_some() {
        return "Claude".to_string();
    }
    "AI".to_string()
}

/// Parses native hook JSON payloads from Antigravity, Claude Code, Tencent WorkBuddy/CodeBuddy,
/// OpenAI Codex CLI (`hooks.json` & `notify`), Cursor, or simplified direct HTTP/CLI payloads.
pub fn parse_hook_payload(
    raw: &Value,
    source_hint: Option<&str>,
    event_hint: Option<&str>,
    text_hint: Option<&str>,
) -> Option<AgentCompanionEvent> {
    let hook_event = raw
        .get("hookEventName")
        .or_else(|| raw.get("hook_event_name"))
        .or_else(|| raw.get("event"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let mut event = normalize_hook_payload(raw, source_hint, event_hint, text_hint, &hook_event)?;
    let session_id = ["sessionId", "session_id", "thread-id", "thread_id"]
        .into_iter()
        .filter_map(|key| raw.get(key).and_then(Value::as_str))
        .find_map(|id| clean_short_text(id, 128));
    let start = matches!(hook_event.as_str(), "preinvocation" | "userpromptsubmit" | "beforesubmitprompt");
    let subagent = matches!(hook_event.as_str(), "subagentstart" | "subagentstop");
    if event.kind == "status" && (session_id.is_some() || start || subagent) {
        event.turn = Some(AgentTurnContext { session_id, start, subagent });
    }
    Some(event)
}

fn normalize_hook_payload(
    raw: &Value,
    source_hint: Option<&str>,
    event_hint: Option<&str>,
    text_hint: Option<&str>,
    hook_event: &str,
) -> Option<AgentCompanionEvent> {
    let ts = now_epoch_ms();
    let source = detect_hook_source(raw, source_hint);
    let override_text = text_hint
        .or_else(|| raw.get("text").and_then(|v| v.as_str()))
        .or_else(|| raw.get("message").and_then(|v| v.as_str()))
        .and_then(|s| clean_short_text(s, 72));
    let explicit_sound = raw.get("sound").and_then(|v| v.as_bool());
    let explicit_auto_type = raw.get("autoType").and_then(|v| v.as_bool());
    let explicit_ttl = raw
        .get("ttlMs")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1_000, 300_000));

    // 1) Simplified / direct payload: explicit `action`, `status`, or CLI `--event`
    if let Some(act_raw) = raw.get("action").and_then(|v| v.as_str()) {
        if let Some(action) = normalize_action(act_raw) {
            return Some(AgentCompanionEvent {
                kind: "action".to_string(),
                status: None,
                action: Some(action.to_string()),
                source: Some(source),
                text_is_default: None,
                error_kind: None,
                text: override_text,
                tool: None,
                auto_type: Some(false),
                sound: explicit_sound,
                ttl_ms: Some(explicit_ttl.unwrap_or(5_000)),
                turn: None,
                ts,
            });
        }
    }

    if let Some(st_raw) = event_hint.or_else(|| raw.get("status").and_then(|v| v.as_str())) {
        if let Some(status) = normalize_status(st_raw) {
            let error_kind = (status == "error").then(|| {
                if raw.get("errorKind").and_then(Value::as_str) == Some("tool") {
                    AgentErrorKind::Tool
                } else {
                    AgentErrorKind::Task
                }
            });
            let (default_text, default_auto_type, default_ttl, default_sound) = match status {
                "working" => (Some("写代码中…".to_string()), true, 45_000, false),
                "thinking" => (Some("思考中…".to_string()), false, 35_000, false),
                "waiting" => (Some("等你确认操作".to_string()), false, 90_000, true),
                "done" => (Some("任务完成！".to_string()), false, 6_500, true),
                "error" => (None, false, 8_500, error_kind != Some(AgentErrorKind::Tool)),
                _ => (None, false, 1_500, false),
            };
            return Some(AgentCompanionEvent {
                kind: "status".to_string(),
                status: Some(status.to_string()),
                action: None,
                source: Some(source),
                text_is_default: (override_text.is_none() && default_text.is_some()).then_some(true),
                error_kind,
                text: override_text.or(default_text),
                tool: raw
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .and_then(|s| clean_short_text(s, 36)),
                auto_type: Some(explicit_auto_type.unwrap_or(default_auto_type)),
                sound: Some(explicit_sound.unwrap_or(default_sound)),
                ttl_ms: Some(explicit_ttl.unwrap_or(default_ttl)),
                turn: None,
                ts,
            });
        }
    }

    // 2) OpenAI Codex `notify` payload (`{"type": "agent-turn-complete", ...}`)
    if let Some(codex_type) = raw.get("type").and_then(|v| v.as_str()) {
        if codex_type.eq_ignore_ascii_case("agent-turn-complete") {
            return Some(AgentCompanionEvent {
                kind: "status".to_string(),
                status: Some("done".to_string()),
                action: None,
                source: Some(source),
                text_is_default: override_text.is_none().then_some(true),
                error_kind: None,
                text: override_text.or_else(|| Some("任务完成！".to_string())),
                tool: None,
                auto_type: Some(false),
                sound: Some(explicit_sound.unwrap_or(true)),
                ttl_ms: Some(explicit_ttl.unwrap_or(6_500)),
                turn: None,
                ts,
            });
        }
    }

    // 3) Native Lifecycle Hooks:
    //    - Antigravity: `hookEventName`, `toolCall.name`, `error`, `terminationReason`
    //    - Claude Code / WorkBuddy / Codex `hooks.json` / Cursor: `hook_event_name`, `tool_name`
    if hook_event.is_empty() {
        // Text-only bubble fallback
        if let Some(txt) = override_text {
            return Some(AgentCompanionEvent {
                kind: "bubble".to_string(),
                status: None,
                action: None,
                source: Some(source),
                text_is_default: None,
                error_kind: None,
                text: Some(txt),
                tool: None,
                auto_type: Some(false),
                sound: Some(explicit_sound.unwrap_or(false)),
                ttl_ms: Some(explicit_ttl.unwrap_or(5_000)),
                turn: None,
                ts,
            });
        }
        return None;
    }

    let tool_name = raw
        .get("toolCall")
        .and_then(|tc| tc.get("name"))
        .or_else(|| raw.get("tool_name"))
        .or_else(|| raw.get("toolName"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let err_str = raw
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let term_reason = raw
        .get("terminationReason")
        .or_else(|| raw.get("termination_reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    match hook_event {
        "sessionstart" | "session_start" => Some(AgentCompanionEvent {
            kind: "status".to_string(),
            status: Some("idle".to_string()),
            action: None,
            source: Some(source),
            text_is_default: None,
            error_kind: None,
            text: override_text.or_else(|| Some("已连接，待命中".to_string())),
            tool: None,
            auto_type: Some(false),
            sound: Some(false),
            ttl_ms: Some(explicit_ttl.unwrap_or(3_200)),
            turn: None,
            ts,
        }),
        "sessionend" | "session_end" => Some(AgentCompanionEvent {
            kind: "status".to_string(),
            status: Some("idle".to_string()),
            action: None,
            source: Some(source),
            text_is_default: None,
            error_kind: None,
            text: None,
            tool: None,
            auto_type: Some(false),
            sound: Some(false),
            ttl_ms: Some(1_000),
            turn: None,
            ts,
        }),
        "preinvocation" | "userpromptsubmit" | "beforesubmitprompt" => Some(AgentCompanionEvent {
            kind: "status".to_string(),
            status: Some("working".to_string()),
            action: None,
            source: Some(source),
            text_is_default: override_text.is_none().then_some(true),
            error_kind: None,
            text: override_text.or_else(|| Some("思考写代码中…".to_string())),
            tool: None,
            auto_type: Some(explicit_auto_type.unwrap_or(true)),
            sound: Some(false),
            ttl_ms: Some(explicit_ttl.unwrap_or(45_000)),
            turn: None,
            ts,
        }),
        "pretooluse" | "posttooluse" | "postinvocation" | "beforeshellexecution"
        | "afterfileedit" | "subagentstart" => {
            if !err_str.is_empty() {
                return Some(AgentCompanionEvent {
                    kind: "status".to_string(),
                    status: Some("error".to_string()),
                    action: None,
                    source: Some(source),
                    text_is_default: None,
                    error_kind: Some(AgentErrorKind::Tool),
                    text: override_text,
                    tool: clean_short_text(tool_name, 36),
                    auto_type: Some(false),
                    sound: Some(explicit_sound.unwrap_or(false)),
                    ttl_ms: Some(explicit_ttl.unwrap_or(8_000)),
                    turn: None,
                    ts,
                });
            }
            let (label, auto_type) = label_for_tool(tool_name);
            Some(AgentCompanionEvent {
                kind: "status".to_string(),
                status: Some("working".to_string()),
                action: None,
                source: Some(source),
                text_is_default: override_text.is_none().then_some(true),
                error_kind: None,
                text: override_text.or_else(|| Some(label.to_string())),
                tool: clean_short_text(tool_name, 36),
                auto_type: Some(explicit_auto_type.unwrap_or(auto_type)),
                sound: Some(false),
                ttl_ms: Some(explicit_ttl.unwrap_or(45_000)),
                turn: None,
                ts,
            })
        }
        "posttoolusefailure" => Some(AgentCompanionEvent {
            kind: "status".to_string(),
            status: Some("error".to_string()),
            action: None,
            source: Some(source),
            text_is_default: None,
            error_kind: Some(AgentErrorKind::Tool),
            text: override_text,
            tool: clean_short_text(tool_name, 36),
            auto_type: Some(false),
            sound: Some(explicit_sound.unwrap_or(false)),
            ttl_ms: Some(explicit_ttl.unwrap_or(8_500)),
            turn: None,
            ts,
        }),
        "permissionrequest" | "notification" => Some(AgentCompanionEvent {
            kind: "status".to_string(),
            status: Some("waiting".to_string()),
            action: None,
            source: Some(source),
            text_is_default: None,
            error_kind: None,
            text: override_text.or_else(|| Some("等你确认操作".to_string())),
            tool: clean_short_text(tool_name, 36),
            auto_type: Some(false),
            sound: Some(explicit_sound.unwrap_or(true)),
            ttl_ms: Some(explicit_ttl.unwrap_or(90_000)),
            turn: None,
            ts,
        }),
        "stop" | "subagentstop" => {
            if term_reason.eq_ignore_ascii_case("USER_CANCELED") {
                return Some(AgentCompanionEvent {
                    kind: "status".to_string(),
                    status: Some("idle".to_string()),
                    action: None,
                    source: Some(source),
                    text_is_default: None,
                    error_kind: None,
                    text: None,
                    tool: None,
                    auto_type: Some(false),
                    sound: Some(false),
                    ttl_ms: Some(1_000),
                    turn: None,
                    ts,
                });
            }
            if term_reason.eq_ignore_ascii_case("ERROR") || !err_str.is_empty() {
                return Some(AgentCompanionEvent {
                    kind: "status".to_string(),
                    status: Some("error".to_string()),
                    action: None,
                    source: Some(source),
                    text_is_default: None,
                    error_kind: Some(AgentErrorKind::Task),
                    text: override_text,
                    tool: None,
                    auto_type: Some(false),
                    sound: Some(explicit_sound.unwrap_or(true)),
                    ttl_ms: Some(explicit_ttl.unwrap_or(9_000)),
                    turn: None,
                    ts,
                });
            }
            Some(AgentCompanionEvent {
                kind: "status".to_string(),
                status: Some("done".to_string()),
                action: None,
                source: Some(source),
                text_is_default: override_text.is_none().then_some(true),
                error_kind: None,
                text: override_text.or_else(|| Some("任务完成！".to_string())),
                tool: None,
                auto_type: Some(false),
                sound: Some(explicit_sound.unwrap_or(true)),
                ttl_ms: Some(explicit_ttl.unwrap_or(6_500)),
                turn: None,
                ts,
            })
        }
        _ => None,
    }
}

fn post_loopback_json(port: u16, path: &str, payload: &Value) -> bool {
    let actual_port = if port == 0 { 27420 } else { port };
    let addr = SocketAddr::from(([127, 0, 0, 1], actual_port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let body = payload.to_string();
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        actual_port,
        body.as_bytes().len(),
        body
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let _ = stream.flush();
    let mut drain = [0u8; 256];
    let _ = stream.read(&mut drain);
    true
}

/// CLI entrypoint for `KuroCat --hook` used by Antigravity, Claude Code, WorkBuddy, and Codex hooks.
/// Never blocks, never slows down the AI host (<=250ms timeout), and always outputs `{}` on stdout.
pub fn run_hook_cli(args: &[String]) {
    let mut source_hint: Option<String> = None;
    let mut event_hint: Option<String> = None;
    let mut text_hint: Option<String> = None;
    let mut port_override: Option<u16> = None;
    let mut positional_json: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--hook" => {}
            "--source" if i + 1 < args.len() => {
                i += 1;
                source_hint = Some(args[i].clone());
            }
            "--event" | "--status" if i + 1 < args.len() => {
                i += 1;
                event_hint = Some(args[i].clone());
            }
            "--text" | "--message" if i + 1 < args.len() => {
                i += 1;
                text_hint = Some(args[i].clone());
            }
            "--port" if i + 1 < args.len() => {
                i += 1;
                port_override = args[i].parse::<u16>().ok();
            }
            other if other.trim_start().starts_with('{') => {
                positional_json = Some(other.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    let raw_payload: Value = if let Some(pos) = positional_json {
        serde_json::from_str(&pos).unwrap_or_else(|_| json!({}))
    } else if event_hint.is_none() && !std::io::stdin().is_terminal() {
        let mut buf = Vec::new();
        let _ = std::io::stdin().take(512 * 1024).read_to_end(&mut buf);
        serde_json::from_slice(&buf).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    let settings = load_settings();
    if settings.mcp_enabled {
        let port = port_override.unwrap_or(settings.mcp_port);
        if let Some(ev) = parse_hook_payload(
            &raw_payload,
            source_hint.as_deref(),
            event_hint.as_deref(),
            text_hint.as_deref(),
        ) {
            if let Ok(val) = serde_json::to_value(&ev) {
                let _ = post_loopback_json(port, "/agent/event", &val);
            }
        }
    }

    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{{}}");
    let _ = out.flush();
}

fn cat_skin_properties_schema() -> Value {
    json!({
        "id": { "type": "string", "description": "Unique lowercase slug ID, e.g. 'bengal-tiger', 'tibetan-fox', 'my-calico'" },
        "name": { "type": "string", "description": "Display name shown in menu, e.g. '霸气小老虎', '淡定藏狐'" },
        "emoji": { "type": "string", "description": "Optional emoji badge, e.g. '🐯', '🦊'" },
        "furColor": { "type": "string", "description": "Base coat hex color, e.g. '#f2a452'" },
        "furSecondaryColor": { "type": "string" },
        "outlineColor": { "type": "string", "description": "Outer stroke hex color for light coats, e.g. '#5d5148'" },
        "legLineColor": { "type": "string" },
        "pattern": { "type": "string", "enum": ["solid", "bicolor", "tuxedo", "tabby", "calico", "siamese"] },
        "faceShape": { "type": "string", "enum": ["round", "square", "wide", "fox", "oval"], "description": "Head silhouette: round (cat), square (Tibetan fox), wide (tiger/bear), fox (pointed chin), oval (bunny/capybara)" },
        "headWidth": { "type": "number", "minimum": 0.78, "maximum": 1.32, "description": "Horizontal head width multiplier (default 1.0)" },
        "jawSquareness": { "type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Jawline & cheek squareness (0.0 round .. 1.0 boxy square face)" },
        "earShape": { "type": "string", "enum": ["cat", "round", "tall", "small", "wide"], "description": "Ear silhouette: cat, round (tiger/bear/panda), tall (fox/bunny), small (Tibetan fox), wide (fennec)" },
        "earScale": { "type": "number", "minimum": 0.55, "maximum": 1.55, "description": "Ear size multiplier (default 1.0)" },
        "eyeSquint": { "type": "number", "minimum": 0.0, "maximum": 0.75, "description": "Resting upper-eyelid droop for deadpan/cool gaze (e.g. 0.50 for Tibetan fox)" },
        "bodyChubbiness": { "type": "number", "minimum": 0.80, "maximum": 1.25, "description": "Torso chubbiness multiplier (default 1.0)" },
        "maskColor": { "type": "string" },
        "foreheadBridgeColor": { "type": "string" },
        "flankColor": { "type": "string" },
        "bellyBibColor": { "type": "string" },
        "stripeColor": { "type": "string" },
        "calicoPatchColor": { "type": "string" },
        "earOuterColor": { "type": "string" },
        "earTipColor": { "type": "string" },
        "earInnerColor": { "type": "string" },
        "earTuftColor": { "type": "string" },
        "tailColor": { "type": "string" },
        "tailTipColor": { "type": "string" },
        "tailFluffy": { "type": "boolean" },
        "tailStripes": { "type": "boolean" },
        "eyeColorTop": { "type": "string" },
        "eyeColorBottom": { "type": "string" },
        "pupilColor": { "type": "string" },
        "eyeCatchlight": { "type": "boolean" },
        "eyeRimColor": { "type": "string" },
        "squintColor": { "type": "string" },
        "squintGlowColor": { "type": "string" },
        "noseColor": { "type": "string" },
        "mouthColor": { "type": "string" },
        "blushColor": { "type": "string" },
        "whiskerColor": { "type": "string" },
        "armColor": { "type": "string" },
        "pawColor": { "type": "string" },
        "pawOutlineColor": { "type": "string" },
        "armCreaseColor": { "type": "string" },
        "accentColor": { "type": "string", "description": "Pressed key & clicked mouse highlight hex color" },
        "accentSideColor": { "type": "string" },
        "spots": {
            "type": "array",
            "description": "Custom pet markings/spots (up to 16 in 3D, unlimited in 2D). rx/ry are in 2D canvas px (recommended 6..42).",
            "items": {
                "type": "object",
                "required": ["part", "x", "y", "rx", "ry", "color"],
                "properties": {
                    "part": { "type": "string", "enum": ["head", "body", "leftArm", "rightArm"] },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "rx": { "type": "number" },
                    "ry": { "type": "number" },
                    "rotation": { "type": "number" },
                    "color": { "type": "string" },
                    "opacity": { "type": "number" }
                }
            }
        }
    })
}

fn tool_definitions() -> Value {
    let skin_props = cat_skin_properties_schema();
    json!([
        {
            "name": "list_cats",
            "description": "List all available KuroCat desktop pet skins (built-in and custom pets) and show which pet is currently active on screen.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "get_cat_schema",
            "description": "Get the full CatSkinConfig JSON schema including faceShape/earShape/eyeSquint contour customization, pattern presets, spot coordinate & pixel units, and Tiger / Tibetan Fox examples.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "upsert_cat",
            "description": "Create a new custom cat/animal or update an existing one (coat colors, pattern, faceShape, jawSquareness, earShape, eyeSquint, tail, eyes, whiskers, accent color, spots[]). Hot-reloads live in both 2D & 3D! Set merge: true to patch only supplied fields on an existing cat.",
            "inputSchema": {
                "type": "object",
                "required": ["cat"],
                "properties": {
                    "cat": {
                        "type": "object",
                        "required": ["id"],
                        "properties": skin_props
                    },
                    "activate": {
                        "type": "boolean",
                        "default": true,
                        "description": "Immediately switch the live desktop pet to this skin (default: true)."
                    },
                    "merge": {
                        "type": "boolean",
                        "default": false,
                        "description": "When true and a cat with this ID already exists, merge only the provided fields onto it instead of replacing all fields."
                    }
                }
            }
        },
        {
            "name": "patch_cat",
            "description": "Partially update an existing cat/animal by ID (only send fields you want to change; keeps all other colors, contour settings, and spots intact).",
            "inputSchema": {
                "type": "object",
                "required": ["id", "patch"],
                "properties": {
                    "id": { "type": "string", "description": "Existing cat ID to modify" },
                    "patch": {
                        "type": "object",
                        "description": "Subset of CatSkinConfig fields to update",
                        "properties": skin_props
                    },
                    "activate": {
                        "type": "boolean",
                        "default": true,
                        "description": "Immediately switch to this cat after patching (default: true)."
                    }
                }
            }
        },
        {
            "name": "switch_cat",
            "description": "Switch the active pet displayed on the user's desktop by ID (e.g. 'black', 'ragdoll', 'dafeiyu', or any custom ID).",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string", "description": "Cat ID to activate" }
                }
            }
        },
        {
            "name": "delete_cat",
            "description": "Delete a custom cat skin (or reset a customized built-in cat back to default).",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string", "description": "Custom cat ID to delete" }
                }
            }
        },
        {
            "name": "get_settings",
            "description": "Read current KuroCat desktop pet settings (active pet, 3D/2D mode, scale, bubbles, purr sound, always-on-top, click-through lock, fpsCap, language, privacy guard, OBS, MCP).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "update_settings",
            "description": "Update KuroCat desktop pet settings live across Windows, macOS, and Linux.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "catBreed": { "type": "string" },
                    "renderMode": { "type": "string", "enum": ["2d", "3d"], "description": "Visual engine mode: '2d' or '3d'." },
                    "collarColor": { "type": "string", "enum": ["none", "red", "blue", "pink", "green"], "description": "Optional 3D collar & gold bell color." },
                    "headphoneMode": { "type": "string", "enum": ["auto", "on", "off"], "description": "Gaming headset mode." },
                    "scale": { "type": "number", "minimum": 0.2, "maximum": 1.5 },
                    "keyboardLayout": { "type": "string", "enum": ["65", "108"] },
                    "keyboardOrientation": { "type": "string", "enum": ["cat", "viewer"] },
                    "tailKeyMode": { "type": "string", "enum": ["smart", "custom", "edge", "off"] },
                    "tailKeyIds": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "showKeyBubbles": { "type": "boolean" },
                    "soundEnabled": { "type": "boolean" },
                    "alwaysOnTop": { "type": "boolean" },
                    "clickThroughLocked": { "type": "boolean" },
                    "privacyPasswordGuard": { "type": "boolean" },
                    "fpsCap": { "type": "integer", "enum": [0, 30, 60], "description": "Frame rate cap: 0 (auto 60), 30 (eco), or 60." },
                    "language": { "type": "string", "enum": ["auto", "zh-CN", "zh-TW", "en-US", "ja-JP", "ko-KR"] },
                    "obsEnabled": { "type": "boolean" },
                    "obsPort": { "type": "integer", "minimum": 1024, "maximum": 65535 },
                    "obsHidePet": { "type": "boolean" },
                    "autostart": { "type": "boolean" }
                }
            }
        },
        {
            "name": "set_agent_status",
            "description": "Drive KuroCat as an AI Coding Companion: show a floating status bubble above the cat, optionally animate the cat typing code automatically while working, and celebrate or alert when done / waiting / errored.",
            "inputSchema": {
                "type": "object",
                "required": ["status"],
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["working", "thinking", "waiting", "done", "error", "idle"],
                        "description": "working = cat auto-types code; thinking = curious head tilt; waiting = awaits user approval; done = happy completion alert; error = reports a tool or task failure; idle = clear status"
                    },
                    "errorKind": { "type": "string", "enum": ["tool", "task"], "description": "Applies to error status. Tool failures get a quiet reaction; task failures get a grumpy alert. Default: task." },
                    "text": { "type": "string", "description": "Short status text shown in the floating pill bubble (<=72 chars), e.g. '正在修复登录 Bug…' or '构建通过！'" },
                    "source": { "type": "string", "description": "Optional agent badge name, e.g. '豆包工作版', 'WorkBuddy', 'DeepSeek', 'Codex', '千问', 'Antigravity', 'Claude'" },
                    "autoType": { "type": "boolean", "description": "Whether the cat should virtually type on its keyboard (defaults to true for 'working')." },
                    "sound": { "type": "boolean", "description": "Play a gentle chime if sound is enabled (defaults to true for done, waiting and task failures)." },
                    "ttlMs": { "type": "integer", "minimum": 1000, "maximum": 300000, "description": "Auto-clear timeout in milliseconds." }
                }
            }
        },
        {
            "name": "trigger_action",
            "description": "Trigger a live pet expression/animation and optional speech bubble on the running desktop cat (and OBS stream).",
            "inputSchema": {
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["happy", "pet", "angry", "curious", "sleep", "wake", "tail_pat"],
                        "description": "Expression or animation to play immediately."
                    },
                    "text": { "type": "string", "description": "Optional floating bubble message to show with the action." },
                    "source": { "type": "string", "description": "Optional agent badge name." },
                    "sound": { "type": "boolean", "description": "Play a gentle sound if enabled." },
                    "ttlMs": { "type": "integer", "minimum": 1000, "maximum": 60000 }
                }
            }
        }
    ])
}

fn ok_text(payload: Value) -> Value {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false
    })
}

fn err_text(msg: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": msg }],
        "isError": true
    })
}

fn execute_tool(
    name: &str,
    args: &Value,
    on_change: &dyn Fn(),
    on_agent_event: &dyn Fn(AgentCompanionEvent),
) -> Value {
    match name {
        "list_cats" => {
            let settings = load_settings();
            let cats = all_cats();
            ok_text(json!({
                "activeCatId": settings.cat_breed,
                "storagePath": cats_path().to_string_lossy(),
                "count": cats.len(),
                "cats": cats
            }))
        }
        "get_cat_schema" => ok_text(cat_schema_guide()),
        "upsert_cat" => {
            let Some(cat_val) = args.get("cat") else {
                return err_text("Missing required 'cat' object argument.");
            };
            let merge = args.get("merge").and_then(|v| v.as_bool()).unwrap_or(false);
            let save_res = if merge {
                let Some(obj) = cat_val.as_object() else {
                    return err_text("'cat' must be a JSON object.");
                };
                let Some(id_raw) = obj.get("id").and_then(|v| v.as_str()) else {
                    return err_text("Missing required 'cat.id' string.");
                };
                if cat_exists(id_raw) {
                    patch_custom_cat(id_raw, obj)
                } else {
                    match serde_json::from_value::<CatSkinConfig>(cat_val.clone()) {
                        Ok(c) => upsert_custom_cat(c),
                        Err(e) => Err(format!("Invalid cat skin JSON: {}", e)),
                    }
                }
            } else {
                match serde_json::from_value::<CatSkinConfig>(cat_val.clone()) {
                    Ok(c) => upsert_custom_cat(c),
                    Err(e) => Err(format!("Invalid cat skin JSON: {}", e)),
                }
            };

            match save_res {
                Ok(saved) => {
                    let activate = args
                        .get("activate")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if activate {
                        let mut s = load_settings();
                        s.cat_breed = saved.id.clone();
                        save_settings(&s);
                    }
                    on_change();
                    ok_text(json!({
                        "status": "saved",
                        "activated": activate,
                        "cat": saved
                    }))
                }
                Err(e) => err_text(&format!("Failed to save custom cat: {}", e)),
            }
        }
        "patch_cat" => {
            let Some(id_raw) = args.get("id").and_then(|v| v.as_str()) else {
                return err_text("Missing required 'id' string argument.");
            };
            let Some(patch_obj) = args.get("patch").and_then(|v| v.as_object()) else {
                return err_text("Missing required 'patch' object argument.");
            };
            match patch_custom_cat(id_raw, patch_obj) {
                Ok(saved) => {
                    let activate = args
                        .get("activate")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if activate {
                        let mut s = load_settings();
                        s.cat_breed = saved.id.clone();
                        save_settings(&s);
                    }
                    on_change();
                    ok_text(json!({
                        "status": "patched",
                        "activated": activate,
                        "cat": saved
                    }))
                }
                Err(e) => err_text(&format!("Failed to patch cat: {}", e)),
            }
        }
        "switch_cat" => {
            let Some(id_raw) = args.get("id").and_then(|v| v.as_str()) else {
                return err_text("Missing required 'id' string argument.");
            };
            let id = sanitize_cat_id(id_raw);
            if !cat_exists(&id) {
                let ids: Vec<String> = all_cats().into_iter().map(|c| c.id).collect();
                return err_text(&format!(
                    "Cat '{}' not found. Available IDs: {:?}",
                    id, ids
                ));
            }
            let mut s = load_settings();
            s.cat_breed = id.clone();
            save_settings(&s);
            on_change();
            ok_text(json!({
                "status": "switched",
                "activeCatId": id
            }))
        }
        "delete_cat" => {
            let Some(id_raw) = args.get("id").and_then(|v| v.as_str()) else {
                return err_text("Missing required 'id' string argument.");
            };
            let id = sanitize_cat_id(id_raw);
            match delete_custom_cat(&id) {
                Ok(removed) => {
                    if removed {
                        let mut s = load_settings();
                        if s.cat_breed == id && !cat_exists(&id) {
                            s.cat_breed = "black".to_string();
                            save_settings(&s);
                        }
                        on_change();
                        ok_text(json!({
                            "status": "deleted",
                            "id": id
                        }))
                    } else {
                        err_text(&format!("No custom cat entry found for '{}'.", id))
                    }
                }
                Err(e) => err_text(&format!("Failed to delete cat: {}", e)),
            }
        }
        "get_settings" => ok_text(serde_json::to_value(load_settings()).unwrap_or(json!({}))),
        "update_settings" => {
            let mut s = load_settings();
            if let Some(breed) = args.get("catBreed").and_then(|v| v.as_str()) {
                let clean = sanitize_cat_id(breed);
                if cat_exists(&clean) {
                    s.cat_breed = clean;
                } else {
                    return err_text(&format!("Unknown catBreed '{}'.", clean));
                }
            }
            if let Some(scale) = args.get("scale").and_then(|v| v.as_f64()) {
                s.scale = scale.clamp(0.20, 1.50);
            }
            if let Some(rm) = args.get("renderMode").and_then(|v| v.as_str()) {
                s.render_mode = crate::config::normalize_render_mode(rm);
            }
            if let Some(cc) = args.get("collarColor").and_then(|v| v.as_str()) {
                s.collar_color = crate::config::normalize_collar_color(cc);
            }
            if let Some(hm) = args.get("headphoneMode").and_then(|v| v.as_str()) {
                s.headphone_mode = crate::config::normalize_headphone_mode(hm);
            }
            if let Some(layout) = args.get("keyboardLayout").and_then(|v| v.as_str()) {
                s.keyboard_layout = crate::config::normalize_keyboard_layout(layout);
            }
            if let Some(orient) = args.get("keyboardOrientation").and_then(|v| v.as_str()) {
                s.keyboard_orientation = crate::config::normalize_keyboard_orientation(orient);
            }
            if let Some(mode) = args.get("tailKeyMode").and_then(|v| v.as_str()) {
                s.tail_key_mode = crate::config::normalize_tail_key_mode(mode);
            }
            if let Some(arr) = args.get("tailKeyIds").and_then(|v| v.as_array()) {
                let raw: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                s.tail_key_ids = crate::config::normalize_tail_key_ids(&raw);
            }
            if let Some(b) = args.get("showKeyBubbles").and_then(|v| v.as_bool()) {
                s.show_key_bubbles = b;
            }
            if let Some(b) = args.get("soundEnabled").and_then(|v| v.as_bool()) {
                s.sound_enabled = b;
            }
            if let Some(b) = args.get("alwaysOnTop").and_then(|v| v.as_bool()) {
                s.always_on_top = b;
            }
            if let Some(b) = args.get("clickThroughLocked").and_then(|v| v.as_bool()) {
                s.click_through_locked = b;
            }
            if let Some(b) = args.get("privacyPasswordGuard").and_then(|v| v.as_bool()) {
                s.privacy_password_guard = b;
            }
            if let Some(cap) = args.get("fpsCap").and_then(|v| v.as_u64()) {
                s.fps_cap = crate::config::normalize_fps_cap(cap as u16);
            }
            if let Some(lang) = args.get("language").and_then(|v| v.as_str()) {
                s.language = crate::config::normalize_language(lang);
            }
            if let Some(b) = args.get("obsEnabled").and_then(|v| v.as_bool()) {
                s.obs_enabled = b;
                if !b {
                    s.obs_hide_pet = false;
                }
            }
            if let Some(port) = args.get("obsPort").and_then(|v| v.as_u64()) {
                if (1024..=65535).contains(&port) {
                    s.obs_port = port as u16;
                }
            }
            if let Some(b) = args.get("obsHidePet").and_then(|v| v.as_bool()) {
                s.obs_hide_pet = s.obs_enabled && b;
            }
            if let Some(b) = args.get("autostart").and_then(|v| v.as_bool()) {
                if set_windows_autostart(b).is_ok() {
                    s.autostart = b;
                }
            }
            save_settings(&s);
            on_change();
            ok_text(serde_json::to_value(&s).unwrap_or(json!({})))
        }
        "set_agent_status" => {
            let default_src = if args.get("source").and_then(|v| v.as_str()).is_some() {
                None
            } else {
                Some("MCP")
            };
            let Some(ev) = parse_hook_payload(args, default_src, None, None) else {
                return err_text("Invalid 'status'. Expected one of: working, thinking, waiting, done, error, idle.");
            };
            on_agent_event(ev.clone());
            ok_text(json!({
                "status": "dispatched",
                "event": ev
            }))
        }
        "trigger_action" => {
            let default_src = if args.get("source").and_then(|v| v.as_str()).is_some() {
                None
            } else {
                Some("MCP")
            };
            let Some(ev) = parse_hook_payload(args, default_src, None, None) else {
                return err_text("Invalid 'action'. Expected one of: happy, pet, angry, curious, sleep, wake, tail_pat.");
            };
            on_agent_event(ev.clone());
            ok_text(json!({
                "status": "dispatched",
                "event": ev
            }))
        }
        _ => err_text(&format!("Unknown tool: {}", name)),
    }
}

pub fn handle_jsonrpc(
    req: &Value,
    on_change: &dyn Fn(),
    on_agent_event: &dyn Fn(AgentCompanionEvent),
) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    // Notifications (no id) must not receive a JSON-RPC response
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let id = id.unwrap();

    let result = match method {
        "initialize" => {
            let client_ver = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            let ver = match client_ver {
                "2025-06-18" | "2025-03-26" | "2024-11-05" => client_ver,
                _ => "2024-11-05",
            };
            Ok(json!({
                "protocolVersion": ver,
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION
                },
                "instructions": "Use get_cat_schema to inspect the custom cat/animal format (including faceShape, jawSquareness, earShape, eyeSquint, and spots[]), upsert_cat or patch_cat to customize the live desktop pet in 2D & 3D, and set_agent_status / trigger_action to animate the pet as a real-time AI coding companion."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Ok(execute_tool(tool_name, &args, on_change, on_agent_event))
        }
        _ => Err((-32601, format!("Method not found: {}", method))),
    };

    Some(match result {
        Ok(res) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": res
        }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }),
    })
}

/// Transport 1: Zero-dependency native Stdio MCP server (`KuroCat --mcp`)
pub fn run_stdio_mcp() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let noop = || {};
    let forward_agent = |ev: AgentCompanionEvent| {
        let port = load_settings().mcp_port;
        if let Ok(val) = serde_json::to_value(&ev) {
            let _ = post_loopback_json(port, "/agent/event", &val);
        }
    };

    for line_res in stdin.lock().lines() {
        let Ok(line) = line_res else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                let _ = writeln!(out, "{}", err_resp);
                let _ = out.flush();
                continue;
            }
        };

        // Respect the user's MCP toggle on tool invocations even over Stdio
        let method = parsed.get("method").and_then(|m| m.as_str()).unwrap_or("");
        if method == "tools/call" && !load_settings().mcp_enabled {
            if let Some(id) = parsed.get("id").cloned() {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": err_text("MCP server is disabled in KuroCat settings. Enable 'MCP 扩展' in KuroCat's menu or tray.")
                });
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
            }
            continue;
        }

        if let Some(resp) = handle_jsonrpc(&parsed, &noop, &forward_agent) {
            let _ = writeln!(out, "{}", resp);
            let _ = out.flush();
        }
    }
}

/// Transport 2: Lightweight loopback-only Streamable HTTP MCP + Agent Hook endpoint
/// (`http://127.0.0.1:<port>/mcp`, `/hook`, `/agent/event`, `/health`).
pub fn start_http_mcp_server<F, G>(
    port: u16,
    enabled: Arc<AtomicBool>,
    on_change: F,
    on_agent_event: G,
) where
    F: Fn() + Send + Sync + 'static,
    G: Fn(AgentCompanionEvent) + Send + Sync + 'static,
{
    let on_change = Arc::new(on_change);
    let on_agent_event = Arc::new(on_agent_event);
    std::thread::spawn(move || {
        let addr = format!("127.0.0.1:{}", port);
        let Ok(listener) = TcpListener::bind(&addr) else {
            return;
        };
        for stream_res in listener.incoming() {
            let Ok(stream) = stream_res else {
                continue;
            };
            let flag = enabled.clone();
            let notify = on_change.clone();
            let agent_cb = on_agent_event.clone();
            std::thread::spawn(move || {
                handle_http_conn(stream, &flag, &*notify, &*agent_cb);
            });
        }
    });
}

fn write_http_response_with_headers(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    extra_headers: &str,
    body: &str,
) {
    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Accept, Mcp-Session-Id\r\n{}Connection: close\r\n\r\n{}",
        status,
        content_type,
        body.as_bytes().len(),
        extra_headers,
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn write_http_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    write_http_response_with_headers(stream, status, content_type, "", body);
}

fn read_chunked_body<R: BufRead>(reader: &mut R, max_bytes: usize) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        if reader.read_line(&mut size_line).map_err(|_| "io")? == 0 {
            return Err("eof");
        }
        let hex_part = size_line.split(';').next().unwrap_or("").trim();
        let chunk_len = usize::from_str_radix(hex_part, 16).map_err(|_| "bad_chunk")?;
        if chunk_len == 0 {
            // Drain trailing CRLF / optional trailer headers
            loop {
                let mut trailer = String::new();
                if reader.read_line(&mut trailer).unwrap_or(0) == 0 || trailer.trim().is_empty() {
                    break;
                }
            }
            return Ok(out);
        }
        if out.len().saturating_add(chunk_len) > max_bytes {
            return Err("too_large");
        }
        let start = out.len();
        out.resize(start + chunk_len, 0);
        reader
            .read_exact(&mut out[start..])
            .map_err(|_| "io")?;
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
}

fn read_http_body<R: BufRead>(
    reader: &mut R,
    content_length: usize,
    is_chunked: bool,
    max_bytes: usize,
) -> Result<Vec<u8>, &'static str> {
    if is_chunked {
        return read_chunked_body(reader, max_bytes);
    }
    if content_length > max_bytes {
        return Err("too_large");
    }
    let mut buf = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut buf).map_err(|_| "io")?;
    }
    Ok(buf)
}

fn handle_http_conn(
    mut stream: TcpStream,
    enabled: &AtomicBool,
    on_change: &dyn Fn(),
    on_agent_event: &dyn Fn(AgentCompanionEvent),
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }
    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    let method = parts[0];
    let raw_target = parts[1];
    let (path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));

    let mut content_length: usize = 0;
    let mut is_chunked = false;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return;
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            } else if k.eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
            {
                is_chunked = true;
            }
        }
    }

    if method == "OPTIONS" {
        write_http_response(&mut stream, "204 No Content", "text/plain", "");
        return;
    }

    if !enabled.load(Ordering::Relaxed) {
        write_http_response(
            &mut stream,
            "403 Forbidden",
            "application/json; charset=utf-8",
            r#"{"error":"MCP server is disabled in KuroCat settings"}"#,
        );
        return;
    }

    if method == "GET" && (path == "/health" || path == "/") {
        let body = json!({
            "ok": true,
            "name": SERVER_NAME,
            "version": SERVER_VERSION,
            "mcpEndpoint": "/mcp",
            "hookEndpoint": "/hook",
            "agentEventEndpoint": "/agent/event"
        })
        .to_string();
        write_http_response(&mut stream, "200 OK", "application/json; charset=utf-8", &body);
        return;
    }

    if method == "GET" && path == "/mcp" {
        // MCP Streamable HTTP spec: return 405 when server does not open unsolicited SSE streams
        write_http_response_with_headers(
            &mut stream,
            "405 Method Not Allowed",
            "application/json; charset=utf-8",
            "Allow: POST, DELETE, OPTIONS\r\n",
            r#"{"error":"Use POST /mcp for JSON-RPC requests"}"#,
        );
        return;
    }

    if method == "DELETE" && path == "/mcp" {
        write_http_response(&mut stream, "204 No Content", "text/plain", "");
        return;
    }

    if method == "POST" && (path == "/hook" || path == "/agent/event") {
        let buf = match read_http_body(&mut reader, content_length, is_chunked, 512 * 1024) {
            Ok(b) => b,
            Err("too_large") => {
                write_http_response(
                    &mut stream,
                    "413 Payload Too Large",
                    "application/json",
                    r#"{"error":"Payload too large"}"#,
                );
                return;
            }
            Err(_) => return,
        };
        let parsed: Value = if buf.is_empty() {
            json!({})
        } else {
            match serde_json::from_slice(&buf) {
                Ok(v) => v,
                Err(e) => {
                    let body = json!({ "ok": false, "error": format!("Invalid JSON: {}", e) }).to_string();
                    write_http_response(&mut stream, "400 Bad Request", "application/json; charset=utf-8", &body);
                    return;
                }
            }
        };

        let mut source_q: Option<String> = None;
        let mut event_q: Option<String> = None;
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "source" && !v.is_empty() {
                    source_q = Some(v.to_string());
                } else if (k == "event" || k == "status") && !v.is_empty() {
                    event_q = Some(v.to_string());
                }
            }
        }

        let dispatched = if path == "/agent/event" {
            if let Ok(mut ev) = serde_json::from_value::<AgentCompanionEvent>(parsed.clone()) {
                if ev.ts == 0 {
                    ev.ts = now_epoch_ms();
                }
                on_agent_event(ev.clone());
                Some(ev)
            } else {
                parse_hook_payload(&parsed, source_q.as_deref(), event_q.as_deref(), None).map(
                    |ev| {
                        on_agent_event(ev.clone());
                        ev
                    },
                )
            }
        } else {
            parse_hook_payload(&parsed, source_q.as_deref(), event_q.as_deref(), None).map(|ev| {
                on_agent_event(ev.clone());
                ev
            })
        };

        let body = json!({
            "ok": true,
            "dispatched": dispatched.is_some(),
            "event": dispatched
        })
        .to_string();
        write_http_response(&mut stream, "200 OK", "application/json; charset=utf-8", &body);
        return;
    }

    if method == "POST" && (path == "/mcp" || path == "/") {
        let buf = match read_http_body(&mut reader, content_length, is_chunked, 2 * 1024 * 1024) {
            Ok(b) => b,
            Err("too_large") => {
                write_http_response(
                    &mut stream,
                    "413 Payload Too Large",
                    "application/json",
                    r#"{"error":"Payload too large"}"#,
                );
                return;
            }
            Err(_) => return,
        };
        let parsed: Value = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                })
                .to_string();
                write_http_response(&mut stream, "400 Bad Request", "application/json; charset=utf-8", &err);
                return;
            }
        };

        if let Some(arr) = parsed.as_array() {
            let mut responses = Vec::new();
            for item in arr {
                if let Some(resp) = handle_jsonrpc(item, on_change, on_agent_event) {
                    responses.push(resp);
                }
            }
            if responses.is_empty() {
                write_http_response(&mut stream, "202 Accepted", "application/json", "");
            } else {
                let body = serde_json::to_string(&responses).unwrap_or_else(|_| "[]".to_string());
                write_http_response(&mut stream, "200 OK", "application/json; charset=utf-8", &body);
            }
        } else if let Some(resp) = handle_jsonrpc(&parsed, on_change, on_agent_event) {
            let body = resp.to_string();
            write_http_response(&mut stream, "200 OK", "application/json; charset=utf-8", &body);
        } else {
            write_http_response(&mut stream, "202 Accepted", "application/json", "");
        }
        return;
    }

    write_http_response(
        &mut stream,
        "404 Not Found",
        "application/json; charset=utf-8",
        r#"{"error":"Not found. Use POST /mcp, POST /hook, POST /agent/event, or GET /health"}"#,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_antigravity_lifecycle_hooks() {
        let pre_inv = json!({
            "hookEventName": "PreInvocation",
            "transcriptPath": "C:/Users/me/.gemini/antigravity/transcript.jsonl"
        });
        let ev = parse_hook_payload(&pre_inv, None, None, None).unwrap();
        assert_eq!(ev.source.as_deref(), Some("Antigravity"));
        assert_eq!(ev.status.as_deref(), Some("working"));
        assert_eq!(ev.auto_type, Some(true));

        let post_edit = json!({
            "hookEventName": "PostToolUse",
            "toolCall": { "name": "edit_file" },
            "error": ""
        });
        let ev2 = parse_hook_payload(&post_edit, None, None, None).unwrap();
        assert_eq!(ev2.status.as_deref(), Some("working"));
        assert_eq!(ev2.text.as_deref(), Some("写代码中…"));

        let post_err = json!({
            "hookEventName": "PostToolUse",
            "toolCall": { "name": "run_command" },
            "error": "exit status 1"
        });
        let ev3 = parse_hook_payload(&post_err, None, None, None).unwrap();
        assert_eq!(ev3.status.as_deref(), Some("error"));
        assert_eq!(ev3.error_kind, Some(AgentErrorKind::Tool));
        assert_eq!(ev3.text, None);
        assert_eq!(ev3.sound, Some(false));

        let stop_ok = json!({
            "hookEventName": "Stop",
            "terminationReason": "NO_TOOL_CALL",
            "fullyIdle": true
        });
        let ev4 = parse_hook_payload(&stop_ok, None, None, None).unwrap();
        assert_eq!(ev4.status.as_deref(), Some("done"));
        assert_eq!(ev4.sound, Some(true));

        let stop_cancel = json!({
            "hookEventName": "Stop",
            "terminationReason": "USER_CANCELED"
        });
        let ev5 = parse_hook_payload(&stop_cancel, None, None, None).unwrap();
        assert_eq!(ev5.status.as_deref(), Some("idle"));
    }

    #[test]
    fn relates_codex_prompt_and_notify_but_marks_subagent_completion() {
        let parse = |raw| parse_hook_payload(&raw, Some("Codex"), None, None).unwrap().turn.unwrap();
        let start = parse(json!({"hook_event_name": "UserPromptSubmit", "session_id": "thread-test"}));
        let done = parse(json!({"type": "agent-turn-complete", "thread-id": "thread-test"}));
        let subagent = parse(json!({"hook_event_name": "SubagentStop", "session_id": "thread-test"}));
        assert!(start.start);
        assert!(!done.start);
        assert_eq!(start.session_id, done.session_id);
        assert!(subagent.subagent);
    }

    #[test]
    fn parses_claude_workbuddy_and_codex_hooks() {
        let wb_perm = json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "transcript_path": "/home/u/.workbuddy/session.jsonl"
        });
        let ev = parse_hook_payload(&wb_perm, None, None, None).unwrap();
        assert_eq!(ev.source.as_deref(), Some("WorkBuddy"));
        assert_eq!(ev.status.as_deref(), Some("waiting"));
        assert_eq!(ev.auto_type, Some(false));

        let codex_notify = json!({
            "type": "agent-turn-complete",
            "thread-id": "t1"
        });
        let ev2 = parse_hook_payload(&codex_notify, None, None, None).unwrap();
        assert_eq!(ev2.source.as_deref(), Some("Codex"));
        assert_eq!(ev2.status.as_deref(), Some("done"));
    }

    #[test]
    fn identifies_failure_scope_and_preserves_custom_captions() {
        let tool = parse_hook_payload(&json!({
            "hook_event_name": "PostToolUseFailure", "text": "构建失败，等待日志", "sound": true
        }), Some("Codex"), None, None).unwrap();
        assert_eq!(tool.error_kind, Some(AgentErrorKind::Tool));
        assert_eq!(tool.text.as_deref(), Some("构建失败，等待日志"));
        assert_eq!(tool.sound, Some(true));

        let task = parse_hook_payload(&json!({
            "hook_event_name": "Stop", "termination_reason": "ERROR"
        }), Some("Codex"), None, None).unwrap();
        assert_eq!(task.error_kind, Some(AgentErrorKind::Task));
        assert_eq!(task.text, None);
        assert_eq!(serde_json::to_value(task).unwrap()["errorKind"], "task");

        let direct = parse_hook_payload(&json!({"status": "error", "errorKind": "tool"}),
            None, None, None).unwrap();
        assert_eq!(direct.error_kind, Some(AgentErrorKind::Tool));
        assert_eq!(direct.sound, Some(false));
    }

    #[test]
    fn labels_automatic_working_text_without_marking_user_copy() {
        let default = parse_hook_payload(&json!({"hook_event_name": "PreToolUse", "tool_name": "Bash"}),
            Some("Codex"), None, None).unwrap();
        assert_eq!(default.status.as_deref(), Some("working"));
        assert_eq!(default.text_is_default, Some(true));
        assert!(default.text.is_some());

        let custom = parse_hook_payload(&json!({"status": "working", "text": "重跑测试"}),
            Some("Codex"), None, None).unwrap();
        assert_eq!(custom.text_is_default, None);
        assert_eq!(custom.text.as_deref(), Some("重跑测试"));
    }

    #[test]
    fn decodes_http_chunked_body() {
        let wire = b"7\r\n{\"ok\": \r\n5\r\ntrue}\r\n0\r\n\r\n";
        let mut cursor = BufReader::new(&wire[..]);
        let body = read_chunked_body(&mut cursor, 1024).unwrap();
        assert_eq!(std::str::from_utf8(&body).unwrap(), "{\"ok\": true}");
    }
}
