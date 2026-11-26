//! Room Control AI: văn phòng 3D, nơi Leader (Claude) điều phối cả phòng Dev.
//! Mỗi thành viên chạy bằng CLI chính chủ đã đăng nhập trên máy:
//!   claude -p   (gói Claude)   ·   codex exec   (gói ChatGPT)   ·   gemini   (tài khoản Google)
//! Không cần API key, không đụng tới token đăng nhập.

mod cli;
mod prompts;

use axum::{
    extract::State,
    http::header,
    response::{
        sse::{Event, KeepAlive, Sse},
        Html,
    },
    routing::{get, post},
    Json, Router,
};
use futures::{stream::FuturesUnordered, Stream, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    convert::Infallible,
    env,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;

// ------------------------------------------------------------ static files

const INDEX_HTML: &str = include_str!("../static/index.html");
const THREE_JS: &str = include_str!("../static/vendor/three.module.js");
const ORBIT_JS: &str = include_str!("../static/vendor/OrbitControls.js");
const CSS2D_JS: &str = include_str!("../static/vendor/CSS2DRenderer.js");

// ------------------------------------------------------------ data

#[derive(Clone, Serialize, Deserialize)]
pub struct Seat {
    pub id: String,
    pub name: String,
    pub short: String,
    /// leader | pm | tester | senior | dev
    pub role: String,
    pub title: String,
    pub job: String,
    /// claude | codex | gemini
    pub cli: String,
    /// để trống = model mặc định của CLI
    #[serde(default)]
    pub model: String,
    pub color: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// cho phép chạy công cụ (lệnh, ghi file) — dùng cho Video Editor
    #[serde(default)]
    pub tools: bool,
}
fn yes() -> bool {
    true
}

fn default_seats() -> Vec<Seat> {
    let s = |id: &str, name: &str, short: &str, role: &str, title: &str, job: &str, cli: &str, model: &str, color: &str| Seat {
        id: id.into(), name: name.into(), short: short.into(), role: role.into(), title: title.into(),
        job: job.into(), cli: cli.into(), model: model.into(), color: color.into(), enabled: true, tools: false,
    };
    let mut v = vec![
        s("leader", "Leader", "LD", "leader", "Trưởng phòng", "Nhận yêu cầu, chia việc, chốt kết quả", "claude", "claude-opus-5-5", "#e8590c"),
        s("pm", "PM", "PM", "pm", "Product Manager", "Viết user story, tiêu chí nghiệm thu, làm rõ yêu cầu", "claude", "claude-opus-4-8", "#7048e8"),
        s("tester", "Tester", "QA", "tester", "QA Tester", "Viết test case, tìm bug, xác nhận bản sửa", "claude", "claude-opus-4-8", "#d6336c"),
        s("senior", "Senior", "SR", "senior", "Senior Dev", "Thiết kế kỹ thuật, review code, chỉ ra rủi ro", "claude", "claude-sonnet-5-5", "#1c7ed6"),
        s("dev1", "Dev Sonnet", "D1", "dev", "Developer", "Viết code theo task", "claude", "claude-sonnet-5-5", "#0c8599"),
        s("dev2", "Dev GPT", "D2", "dev", "Developer", "Viết code theo task", "codex", "", "#2b8a3e"),
        s("dev3", "Dev Gemini", "D3", "dev", "Developer", "Viết code theo task", "gemini", "", "#3b5bdb"),
    ];
    v.push(editor_seat());
    v
}

pub fn editor_seat() -> Seat {
    Seat {
        id: "editor".into(), name: "Video Editor".into(), short: "VE".into(), role: "editor".into(),
        title: "Video Editor".into(),
        job: "Dựng video: cắt ghép, chữ, nhạc, phụ đề, chuyển cảnh; tạo project CapCut".into(),
        cli: "claude".into(), model: "claude-sonnet-5-5".into(), color: "#f76707".into(), enabled: true, tools: true,
    }
}

#[derive(Clone, Default, Serialize)]
struct Runtime {
    state: String,
    task: String,
    tokens: u64,
    cost: f64,
    sleep_text: String,
    #[serde(skip)]
    sleep_until: Option<Instant>,
}

#[derive(Clone)]
struct Entry {
    who: String,
    text: String,
}

pub struct AppState {
    seats: RwLock<Vec<Seat>>,
    rt: Mutex<HashMap<String, Runtime>>,
    clis: RwLock<HashMap<String, cli::CliInfo>>,
    rooms: Mutex<HashMap<String, Vec<Entry>>>,
    answers: Mutex<HashMap<String, oneshot::Sender<String>>>,
    seats_path: PathBuf,
    pub workdir: PathBuf,
    video: prompts::VideoEnv,
    max_rounds: usize,
    sleep_minutes: u64,
}
type Shared = Arc<AppState>;
type Tx = mpsc::Sender<Value>;

// ------------------------------------------------------------ main

#[tokio::main]
async fn main() {
    let base = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let seats_path = base.join("seats.json");
    let workdir = base.join("workspace");
    let _ = std::fs::create_dir_all(&workdir);

    let seats: Vec<Seat> = std::fs::read_to_string(&seats_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            let d = default_seats();
            let _ = std::fs::write(&seats_path, serde_json::to_string_pretty(&d).unwrap());
            d
        });

    // thêm Video Editor cho seats.json cũ
    let mut seats = seats;
    if !seats.iter().any(|s| s.role == "editor") {
        seats.push(editor_seat());
        let _ = std::fs::write(&seats_path, serde_json::to_string_pretty(&seats).unwrap());
    }
    let video = prompts::VideoEnv::detect(&base, &workdir);

    let port: u16 = env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let max_rounds = env::var("MAX_ROUNDS").ok().and_then(|p| p.parse().ok()).unwrap_or(8);

    println!("\n  ✦ Room Control AI · Phòng Dev");
    println!("  ───────────────────────────────────────────────");
    let clis = cli::detect_all().await;
    for name in ["claude", "codex", "gemini"] {
        match &clis[name].version {
            Some(v) => println!("  {name:<8} ✓ {v}"),
            None => println!("  {name:<8} ✗ chưa cài (hoặc không có trong PATH)"),
        }
    }
    println!("  video    đầu vào: {}", video.input.display());
    if video.drafts_found {
        println!("  capcut   ✓ {}", video.drafts.display());
    } else {
        println!("  capcut   ✗ chưa thấy thư mục draft (mở CapCut 1 lần, hoặc đặt CAPCUT_DRAFTS)");
    }

    let mut rt = HashMap::new();
    for s in &seats {
        rt.insert(s.id.clone(), Runtime { state: "idle".into(), task: "Chưa có việc".into(), ..Default::default() });
    }

    let state = Arc::new(AppState {
        seats: RwLock::new(seats),
        rt: Mutex::new(rt),
        clis: RwLock::new(clis),
        rooms: Mutex::new(HashMap::new()),
        answers: Mutex::new(HashMap::new()),
        seats_path,
        workdir,
        video,
        max_rounds,
        sleep_minutes: 30,
    });

    let js = |body: &'static str| async move { ([(header::CONTENT_TYPE, "application/javascript; charset=utf-8")], body) };
    let app = Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route("/vendor/three.module.js", get(move || js(THREE_JS)))
        .route("/vendor/OrbitControls.js", get(move || js(ORBIT_JS)))
        .route("/vendor/CSS2DRenderer.js", get(move || js(CSS2D_JS)))
        .route("/api/seats", get(get_seats).post(save_seats))
        .route("/api/chat", post(chat))
        .route("/api/answer", post(answer))
        .route("/api/check", post(check))
        .route("/api/check/{id}", post(check_one))
        .route("/api/wake", post(wake))
        .route("/api/reset", post(reset))
        .route("/api/open-folder", post(open_folder))
        .with_state(state);

    let url = format!("http://localhost:{port}");
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("  Không mở được cổng {port}: {e}. Đặt biến PORT khác rồi chạy lại.");
            std::process::exit(1);
        }
    };
    println!("  ───────────────────────────────────────────────");
    println!("  Mở trình duyệt: {url}\n");
    if env::var("NO_BROWSER").is_err() {
        open_browser(&url);
    }
    axum::serve(listener, app).await.unwrap();
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

