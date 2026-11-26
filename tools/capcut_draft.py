"""
Tạo project nháp (draft) CapCut từ một file "kế hoạch dựng" JSON.
AI Video Editor trong Room Control AI dùng script này; bạn cũng có thể chạy tay.

    python tools/capcut_draft.py plan.json                # tạo draft trong thư mục draft của CapCut
    python tools/capcut_draft.py plan.json --out <dir>    # tạo vào thư mục khác (để thử)
    python tools/capcut_draft.py --list transitions|intros|outros|fonts

Cấu trúc plan.json (thời gian tính bằng giây, đường dẫn tương đối tính từ vị trí file plan):
{
  "name": "Vlog Da Lat",              // tên project trong CapCut
  "width": 1080, "height": 1920, "fps": 30,
  "clips": [                           // nối liền nhau trên track video chính
    {"file": "input/a.mp4", "from": 2, "to": 7, "speed": 1.0, "volume": 1.0,
     "transition": "叠化", "intro": null, "outro": null},
    {"file": "input/anh.jpg", "duration": 3}      // ảnh: dùng "duration"
  ],
  "texts": [
    {"text": "Ngày 1", "start": 0, "end": 3, "size": 10, "color": "#FFFFFF",
     "bold": true, "y": -0.7, "font": null}
  ],
  "music": {"file": "input/nhac.mp3", "from": 0, "volume": 0.5, "fade_in": 1, "fade_out": 2},
  "srt": "input/phude.srt"             // phụ đề (tuỳ chọn)
}
"""
import json
import os
import sys
from pathlib import Path

try:
    import pycapcut as cc
    from pycapcut import trange
except ImportError:
    sys.exit("Thiếu thư viện pycapcut. Chạy: pip install pycapcut")


def default_drafts_dir() -> Path:
    env = os.environ.get("CAPCUT_DRAFTS")
    if env:
        return Path(env)
    if sys.platform == "win32":
        return Path(os.environ.get("LOCALAPPDATA", "")) / "CapCut" / "User Data" / "Projects" / "com.lveditor.draft"
    return Path.home() / "Movies" / "CapCut" / "User Data" / "Projects" / "com.lveditor.draft"


def enum_value(enum, name, kind):
    if not name:
        return None
    if name in enum.__members__:
        return enum[name]
    low = name.lower()
    for k, v in enum.__members__.items():
        if k.lower() == low:
            return v
    print(f"[bỏ qua] Không có {kind} tên '{name}'. Xem danh sách: --list {kind}s", file=sys.stderr)
    return None


def hex_rgb(h):
    h = (h or "#FFFFFF").lstrip("#")
    return tuple(int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))


def us(sec):
    return int(round(float(sec) * 1_000_000))


def build(plan: dict, base: Path, out_dir: Path) -> Path:
    name = plan.get("name") or "Room AI draft"
    w, h, fps = int(plan.get("width", 1920)), int(plan.get("height", 1080)), int(plan.get("fps", 30))
    out_dir.mkdir(parents=True, exist_ok=True)
    folder = cc.DraftFolder(str(out_dir))
    script = folder.create_draft(name, w, h, fps=fps, allow_replace=True)
    script.add_track(cc.TrackType.video)

    def path(p):
        q = Path(p)
        q = q if q.is_absolute() else (base / q)
        if not q.exists():
            sys.exit(f"Không tìm thấy file: {q}")
        return str(q.resolve())

    # ---- video / ảnh nối tiếp nhau
    t = 0
    for i, c in enumerate(plan.get("clips", [])):
        mat = cc.VideoMaterial(path(c["file"]))
        speed = float(c.get("speed", 1.0))
        if mat.material_type == "photo":
            dur = us(c.get("duration", 3))
            seg = cc.VideoSegment(mat, cc.Timerange(t, dur))
        else:
            src_from = us(c.get("from", 0))
            src_to = us(c["to"]) if c.get("to") is not None else mat.duration
            src_to = min(src_to, mat.duration)
            if src_to <= src_from:
                sys.exit(f"Clip {i + 1}: 'to' phải lớn hơn 'from'")
            src_len = src_to - src_from
            dur = int(src_len / speed)
            seg = cc.VideoSegment(mat, cc.Timerange(t, dur), source_timerange=cc.Timerange(src_from, src_len),
                                  speed=speed, volume=float(c.get("volume", 1.0)))
        if (tr := enum_value(cc.TransitionType, c.get("transition"), "transition")):
            seg.add_transition(tr)
        if (a := enum_value(cc.IntroType, c.get("intro"), "intro")):
            seg.add_animation(a)
        if (a := enum_value(cc.OutroType, c.get("outro"), "outro")):
            seg.add_animation(a)
        script.add_segment(seg)
        t += dur
    total = t

    # ---- chữ
    texts = plan.get("texts", [])
    if texts:
        script.add_track(cc.TrackType.text, "chu")
    for x in texts:
        style = cc.TextStyle(size=float(x.get("size", 8)), bold=bool(x.get("bold", False)),
                             color=hex_rgb(x.get("color")), align=1, auto_wrapping=True)
        seg = cc.TextSegment(x["text"], cc.Timerange(us(x.get("start", 0)), us(x["end"]) - us(x.get("start", 0))),
                             font=enum_value(cc.FontType, x.get("font"), "font"), style=style,
                             clip_settings=cc.ClipSettings(transform_y=float(x.get("y", -0.75))),
                             border=cc.TextBorder(color=(0, 0, 0), width=40.0) if x.get("outline", True) else None)
        script.add_segment(seg, "chu")

    # ---- nhạc nền
    m = plan.get("music")
    if m:
        amat = cc.AudioMaterial(path(m["file"]))
        start = us(m.get("from", 0))
        length = min(amat.duration - start, total) if total else amat.duration - start
        script.add_track(cc.TrackType.audio, "nhac")
        aseg = cc.AudioSegment(amat, cc.Timerange(0, length), source_timerange=cc.Timerange(start, length),
                               volume=float(m.get("volume", 0.6)))
        if m.get("fade_in") or m.get("fade_out"):
            aseg.add_fade(us(m.get("fade_in", 0)), us(m.get("fade_out", 0)))
        script.add_segment(aseg, "nhac")

    # ---- phụ đề
    if plan.get("srt"):
        script.import_srt(path(plan["srt"]), "phude", clip_settings=cc.ClipSettings(transform_y=-0.8))

    script.save()
    return out_dir / name


def main():
    args = sys.argv[1:]
    if args[:1] == ["--list"]:
        kind = (args[1:] or ["transitions"])[0]
        enum = {"transitions": cc.TransitionType, "intros": cc.IntroType, "outros": cc.OutroType, "fonts": cc.FontType}.get(kind)
        if not enum:
            sys.exit("Dùng: --list transitions|intros|outros|fonts")
        print("\n".join(enum.__members__))
        return
    if not args:
        sys.exit(__doc__)
    plan_path = Path(args[0]).resolve()
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    out = Path(args[args.index("--out") + 1]) if "--out" in args else default_drafts_dir()
    draft = build(plan, plan_path.parent, out)
    print(json.dumps({"ok": True, "draft": str(draft), "name": plan.get("name")}, ensure_ascii=False))


if __name__ == "__main__":
    main()
