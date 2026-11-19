//! Gọi các CLI chính chủ ở chế độ không tương tác. Prompt luôn đi qua stdin.

use crate::Seat;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, process::Command};

const TIMEOUT_SECS: u64 = 900;

#[derive(Clone, Serialize, Default)]
pub struct CliInfo {
    pub path: Option<String>,
    pub version: Option<String>,
}

pub struct CliOut {
    pub text: String,
    pub tokens: u64,
    pub cost: f64,
}

pub struct CliErr {
    pub message: String,
    pub rate_limited: bool,
    pub reset_hint: String,
}

fn err(message: impl Into<String>) -> CliErr {
    let message = message.into();
    let (rate_limited, reset_hint) = detect_limit(&message);
    CliErr { message, rate_limited, reset_hint }
}

/// Tìm file thực thi (trên Windows: claude.cmd / claude.exe ...).
/// Có thể ghi đè bằng biến môi trường CLAUDE_BIN / CODEX_BIN / GEMINI_BIN.
pub fn resolve(name: &str) -> Option<PathBuf> {
    let var = format!("{}_BIN", name.to_uppercase());
    if let Ok(p) = std::env::var(&var) {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    which::which(name).ok()
}

pub async fn detect_all() -> HashMap<String, CliInfo> {
    let mut out = HashMap::new();
    for name in ["claude", "codex", "gemini"] {
        let mut info = CliInfo::default();
        if let Some(p) = resolve(name) {
            info.path = Some(p.display().to_string());
            let mut cmd = command(&p);
            cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
            if let Ok(Ok(o)) = tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let s = if s.is_empty() { String::from_utf8_lossy(&o.stderr).trim().to_string() } else { s };
                if o.status.success() {
                    info.version = Some(s.lines().next().unwrap_or("").chars().take(60).collect());
                }
            }
        }
        out.insert(name.to_string(), info);
    }
    out
}

fn command(path: &Path) -> Command {
    let mut c = Command::new(path);
    #[cfg(windows)]
    {
        // không bật cửa sổ console đen mỗi lần gọi
        c.creation_flags(0x0800_0000);
    }
    c.kill_on_drop(true);
    c
}

pub async fn run(workdir: &Path, seat: &Seat, prompt: &str) -> Result<CliOut, CliErr> {
    let Some(bin) = resolve(&seat.cli) else {
        return Err(CliErr { message: format!("Chưa cài `{}` (không thấy trong PATH).", seat.cli), rate_limited: false, reset_hint: String::new() });
    };
    let mut cmd = command(&bin);
    cmd.current_dir(workdir);
    let model = seat.model.trim();
    let mut last_msg: Option<PathBuf> = None;

    match seat.cli.as_str() {
        "claude" => {
            cmd.args(["-p", "--output-format", "json"]);
            if !model.is_empty() {
                cmd.args(["--model", model]);
            }
        }
        "codex" => {
            let f = std::env::temp_dir().join(format!("roomai-codex-{}-{}.txt", seat.id, std::process::id()));
            let _ = std::fs::remove_file(&f);
            cmd.args(["exec", "--skip-git-repo-check", "--sandbox", "read-only", "--output-last-message"]);
            cmd.arg(&f);
            if !model.is_empty() {
                cmd.args(["-m", model]);
            }
            cmd.arg("-");
            last_msg = Some(f);
        }
        "gemini" => {
            if !model.is_empty() {
                cmd.args(["-m", model]);
            }
        }
        other => return Err(err(format!("CLI không hỗ trợ: {other}"))),
    }

    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| err(format!("Không chạy được {}: {e}", bin.display())))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let output = match tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(err(format!("Lỗi khi chạy {}: {e}", seat.cli))),
        Err(_) => return Err(err(format!("{} chạy quá {} phút, đã dừng.", seat.cli, TIMEOUT_SECS / 60))),
    };
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    match seat.cli.as_str() {
        "claude" => parse_claude(&stdout, &stderr, output.status.success()),
        _ => {
            let mut text = last_msg
                .as_ref()
                .and_then(|f| std::fs::read_to_string(f).ok())
                .unwrap_or_default();
            if let Some(f) = &last_msg {
                let _ = std::fs::remove_file(f);
            }
            if text.trim().is_empty() {
                text = stdout.clone();
            }
            if !output.status.success() || text.trim().is_empty() {
                let msg = tail(&format!("{stderr}\n{stdout}"), 600);
                return Err(err(if msg.trim().is_empty() { format!("{} không trả về gì.", seat.cli) } else { msg }));
            }
            let (lim, _) = detect_limit(&text);
            if lim && text.len() < 400 {
                return Err(err(text));
            }
            let tokens = ((prompt.chars().count() + text.chars().count()) / 4) as u64;
            Ok(CliOut { text: text.trim().to_string(), tokens, cost: 0.0 })
        }
    }
}

fn parse_claude(stdout: &str, stderr: &str, success: bool) -> Result<CliOut, CliErr> {
    let v: Option<serde_json::Value> = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok().filter(|v| v.is_object()))
        .or_else(|| serde_json::from_str(stdout.trim()).ok());
    let Some(v) = v else {
        let msg = tail(&format!("{stderr}\n{stdout}"), 600);
        return Err(err(if msg.trim().is_empty() { "claude không trả về gì.".to_string() } else { msg }));
    };
    let text = v["result"].as_str().unwrap_or_default().to_string();
    if v["is_error"].as_bool().unwrap_or(false) || !success {
        let msg = if text.is_empty() { tail(&format!("{stderr}\n{stdout}"), 600) } else { text };
        return Err(err(msg));
    }
    let u = &v["usage"];
    let tokens = ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
        .iter()
        .map(|k| u[*k].as_u64().unwrap_or(0))
        .sum();
    let cost = v["total_cost_usd"].as_f64().unwrap_or(0.0);
    Ok(CliOut { text: text.trim().to_string(), tokens, cost })
}

fn tail(s: &str, n: usize) -> String {
    let s = s.trim();
    let count = s.chars().count();
    if count <= n { s.to_string() } else { format!("…{}", s.chars().skip(count - n).collect::<String>()) }
}

/// Nhận diện thông báo hết hạn mức / bị giới hạn tốc độ của 3 CLI.
pub fn detect_limit(msg: &str) -> (bool, String) {
    let low = msg.to_lowercase();
    const KEYS: [&str; 12] = [
        "rate limit", "rate_limit", "usage limit", "limit reached", "limit will reset", "quota",
        "429", "too many requests", "resource_exhausted", "reached your", "hit your limit", "out of credits",
    ];
    if !KEYS.iter().any(|k| low.contains(k)) {
        return (false, String::new());
    }
    let hint = msg
        .lines()
        .find(|l| l.to_lowercase().contains("reset") || l.to_lowercase().contains("try again"))
        .map(|l| l.trim().chars().take(90).collect())
        .unwrap_or_default();
    (true, hint)
}