// ------------------------------------------------------------ helpers

async fn emit(tx: &Tx, v: Value) {
    let _ = tx.send(v).await;
}

async fn set_state(st: &AppState, tx: &Tx, id: &str, state: &str, task: &str) {
    {
        let mut rt = st.rt.lock().await;
        if let Some(r) = rt.get_mut(id) {
            r.state = state.into();
            r.task = task.into();
        }
    }
    emit(tx, json!({"type": "status", "member": id, "state": state, "task": task})).await;
}

/// Tự đánh thức những ai đã ngủ đủ giờ
async fn wake_expired(st: &AppState) {
    let mut rt = st.rt.lock().await;
    for r in rt.values_mut() {
        if r.state == "sleeping" && r.sleep_until.map(|u| Instant::now() >= u).unwrap_or(false) {
            r.state = "idle".into();
            r.task = "Chưa có việc".into();
            r.sleep_until = None;
            r.sleep_text.clear();
        }
    }
}

async fn put_to_sleep(st: &AppState, tx: &Tx, seat: &Seat, hint: &str) {
    let mins = st.sleep_minutes;
    let text = if hint.is_empty() {
        format!("Hết hạn mức · thử lại sau {mins} phút")
    } else {
        format!("Hết hạn mức · {hint}")
    };
    {
        let mut rt = st.rt.lock().await;
        if let Some(r) = rt.get_mut(&seat.id) {
            r.state = "sleeping".into();
            r.task = text.clone();
            r.sleep_text = text.clone();
            r.sleep_until = Some(Instant::now() + Duration::from_secs(mins * 60));
        }
    }
    emit(tx, json!({"type": "sleep", "member": seat.id, "text": text})).await;
    emit(tx, json!({"type": "sys", "text": format!("{} {}. Đi ngủ.", seat.name, text.to_lowercase())})).await;
}

