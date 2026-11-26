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

/// Bản CLI đã chọn (bản mới nhất tìm thấy trên máy), điền bởi detect_all.
static CHOSEN: std::sync::RwLock<Option<HashMap<String, PathBuf>>> = std::sync::RwLock::new(None);

/// Tìm file thực thi (trên Windows: claude.cmd / claude.exe ...).
/// Có thể ghi đè bằng biến môi trường CLAUDE_BIN / CODEX_BIN / GEMINI_BIN.
pub fn resolve(name: &str) -> Option<PathBuf> {
    let var = format!("{}_BIN", name.to_uppercase());
    if let Ok(p) = std::env::var(&var) {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Some(p) = CHOSEN.read().ok().and_then(|m| m.as_ref().and_then(|m| m.get(name).cloned())) {
        return Some(p);
    }
    which::which(name).ok()
}

/// Mọi bản của một CLI trên máy: trong PATH và các chỗ cài phổ biến.
fn candidates(name: &str) -> Vec<PathBuf> {
    let var = format!("{}_BIN", name.to_uppercase());
    if let Ok(p) = std::env::var(&var) {
        if !p.trim().is_empty() {
            return vec![PathBuf::from(p)];
        }
    }
    let mut v: Vec<PathBuf> = which::which_all(name).map(|it| it.collect()).unwrap_or_default();
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).map(PathBuf::from).ok();
    let mut extra = vec![];
    if let Some(h) = &home {
        extra.push(h.join(".local").join("bin").join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }));
    }
    if cfg!(windows) {
        if let Ok(a) = std::env::var("APPDATA") {
            extra.push(PathBuf::from(a).join("npm").join(format!("{name}.cmd")));
        }
    }
    for e in extra {
        if e.is_file() {
            v.push(e);
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.to_string_lossy().to_lowercase()));
    v
}

fn parse_version(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit() && c != '.')
        .find(|t| t.contains('.'))
        .map(|t| t.split('.').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_default()
}

async fn version_of(p: &Path) -> Option<String> {
    let mut cmd = command(p);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let o = tokio::time::timeout(Duration::from_secs(20), cmd.output()).await.ok()?.ok()?;
    if !o.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    let s = if s.is_empty() { String::from_utf8_lossy(&o.stderr).trim().to_string() } else { s };
    Some(s.lines().next().unwrap_or("").chars().take(60).collect())
}

pub async fn detect_all() -> HashMap<String, CliInfo> {
    let mut out = HashMap::new();
    let mut chosen = HashMap::new();
    for name in ["claude", "codex", "gemini"] {
        let mut best: Option<(Vec<u64>, PathBuf, String)> = None;
        for p in candidates(name) {
            if let Some(ver) = version_of(&p).await {
                let key = parse_version(&ver);
                if best.as_ref().map(|(k, _, _)| key > *k).unwrap_or(true) {
                    best = Some((key, p, ver));
                }
            }
        }
        let mut info = CliInfo::default();
        if let Some((_, p, ver)) = best {
            info.path = Some(p.display().to_string());
            info.version = Some(ver);
            chosen.insert(name.to_string(), p);
        }
        out.insert(name.to_string(), info);
    }
    if let Ok(mut c) = CHOSEN.write() {
        *c = Some(chosen);
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

/// Lỗi biết trước mà không cần gọi CLI (trả về thông báo nếu có).
pub fn precheck(cli: &str) -> Option<String> {
    if cli == "gemini" {
        let has = |k: &str| std::env::var(k).map(|v| !v.trim().is_empty()).unwrap_or(false);
        if !(has("GEMINI_API_KEY") || has("GOOGLE_API_KEY") || has("GOOGLE_GENAI_USE_VERTEXAI")) {
            return Some("App chưa thấy GEMINI_API_KEY. Chạy setx GEMINI_API_KEY \"key\" rồi đóng hết cửa sổ và mở lại run.bat.".into());
        }
    }
    None
}

pub async fn run(workdir: &Path, seat: &Seat, prompt: &str, add_dirs: &[PathBuf]) -> Result<CliOut, CliErr> {
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
            if seat.tools {
                cmd.args(["--allowedTools", "Bash,Read,Write,Edit,Glob,Grep"]);
                for d in add_dirs {
                    cmd.arg("--add-dir").arg(d);
                }
            }
        }
        "codex" => {
            let f = std::env::temp_dir().join(format!("roomai-codex-{}-{}.txt", seat.id, std::process::id()));
            let _ = std::fs::remove_file(&f);
            let sandbox = if seat.tools { "workspace-write" } else { "read-only" };
            cmd.args(["exec", "--skip-git-repo-check", "--sandbox", sandbox]);
            if seat.tools {
                for d in add_dirs {
                    cmd.arg("--add-dir").arg(d);
                }
            }
            cmd.arg("--output-last-message");
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
            if seat.tools {
                cmd.arg("--yolo");
                if !add_dirs.is_empty() {
                    let list: Vec<String> = add_dirs.iter().map(|d| d.display().to_string()).collect();
                    cmd.arg("--include-directories").arg(list.join(","));
                }
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
    let limit = if seat.tools { TIMEOUT_SECS * 2 } else { TIMEOUT_SECS };
    let output = match tokio::time::timeout(Duration::from_secs(limit), child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(err(format!("Lỗi khi chạy {}: {e}", seat.cli))),
        Err(_) => return Err(err(format!("{} chạy quá {} phút, đã dừng.", seat.cli, limit / 60))),
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
