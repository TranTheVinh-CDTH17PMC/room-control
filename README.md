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

## Video Editor (CapCut)

Phòng có thêm một AI **Video Editor** chuyên dựng video. CapCut không có API, nên Editor **tạo project nháp (draft) của CapCut** trên máy bạn. Bạn mở CapCut lên là thấy project đã dựng sẵn timeline, chỉnh thêm nếu muốn rồi bấm Export.

1. Bỏ video, ảnh, nhạc vào thư mục đầu vào: bấm **Bỏ video/ảnh/nhạc vào đây** ở cột trái (`workspace/video/input`).
2. Nhắn cho Leader, ví dụ: *"Dựng vlog 9:16 dài 30 giây từ mấy clip Đà Lạt, chèn chữ mở đầu, nhạc nền nhẹ"*. Leader giao việc cho Editor. Bạn cũng có thể nhắn riêng cho Editor.
3. Editor xem file, viết kế hoạch dựng rồi chạy `tools/capcut_draft.py` để tạo project.
4. Mở CapCut (nếu đang mở thì thoát ra vào lại màn hình chính) để thấy project mới.

Editor làm được: cắt và nối clip, ảnh, tốc độ, chuyển cảnh, hiệu ứng vào/ra, chữ, nhạc nền (fade), phụ đề `.srt`. Nếu bạn muốn file video thành phẩm ngay, Editor có thể xuất thêm bằng ffmpeg vào `workspace/video/output`.

Cần thêm: Python, `pip install pycapcut` và ffmpeg (`cai-cli.bat` cài sẵn cả ba). App tự tìm thư mục draft của CapCut bản Windows (`%LOCALAPPDATA%\CapCut\User Data\Projects\com.lveditor.draft`). Nếu bạn đã đổi chỗ lưu draft trong cài đặt CapCut, đặt biến `CAPCUT_DRAFTS` trỏ tới thư mục đó.

**Lưu ý:**
- Editor là AI duy nhất được bật **Công cụ**: được chạy lệnh và ghi file (qua `claude -p --allowedTools`) trong `workspace` và thư mục draft CapCut. Chỉ giao việc dựng video cho Editor. Đừng bật Công cụ cho các vị trí khác nếu không cần.
- Draft do Editor tạo luôn là project mới. Editor không mở được project có sẵn trong CapCut bản mới, vì CapCut đã mã hoá file project.
- Thư viện tạo draft (`pycapcut`) là thư viện cộng đồng, không phải của CapCut. Nếu CapCut cập nhật mà project mở bị lỗi, hãy cập nhật bằng `pip install -U pycapcut`.

## Cấu hình

| Biến môi trường | Ý nghĩa | Mặc định |
|---|---|---|
| `PORT` | Cổng web | `3000` |
| `MAX_ROUNDS` | Số vòng điều phối tối đa cho mỗi yêu cầu | `8` |
| `NO_BROWSER` | Đặt bất kỳ giá trị nào để không tự mở trình duyệt | — |
| `CAPCUT_DRAFTS` | Thư mục draft của CapCut | tự tìm |
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
src/prompts.rs     prompt cho Leader, thành viên, tin nhắn riêng; môi trường video
tools/capcut_draft.py  tạo project CapCut từ kế hoạch dựng JSON
static/index.html  văn phòng 3D (Three.js) + chat + cài đặt
static/vendor/     three.js (MIT) nhúng sẵn, chạy được không cần mạng
```