/// Chạy 1 thành viên; tự cập nhật token, chi phí, ngủ khi hết hạn mức.
async fn run_seat(st: &AppState, tx: &Tx, seat: &Seat, prompt: &str) -> Result<String, String> {
    let started = Instant::now();
    match cli::run(&st.workdir, seat, prompt, &st.video.dirs_for(seat)).await {
        Ok(out) => {
            {
                let mut rt = st.rt.lock().await;
                if let Some(r) = rt.get_mut(&seat.id) {
                    r.tokens += out.tokens;
                    r.cost += out.cost;
                }
            }
            emit(tx, json!({"type": "usage", "member": seat.id, "tokens": out.tokens, "cost": out.cost, "ms": started.elapsed().as_millis() as u64})).await;
            Ok(out.text)
        }
        Err(e) => {
            if e.rate_limited {
                put_to_sleep(st, tx, seat, &e.reset_hint).await;
            }
            Err(e.message)
        }
    }
}

// ------------------------------------------------------------ handlers: seats

async fn seats_view(st: &AppState) -> Value {
    wake_expired(st).await;
    let seats = st.seats.read().await.clone();
    let rt = st.rt.lock().await.clone();
    let clis = st.clis.read().await.clone();
    let list: Vec<Value> = seats
        .iter()
        .map(|s| {
            let r = rt.get(&s.id).cloned().unwrap_or_default();
            let installed = clis.get(&s.cli).map(|c| c.version.is_some()).unwrap_or(false);
            json!({
                "seat": s, "state": if !installed { "offline".to_string() } else { r.state }, "task": r.task,
                "tokens": r.tokens, "cost": r.cost, "sleep_text": r.sleep_text, "installed": installed,
            })
        })
        .collect();
    json!({ "seats": list, "clis": clis, "gemini_key": cli::precheck("gemini").is_none(), "workdir": st.workdir.display().to_string(), "video": st.video.view() })
}

async fn get_seats(State(st): State<Shared>) -> Json<Value> {
    Json(seats_view(&st).await)
}

