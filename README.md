# Room Control AI

Văn phòng 3D cho một phòng Dev toàn AI: **Leader** nhận việc từ bạn, chia cho **PM, Tester, Senior, Dev**. Mọi người làm việc, họp review, báo bug, và đi ngủ khi hết hạn mức. Bạn xem tất cả diễn ra trong văn phòng 3D.

Mỗi AI chạy bằng **CLI chính chủ đã đăng nhập trên máy bạn**, dùng gói thuê bao của bạn, **không cần API key**:

| CLI | Dùng gói | Vị trí mặc định |
|---|---|---|
| `claude` (Claude Code) | Gói Claude (Pro/Max) | Leader, PM, Tester, Senior, Dev Sonnet |
| `codex` (Codex CLI) | Gói ChatGPT | Dev GPT |
| `gemini` (Gemini CLI) | Tài khoản Google | Dev Gemini |

App chỉ gọi các CLI này ở chế độ không tương tác (`claude -p`, `codex exec`, `gemini`), không đọc hay dùng token đăng nhập.

## 1. Cài và đăng nhập CLI (mỗi máy làm một lần)

Mở PowerShell hoặc Terminal:

```bash
# Claude Code: xem cách cài mới nhất tại https://docs.claude.com/en/docs/claude-code
npm install -g @anthropic-ai/claude-code
claude            # đăng nhập bằng tài khoản Claude, xong gõ /exit

# Codex CLI
npm install -g @openai/codex
codex login       # đăng nhập bằng tài khoản ChatGPT

# Gemini CLI
npm install -g @google/gemini-cli
gemini            # chọn "Login with Google", xong thoát
```

Không cần đủ cả 3 CLI. Thiếu CLI nào thì AI dùng CLI đó đứng xám trong văn phòng, Leader sẽ không giao việc cho AI đó.

## 2. Chạy app

```bash
cargo run --release
```

Hoặc bấm đúp vào `run.bat` trên Windows. Trình duyệt sẽ tự mở http://localhost:3000.

Lần đầu chạy, app tạo file `seats.json` chứa danh sách chỗ ngồi và model, và thư mục `workspace/` là nơi các CLI làm việc.

## 3. Sử dụng

- **Giao việc cho cả phòng:** gõ vào khung chat (người nhận mặc định là Leader).
- **Nhắn riêng một AI:** bấm tên ở cột trái, hoặc bấm vào robot rồi chọn **Nhắn riêng**.
- **Leader hỏi bạn:** bấm một lựa chọn hiện trong chat, hoặc gõ câu trả lời.
- **Chỗ ngồi & model:** đổi CLI hoặc model cho từng vị trí, thêm Dev, bấm **Kiểm tra kết nối** để thử từng AI.
- **Hết hạn mức:** AI tự đi ngủ khoảng 30 phút. Bấm vào robot rồi chọn **Đánh thức** nếu muốn thử lại sớm.
- **Phím tắt:** `O` toàn cảnh · `M` phòng họp · `N` khu ngủ · `R` tự xoay · `C` thu gọn chat · `Esc` đóng.

## Cấu hình

| Biến môi trường | Ý nghĩa | Mặc định |
|---|---|---|
| `PORT` | Cổng web | `3000` |
| `MAX_ROUNDS` | Số vòng điều phối tối đa cho mỗi yêu cầu | `8` |
| `NO_BROWSER` | Đặt bất kỳ giá trị nào để không tự mở trình duyệt | — |
| `CLAUDE_BIN`, `CODEX_BIN`, `GEMINI_BIN` | Đường dẫn CLI, khi app không tự tìm thấy | tìm trong PATH |

Tên model Claude có thể ghi đầy đủ (`claude-opus-5-5`) hoặc dùng bí danh (`opus`, `sonnet`). Để trống ô model thì CLI dùng model mặc định.

## Lưu ý về hạn mức

- Mọi vị trí dùng `claude` chia chung một nguồn hạn mức chạy nền của gói Claude. Nhiều vị trí dùng Opus sẽ hết hạn mức nhanh. Nên để Opus cho Leader, các vị trí còn lại dùng Sonnet.
- Mỗi yêu cầu có thể tốn nhiều lượt gọi: Leader điều phối nhiều vòng, cộng thêm lượt của từng thành viên.
- Con số "Quy đổi giá API" chỉ để tham khảo (Claude Code tự báo con số này). Bạn đang dùng gói thuê bao nên không bị tính tiền theo số đó.

## Cấu trúc

```
src/main.rs        server axum, điều phối Leader, SSE
src/cli.rs         gọi claude / codex / gemini, nhận diện hết hạn mức
src/prompts.rs     prompt cho Leader, thành viên, tin nhắn riêng
static/index.html  văn phòng 3D (Three.js) + chat + cài đặt
static/vendor/     three.js (MIT) nhúng sẵn, chạy được không cần mạng
```