async fn save_seats(State(st): State<Shared>, Json(new): Json<Vec<Seat>>) -> Json<Value> {
    if new.iter().filter(|s| s.role == "leader").count() != 1 {
        return Json(json!({"ok": false, "error": "Phòng cần đúng 1 Leader."}));
    }
    if let Err(e) = std::fs::write(&st.seats_path, serde_json::to_string_pretty(&new).unwrap()) {
        return Json(json!({"ok": false, "error": format!("Không ghi được seats.json: {e}")}));
    }
    {
        let mut rt = st.rt.lock().await;
        for s in &new {
            rt.entry(s.id.clone()).or_insert(Runtime { state: "idle".into(), task: "Chưa có việc".into(), ..Default::default() });
        }
    }
    *st.seats.write().await = new;
    *st.clis.write().await = cli::detect_all().await;
    Json(json!({"ok": true}))
}

async fn check(State(st): State<Shared>) -> Json<Value> {
    *st.clis.write().await = cli::detect_all().await;
    let seats = st.seats.read().await.clone();
    let futs = seats.iter().map(|s| {
        let st = st.clone();
        let s = s.clone();
        async move {
            let started = Instant::now();
            let r = cli::run(&st.workdir, &s, "Đây là bài kiểm tra kết nối. Chỉ trả lời đúng một từ: OK", &[]).await;
            let ms = started.elapsed().as_millis() as u64;
            match r {
                Ok(o) => json!({"id": s.id, "ok": true, "text": o.text.chars().take(80).collect::<String>(), "ms": ms}),
                Err(e) => json!({"id": s.id, "ok": false, "text": e.message, "rate_limited": e.rate_limited, "ms": ms}),
            }
        }
    });
    let results = futures::future::join_all(futs).await;
    Json(json!({"results": results}))
}

/// Kiểm tra 1 vị trí, tối đa 60 giây.
async fn check_one(State(st): State<Shared>, axum::extract::Path(id): axum::extract::Path<String>) -> Json<Value> {
    let Some(s) = st.seats.read().await.iter().find(|s| s.id == id).cloned() else {
        return Json(json!({"id": id, "ok": false, "text": "Không có vị trí này"}));
    };
    if let Some(msg) = cli::precheck(&s.cli) {
        return Json(json!({"id": id, "ok": false, "text": msg, "ms": 0}));
    }
    let started = Instant::now();
    let fut = cli::run(&st.workdir, &s, "Đây là bài kiểm tra kết nối. Chỉ trả lời đúng một từ: OK", &[]);
    let r = tokio::time::timeout(Duration::from_secs(60), fut).await;
    let ms = started.elapsed().as_millis() as u64;
    Json(match r {
        Ok(Ok(o)) => json!({"id": id, "ok": true, "text": o.text.chars().take(80).collect::<String>(), "ms": ms}),
        Ok(Err(e)) => json!({"id": id, "ok": false, "text": e.message, "ms": ms}),
        Err(_) => json!({"id": id, "ok": false, "ms": ms, "text": format!("{} không trả lời sau 60 giây (có thể đang chờ đăng nhập). Chạy `{}` trong terminal để xem nó hỏi gì.", s.cli, s.cli)}),
    })
}

#[derive(Deserialize)]
struct WakeReq {
    id: String,
}
async fn wake(State(st): State<Shared>, Json(req): Json<WakeReq>) -> Json<Value> {
    let mut rt = st.rt.lock().await;
    if let Some(r) = rt.get_mut(&req.id) {
        r.state = "idle".into();
        r.task = "Chưa có việc".into();
        r.sleep_until = None;
        r.sleep_text.clear();
    }
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct RoomReq {
    room_id: String,
}
async fn reset(State(st): State<Shared>, Json(req): Json<RoomReq>) -> Json<Value> {
    st.rooms.lock().await.remove(&req.room_id);
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct FolderReq {
    which: String,
}
async fn open_folder(State(st): State<Shared>, Json(req): Json<FolderReq>) -> Json<Value> {
    let dir = match req.which.as_str() {
        "input" => st.video.input.clone(),
        "output" => st.video.output.clone(),
        "drafts" => st.video.drafts.clone(),
        _ => st.workdir.clone(),
    };
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(&dir).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&dir).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&dir).spawn();
    Json(json!({"ok": r.is_ok(), "path": dir.display().to_string()}))
}

#[derive(Deserialize)]
struct AnswerReq {
    room_id: String,
    answer: String,
}
async fn answer(State(st): State<Shared>, Json(req): Json<AnswerReq>) -> Json<Value> {
    let ok = match st.answers.lock().await.remove(&req.room_id) {
        Some(tx) => tx.send(req.answer).is_ok(),
        None => false,
    };
    Json(json!({"ok": ok}))
}

// ------------------------------------------------------------ handlers: chat

#[derive(Deserialize)]
struct ChatReq {
    room_id: String,
    message: String,
    /// id thành viên nếu nhắn riêng; trống = giao cho Leader (cả phòng)
    #[serde(default)]
    to: Option<String>,
}

async fn chat(State(st): State<Shared>, Json(req): Json<ChatReq>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<Value>(128);
    tokio::spawn(async move {
        wake_expired(&st).await;
        match req.to.filter(|t| !t.is_empty()) {
            Some(to) => run_direct(&st, &req.room_id, &to, &req.message, &tx).await,
            None => run_leader(&st, &req.room_id, &req.message, &tx).await,
        }
        emit(&tx, json!({"type": "done"})).await;
    });
    let stream = ReceiverStream::new(rx).map(|v| Ok(Event::default().data(v.to_string())));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn seat_status(st: &AppState, id: &str) -> String {
    st.rt.lock().await.get(id).map(|r| r.state.clone()).unwrap_or_default()
}

async fn run_direct(st: &Shared, room_id: &str, to: &str, message: &str, tx: &Tx) {
    let Some(seat) = st.seats.read().await.iter().find(|s| s.id == to).cloned() else {
        emit(tx, json!({"type": "error", "text": "Không tìm thấy thành viên này."})).await;
        return;
    };
    if seat_status(st, to).await == "sleeping" {
        emit(tx, json!({"type": "sys", "text": format!("{} đang ngủ vì hết hạn mức. Bấm vào robot → Đánh thức nếu muốn thử lại.", seat.name)})).await;
        return;
    }
    let history = st.rooms.lock().await.get(room_id).cloned().unwrap_or_default();
    let prompt = prompts::direct(&seat, message, &render_history(&history, 12), &st.video);
    set_state(st, tx, to, "working", "Trả lời tin nhắn riêng").await;
    let res = run_seat(st, tx, &seat, prompt.as_str()).await;
    match res {
        Ok(text) => {
            emit(tx, json!({"type": "say", "member": to, "text": text, "dm": true})).await;
            let mut rooms = st.rooms.lock().await;
            let h = rooms.entry(room_id.to_string()).or_default();
            h.push(Entry { who: "user".into(), text: format!("(nhắn riêng {}) {message}", seat.name) });
            h.push(Entry { who: seat.id.clone(), text });
        }
        Err(e) => emit(tx, json!({"type": "error", "text": format!("{}: {e}", seat.name)})).await,
    }
    if seat_status(st, to).await != "sleeping" {
        set_state(st, tx, to, "idle", "Chưa có việc").await;
    }
}

fn render_history(h: &[Entry], last: usize) -> String {
    let start = h.len().saturating_sub(last);
    h[start..]
        .iter()
        .map(|e| {
            let text: String = e.text.chars().take(4000).collect();
            format!("[{}]\n{}\n", e.who, text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Deserialize, Default)]
struct Plan {
    #[serde(default)]
    say: String,
    #[serde(default)]
    assign: Vec<Assign>,
    #[serde(default)]
    meeting: Vec<String>,
    #[serde(default)]
    ask_user: Option<Ask>,
    #[serde(default)]
    final_answer: Option<String>,
    #[serde(default, rename = "final")]
    final_: Option<String>,
}
#[derive(Deserialize, Clone)]
struct Assign {
    to: String,
    task: String,
}
#[derive(Deserialize)]
struct Ask {
    question: String,
    #[serde(default)]
    options: Vec<String>,
}

fn parse_plan(text: &str) -> Option<Plan> {
    let s = text.find('{')?;
    let e = text.rfind('}')?;
    if e <= s {
        return None;
    }
    serde_json::from_str(&text[s..=e]).ok()
}

async fn run_leader(st: &Shared, room_id: &str, message: &str, tx: &Tx) {
    let seats = st.seats.read().await.clone();
    let Some(leader) = seats.iter().find(|s| s.role == "leader").cloned() else {
        emit(tx, json!({"type": "error", "text": "Phòng chưa có Leader."})).await;
        return;
    };
    if seat_status(st, &leader.id).await == "sleeping" {
        emit(tx, json!({"type": "sys", "text": "Leader đang ngủ vì hết hạn mức, chưa nhận việc được. Bạn có thể nhắn riêng từng thành viên."})).await;
        return;
    }

    let mut history = st.rooms.lock().await.get(room_id).cloned().unwrap_or_default();
    history.push(Entry { who: "Người dùng".into(), text: message.into() });

    for round in 1..=st.max_rounds {
        set_state(st, tx, &leader.id, "working", &format!("Điều phối · vòng {round}")).await;

        let roster = roster(st, &seats).await;
        let prompt = prompts::leader(&leader, &roster, &render_history(&history, 30), round, st.max_rounds);
        let out = match run_seat(st, tx, &leader, &prompt).await {
            Ok(t) => t,
            Err(e) => {
                if seat_status(st, &leader.id).await != "sleeping" {
                    set_state(st, tx, &leader.id, "idle", "Chưa có việc").await;
                }
                emit(tx, json!({"type": "error", "text": format!("Leader lỗi: {e}")})).await;
                break;
            }
        };

        let plan = parse_plan(&out).unwrap_or_else(|| Plan { final_: Some(out.clone()), ..Default::default() });
        if !plan.say.trim().is_empty() {
            emit(tx, json!({"type": "say", "member": leader.id, "text": plan.say})).await;
            history.push(Entry { who: format!("{} (Leader)", leader.name), text: plan.say.clone() });
        }

        if let Some(fin) = plan.final_.clone().or(plan.final_answer.clone()).filter(|f| !f.trim().is_empty()) {
            emit(tx, json!({"type": "final", "member": leader.id, "text": fin})).await;
            history.push(Entry { who: format!("{} (Leader) - chốt", leader.name), text: fin });
            set_state(st, tx, &leader.id, "idle", "Chưa có việc").await;
            st.rooms.lock().await.insert(room_id.to_string(), history);
            return;
        }

        if let Some(ask) = plan.ask_user {
            set_state(st, tx, &leader.id, "waiting", "Chờ bạn trả lời").await;
            let (atx, arx) = oneshot::channel();
            st.answers.lock().await.insert(room_id.to_string(), atx);
            emit(tx, json!({"type": "ask", "member": leader.id, "question": ask.question, "options": ask.options})).await;
            let ans = match tokio::time::timeout(Duration::from_secs(600), arx).await {
                Ok(Ok(a)) => a,
                _ => {
                    st.answers.lock().await.remove(room_id);
                    emit(tx, json!({"type": "sys", "text": "Chờ 10 phút không thấy bạn trả lời, Leader tự quyết."})).await;
                    "Người dùng không trả lời, hãy tự chọn phương án hợp lý nhất.".into()
                }
            };
            history.push(Entry { who: "Người dùng (trả lời câu hỏi)".into(), text: format!("Hỏi: {}\nĐáp: {ans}", ask.question) });
            continue;
        }

        if plan.assign.is_empty() {
            // Không giao gì, không chốt: coi lời nói là câu trả lời cuối
            let fin = if plan.say.trim().is_empty() { out } else { plan.say.clone() };
            emit(tx, json!({"type": "final", "member": leader.id, "text": fin})).await;
            set_state(st, tx, &leader.id, "idle", "Chưa có việc").await;
            st.rooms.lock().await.insert(room_id.to_string(), history);
            return;
        }

        // ---- giao việc
        let meeting: Vec<String> = plan.meeting.clone();
        let in_meeting = !meeting.is_empty();
        if in_meeting {
            emit(tx, json!({"type": "meeting", "members": meeting.iter().chain(std::iter::once(&leader.id)).collect::<Vec<_>>()})).await;
            set_state(st, tx, &leader.id, "meeting", "Chủ trì họp").await;
        } else {
            set_state(st, tx, &leader.id, "idle", "Chờ team làm việc").await;
        }

        let mut jobs = vec![];
        for a in &plan.assign {
            let Some(seat) = seats.iter().find(|s| s.id == a.to).cloned() else {
                history.push(Entry { who: "Hệ thống".into(), text: format!("Không có thành viên id '{}'.", a.to) });
                continue;
            };
            let status = seat_status(st, &seat.id).await;
            if status == "sleeping" || !seat.enabled {
                history.push(Entry { who: "Hệ thống".into(), text: format!("{} đang ngủ/tắt, không nhận việc: {}", seat.name, a.task) });
                emit(tx, json!({"type": "sys", "text": format!("{} đang nghỉ, việc \"{}\" chưa giao được.", seat.name, short(&a.task, 60))})).await;
                continue;
            }
            emit(tx, json!({"type": "handoff", "from": leader.id, "to": seat.id, "task": a.task})).await;
            let state = if meeting.contains(&seat.id) { "meeting" } else { "working" };
            set_state(st, tx, &seat.id, state, &short(&a.task, 80)).await;
            jobs.push((seat, a.task.clone()));
        }

        let mut pending: FuturesUnordered<_> = jobs
            .into_iter()
            .map(|(seat, task)| {
                let st = st.clone();
                let tx = tx.clone();
                let ctx = render_history(&history, 6);
                async move {
                    let started = Instant::now();
                    let prompt = prompts::worker(&seat, &task, &ctx, &st.video);
                    let res = run_seat(&st, &tx, &seat, &prompt).await;
                    (seat, task, res, started.elapsed().as_millis() as u64)
                }
            })
            .collect();

        while let Some((seat, task, res, ms)) = pending.next().await {
            let (ok, text) = match res {
                Ok(t) => (true, t),
                Err(e) => (false, e),
            };
            emit(tx, json!({"type": "result", "member": seat.id, "ok": ok, "text": text, "ms": ms, "task": task})).await;
            history.push(Entry {
                who: format!("{} ({}) - kết quả việc: {}", seat.name, seat.title, short(&task, 80)),
                text: if ok { text } else { format!("LỖI: {text}") },
            });
            if seat_status(st, &seat.id).await != "sleeping" {
                set_state(st, tx, &seat.id, "idle", "Chưa có việc").await;
            }
        }
        if in_meeting {
            emit(tx, json!({"type": "meeting_end"})).await;
        }
    }

    set_state(st, tx, &leader.id, "idle", "Chưa có việc").await;
    emit(tx, json!({"type": "error", "text": format!("Leader đã dùng hết {} vòng mà chưa chốt. Tăng MAX_ROUNDS nếu cần.", st.max_rounds)})).await;
    st.rooms.lock().await.insert(room_id.to_string(), history);
}

fn short(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

async fn roster(st: &AppState, seats: &[Seat]) -> String {
    let rt = st.rt.lock().await.clone();
    let clis = st.clis.read().await.clone();
    seats
        .iter()
        .filter(|s| s.role != "leader")
        .map(|s| {
            let r = rt.get(&s.id).cloned().unwrap_or_default();
            let installed = clis.get(&s.cli).map(|c| c.version.is_some()).unwrap_or(false);
            let status = if !s.enabled {
                "TẮT".to_string()
            } else if !installed {
                format!("KHÔNG DÙNG ĐƯỢC (chưa cài {})", s.cli)
            } else if r.state == "sleeping" {
                format!("ĐANG NGỦ ({})", r.sleep_text)
            } else {
                "sẵn sàng".to_string()
            };
            let model = if s.model.is_empty() { format!("{} mặc định", s.cli) } else { s.model.clone() };
            format!("- id: {} | {} | {} | việc: {} | model: {} | {}", s.id, s.name, s.title, s.job, model, status)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
