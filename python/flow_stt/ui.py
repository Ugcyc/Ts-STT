import tkinter as tk
from queue import Queue
from threading import Thread
from tkinter import messagebox, ttk
from typing import Callable, Optional, Sequence
import ctypes

# --- USER IMPORTS ---
from .audio_capture import list_input_devices
from .config import ConfigManager

# Enable High DPI on Windows
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(1)
except Exception:
    pass

# --- Modern Palette ---
PALETTE = {
    "bg": "#1e293b",
    "card": "#0f172a",
    "header": "#020617",
    "text": "#f8fafc",
    "muted": "#94a3b8",
    "border": "#334155",
    "input_bg": "#1e293b",
    "accent": "#38bdf8",
    "accent_hover": "#0ea5e9",
    "danger": "#ef4444",
    "badge_idle": "#334155",
    "badge_listen": "#0ea5e9",
    "badge_transcribe": "#8b5cf6",
}


def minimize_window(win: tk.Tk):
    """Safely minimize an overrideredirect window."""
    try:
        was_top = bool(win.attributes("-topmost"))
    except Exception:
        was_top = False
    try:
        win.overrideredirect(False)
        win.attributes("-topmost", False)
        win.update_idletasks()
        handler_id = None

        def _restore(event=None):
            try:
                win.overrideredirect(True)
                win.attributes("-topmost", was_top)
            except Exception:
                pass
            if handler_id is not None:
                try:
                    win.unbind("<Map>", handler_id)
                except Exception:
                    pass

        handler_id = win.bind("<Map>", _restore)
        win.iconify()
    except tk.TclError:
        try:
            win.withdraw()
        except Exception:
            pass


def apply_rounded_corners(window_handle, width, height, radius=20):
    """Uses Windows API to clip the window into a rounded rectangle."""
    try:
        rgn = ctypes.windll.gdi32.CreateRoundRectRgn(0, 0, width, height, radius, radius)
        ctypes.windll.user32.SetWindowRgn(window_handle, rgn, True)
    except Exception:
        pass


class ModernButton(tk.Label):
    """A custom flat button."""
    def __init__(self, parent, text, command, bg=PALETTE["card"], fg=PALETTE["text"],
                 hover_bg=PALETTE["border"], width=None, font=("Segoe UI", 9)):
        super().__init__(parent, text=text, bg=bg, fg=fg, font=font, cursor="hand2", padx=10, pady=6)
        self.command = command
        self.default_bg = bg
        self.hover_bg = hover_bg
        if width:
            self.configure(width=width)
        self.bind("<Enter>", lambda e: self.configure(bg=self.hover_bg))
        self.bind("<Leave>", lambda e: self.configure(bg=self.default_bg))
        self.bind("<Button-1>", lambda e: self.command() if self.command else None)


class ResizeGrip(tk.Label):
    """A draggable handle to resize the window."""
    def __init__(self, parent, target_window):
        super().__init__(parent, text="⋮⋮", bg=PALETTE["bg"], fg=PALETTE["muted"],
                         font=("Segoe UI", 8), cursor="size_nw_se")
        self.target = target_window
        self.bind("<ButtonPress-1>", self._start_resize)
        self.bind("<B1-Motion>", self._on_resize)
        self.bind("<ButtonRelease-1>", self._end_resize)

    def _start_resize(self, event):
        self.start_x = event.x_root
        self.start_y = event.y_root
        self.start_w = self.target.winfo_width()
        self.start_h = self.target.winfo_height()

    def _on_resize(self, event):
        dx = event.x_root - self.start_x
        dy = event.y_root - self.start_y
        new_w = max(280, self.start_w + dx)
        new_h = max(280, self.start_h + dy)
        self.target.geometry(f"{new_w}x{new_h}")

    def _end_resize(self, event):
        apply_rounded_corners(self.target.winfo_id(), self.target.winfo_width(), self.target.winfo_height())


class CompactTitleBar(tk.Frame):
    """Slim draggable title bar for the main UI."""

    def __init__(self, parent, on_close, on_settings=None, on_minimize=None):
        super().__init__(parent, bg=PALETTE["header"], height=28)
        self.parent = parent
        self.pack(fill="x", side="top")
        self.pack_propagate(False)

        self.bind("<ButtonPress-1>", self._start_move)
        self.bind("<B1-Motion>", self._on_move)

        if on_settings:
            settings_btn = tk.Label(self, text="⚙", bg=PALETTE["header"], fg=PALETTE["muted"],
                                    font=("Segoe UI", 10), cursor="hand2", width=3)
            settings_btn.pack(side="left", fill="y")
            settings_btn.bind("<Enter>", lambda e: settings_btn.configure(bg=PALETTE["border"], fg="white"))
            settings_btn.bind("<Leave>", lambda e: settings_btn.configure(bg=PALETTE["header"], fg=PALETTE["muted"]))
            settings_btn.bind("<Button-1>", lambda e: on_settings())

        close_btn = tk.Label(self, text="×", bg=PALETTE["header"], fg=PALETTE["muted"],
                             font=("Segoe UI", 11), cursor="hand2", width=3)
        close_btn.pack(side="right", fill="y")
        close_btn.bind("<Enter>", lambda e: close_btn.configure(bg=PALETTE["danger"], fg="white"))
        close_btn.bind("<Leave>", lambda e: close_btn.configure(bg=PALETTE["header"], fg=PALETTE["muted"]))
        close_btn.bind("<Button-1>", lambda e: on_close())

        if on_minimize:
            min_btn = tk.Label(self, text="–", bg=PALETTE["header"], fg=PALETTE["muted"],
                               font=("Segoe UI", 10), cursor="hand2", width=3)
            min_btn.pack(side="right", fill="y")
            min_btn.bind("<Enter>", lambda e: min_btn.configure(bg=PALETTE["border"], fg="white"))
            min_btn.bind("<Leave>", lambda e: min_btn.configure(bg=PALETTE["header"], fg=PALETTE["muted"]))
            min_btn.bind("<Button-1>", lambda e: on_minimize())

    def _start_move(self, event):
        self.parent._drag_start = (event.x, event.y)

    def _on_move(self, event):
        x = event.x_root - self.parent._drag_start[0]
        y = event.y_root - self.parent._drag_start[1]
        self.parent.geometry(f"+{x}+{y}")


class TitleBar(tk.Frame):
    """Standard title bar for settings window."""

    def __init__(self, parent, title, on_close, on_settings=None, on_minimize=None):
        super().__init__(parent, bg=PALETTE["header"], height=32)
        self.parent = parent
        self.pack(fill="x", side="top")
        self.pack_propagate(False)

        self.bind("<ButtonPress-1>", self._start_move)
        self.bind("<B1-Motion>", self._on_move)

        if on_settings:
            settings_btn = tk.Label(self, text="⚙", bg=PALETTE["header"], fg=PALETTE["muted"],
                                    font=("Segoe UI", 11), cursor="hand2", width=4)
            settings_btn.pack(side="left", fill="y")
            settings_btn.bind("<Enter>", lambda e: settings_btn.configure(bg=PALETTE["border"], fg="white"))
            settings_btn.bind("<Leave>", lambda e: settings_btn.configure(bg=PALETTE["header"], fg=PALETTE["muted"]))
            settings_btn.bind("<Button-1>", lambda e: on_settings())

        if title:
            tk.Label(self, text=title, bg=PALETTE["header"], fg=PALETTE["muted"],
                     font=("Segoe UI Semibold", 10)).pack(side="left", padx=8)

        close_btn = tk.Label(self, text="×", bg=PALETTE["header"], fg=PALETTE["muted"],
                             font=("Segoe UI", 11), cursor="hand2", width=4)
        close_btn.pack(side="right", fill="y")
        close_btn.bind("<Enter>", lambda e: close_btn.configure(bg=PALETTE["danger"], fg="white"))
        close_btn.bind("<Leave>", lambda e: close_btn.configure(bg=PALETTE["header"], fg=PALETTE["muted"]))
        close_btn.bind("<Button-1>", lambda e: on_close())

        if on_minimize:
            min_btn = tk.Label(self, text="–", bg=PALETTE["header"], fg=PALETTE["muted"],
                               font=("Segoe UI", 10), cursor="hand2", width=4)
            min_btn.pack(side="right", fill="y")
            min_btn.bind("<Enter>", lambda e: min_btn.configure(bg=PALETTE["border"], fg="white"))
            min_btn.bind("<Leave>", lambda e: min_btn.configure(bg=PALETTE["header"], fg=PALETTE["muted"]))
            min_btn.bind("<Button-1>", lambda e: on_minimize())

    def _start_move(self, event):
        self.parent._drag_start = (event.x, event.y)

    def _on_move(self, event):
        x = event.x_root - self.parent._drag_start[0]
        y = event.y_root - self.parent._drag_start[1]
        self.parent.geometry(f"+{x}+{y}")


def _apply_theme(root: tk.Misc):
    style = ttk.Style(root)
    style.theme_use("clam")
    root.option_add('*TCombobox*Listbox.background', PALETTE['card'])
    root.option_add('*TCombobox*Listbox.foreground', PALETTE['text'])
    root.option_add('*TCombobox*Listbox.selectBackground', PALETTE['accent'])
    root.option_add('*TCombobox*Listbox.selectForeground', PALETTE['header'])

    style.configure("TFrame", background=PALETTE["bg"])
    style.configure("Card.TFrame", background=PALETTE["card"])
    style.map("TCombobox", fieldbackground=[("readonly", PALETTE["card"])],
              selectbackground=[("readonly", PALETTE["card"])],
              selectforeground=[("readonly", PALETTE["text"])])
    style.configure("TCombobox",
                    background=PALETTE["bg"],
                    foreground=PALETTE["text"],
                    arrowcolor=PALETTE["text"],
                    borderwidth=0)


class StatusUI:
    def __init__(self, title: str = "Open STT", on_settings_saved: Optional[Callable[[], None]] = None):
        self.title = title
        self._thread: Optional[Thread] = None
        self._root: Optional[tk.Tk] = None
        self._status_label: Optional[tk.Label] = None
        self._status_queue: "Queue[str]" = Queue()
        self._on_settings_saved = on_settings_saved
        self._drag_start = (0, 0)
        self._canvas: Optional[tk.Canvas] = None
        self._dots: list[int] = []
        self._dot_step = 0
        self._current_status = "Idle"

    def start(self) -> None:
        if self._thread and self._thread.is_alive():
            return
        self._thread = Thread(target=self._run, daemon=True)
        self._thread.start()

    def set_status(self, text: str) -> None:
        if self._status_queue is not None:
            self._status_queue.put(text)

    def _run(self) -> None:
        self._root = tk.Tk()
        self._root.title(self.title)
        # Compact size
        w, h = 180, 100
        self._root.geometry(f"{w}x{h}")
        self._root.configure(bg=PALETTE["bg"])
        self._root.overrideredirect(True)
        self._root.wm_attributes("-topmost", True)
        self._root.config(
            bd=0,
            relief="flat",
            highlightthickness=1,
            highlightbackground=PALETTE["border"],
            highlightcolor=PALETTE["border"]
        )
        _apply_theme(self._root)

        self._root.update_idletasks()
        apply_rounded_corners(self._root.winfo_id(), w, h, radius=18)

        CompactTitleBar(self._root, self._root.quit, self._open_settings, lambda: minimize_window(self._root))

        content = tk.Frame(self._root, bg=PALETTE["bg"])
        content.pack(fill="both", expand=True, padx=8, pady=4)

        center = tk.Frame(content, bg=PALETTE["bg"])
        center.pack(expand=True)

        # Smaller dots
        canvas_width = 100
        self._canvas = tk.Canvas(center, width=canvas_width, height=24, bg=PALETTE["bg"], bd=0, highlightthickness=0)
        self._canvas.pack()

        self._dots = []
        num_dots = 3
        dot_diameter = 10
        spacing = 20
        group_width = (num_dots - 1) * spacing + dot_diameter
        start_x = (canvas_width - group_width) / 2

        for i in range(num_dots):
            x = start_x + i * spacing
            dot = self._canvas.create_oval(x, 7, x + dot_diameter, 17, fill=PALETTE["badge_idle"], outline="")
            self._dots.append(dot)

        self._state_label = tk.Label(
            center,
            text="Open STT",
            font=("Segoe UI Semibold", 10),
            bg=PALETTE["bg"],
            fg=PALETTE["text"],
        )
        self._state_label.pack(pady=(2, 0))

        self._poll_status()
        self._animate_dots()
        self._root.mainloop()

    def _poll_status(self):
        if self._root is None:
            return
        while not self._status_queue.empty():
            status = self._status_queue.get()
            self._update_status(status)
        self._root.after(150, self._poll_status)

    def _update_status(self, status: str) -> None:
        self._current_status = status
        colors = self._badge_colors(status)
        if self._dots and self._canvas:
            for dot in self._dots:
                self._canvas.itemconfig(dot, fill=colors["bg"])
        if hasattr(self, "_state_label") and self._state_label is not None:
            self._state_label.config(text=self._status_text(status), fg=colors["fg"])

    def _badge_colors(self, status: str):
        lowered = status.lower()
        if "listen" in lowered:
            return {"bg": PALETTE["badge_listen"], "fg": PALETTE["badge_listen"]}
        if "transcrib" in lowered:
            return {"bg": PALETTE["badge_transcribe"], "fg": PALETTE["badge_transcribe"]}
        return {"bg": PALETTE["badge_idle"], "fg": PALETTE["muted"]}

    def _status_text(self, status: str) -> str:
        lowered = status.lower()
        if "listen" in lowered:
            return "Listening"
        if "transcrib" in lowered:
            return "Transcribing"
        return "Open STT"

    def _animate_dots(self):
        if not self._dots or self._canvas is None or self._root is None:
            return
        colors = self._badge_colors(self._current_status)
        base_color = colors["bg"]
        for idx, dot in enumerate(self._dots):
            phase = (self._dot_step + idx * 4) % 30
            intensity = 0.4 + 0.6 * (1 - abs(15 - phase) / 15)
            if self._current_status == "Idle":
                fill = _mix_hex(base_color, PALETTE["bg"], 0.5)
            else:
                fill = _mix_hex(base_color, PALETTE["text"], intensity * 0.5)
            self._canvas.itemconfig(dot, fill=fill)
        self._dot_step = (self._dot_step + 1) % 30
        self._root.after(50, self._animate_dots)

    def _open_settings(self):
        if self._root is None:
            return
        SettingsWindow(self._root, on_saved=self._on_settings_saved).open()


class SettingsWindow:
    def __init__(self, root: tk.Tk, on_saved: Optional[Callable[[], None]] = None):
        self.root = root
        self.on_saved = on_saved
        self.cfg_manager = ConfigManager()
        self.window: Optional[tk.Toplevel] = None
        self._drag_start = (0, 0)

    def open(self):
        if self.window:
            self.window.lift()
            return
        cfg = self.cfg_manager.config
        self.window = tk.Toplevel(self.root)
        self.window.title("Settings")
        w, h = 420, 760
        self.window.geometry(f"{w}x{h}")
        self.window.configure(bg=PALETTE["bg"])
        self.window.overrideredirect(True)
        self.window.minsize(w, 620)
        self.window.config(
            bd=0,
            relief="flat",
            highlightthickness=1,
            highlightbackground=PALETTE["border"],
            highlightcolor=PALETTE["border"]
        )
        _apply_theme(self.window)
        self.window.update_idletasks()
        apply_rounded_corners(self.window.winfo_id(), w, h, radius=20)

        TitleBar(self.window, "Settings", self.window.destroy, on_minimize=lambda: minimize_window(self.window))

        # Pack actions before the form so they always retain space when the
        # settings window is shorter than the full form.
        btn_frame = tk.Frame(self.window, bg=PALETTE["bg"])
        btn_frame.pack(side="bottom", fill="x", padx=20, pady=(8, 18))

        ResizeGrip(btn_frame, self.window).pack(side="right", anchor="se")

        ModernButton(btn_frame, "Save", self._save,
                     bg=PALETTE["accent"], hover_bg=PALETTE["accent_hover"], fg="#0f172a", width=10).pack(side="right", padx=8)
        ModernButton(btn_frame, "Cancel", self.window.destroy,
                     bg=PALETTE["card"], hover_bg=PALETTE["border"], width=8).pack(side="right", padx=8)

        container = tk.Frame(self.window, bg=PALETTE["bg"])
        container.pack(fill="both", expand=True, padx=20, pady=14)

        # --- INPUT FIELDS ---
        self._hotkey = self._add_entry(container, "Hotkey", cfg.hotkey)
        self._mode = self._add_combo(container, "Mode", ["push_to_talk", "toggle"], cfg.mode)
        self._output_mode = self._add_combo(container, "Output", ["type", "clipboard", "paste"], cfg.output_mode)
        self._stt_backend = self._add_combo(
            container,
            "STT backend",
            ["whisper", "nemotron"],
            cfg.stt_backend,
        )
        self._model_size = self._add_combo(container, "Whisper model", ["tiny", "base", "small", "medium", "large"], cfg.model_size)
        self._language = self._add_entry(container, "Whisper language", cfg.language)
        self._nemotron_target_lang = self._add_entry(container, "Nemotron target language", cfg.nemotron_target_lang)
        self._nemotron_latency_ms = self._add_combo(
            container,
            "Nemotron ONNX latency (fixed)",
            ["560"],
            str(cfg.nemotron_latency_ms),
        )
        self._mic_device = self._add_combo(container, "Microphone", self._mic_choices(), cfg.mic_device or "")

        # --- CHECKBOXES ---
        chk_frame = tk.Frame(container, bg=PALETTE["bg"])
        chk_frame.pack(fill="x", pady=10)
        self._spoken_punct = tk.BooleanVar(value=cfg.spoken_punctuation)
        self._auto_paste = tk.BooleanVar(value=cfg.auto_paste_clipboard)
        self._live_typing = tk.BooleanVar(value=cfg.live_typing)

        self._make_checkbox(chk_frame, "Type Nemotron transcript while speaking", self._live_typing)
        self._make_checkbox(chk_frame, "Parse spoken punctuation", self._spoken_punct)
        self._make_checkbox(chk_frame, "Auto-paste from clipboard", self._auto_paste)

    def _make_checkbox(self, parent, text, var):
        cb = tk.Checkbutton(parent, text=text, variable=var,
                            bg=PALETTE["bg"], fg=PALETTE["text"],
                            selectcolor=PALETTE["card"],
                            activebackground=PALETTE["bg"], activeforeground=PALETTE["text"],
                            font=("Segoe UI", 9), bd=0, highlightthickness=0)
        cb.pack(anchor="w", pady=3)

    def _add_entry(self, parent, label: str, value: str) -> tk.StringVar:
        var = tk.StringVar(value=value)
        self._create_field_label(parent, label)
        wrapper = tk.Frame(parent, bg=PALETTE["card"], pady=1, padx=1)
        wrapper.pack(fill="x", pady=(0, 10))
        entry = tk.Entry(wrapper, textvariable=var, font=("Segoe UI", 10),
                         bg=PALETTE["card"], fg=PALETTE["text"],
                         insertbackground=PALETTE["text"],
                         bd=0, highlightthickness=4, highlightbackground=PALETTE["card"], highlightcolor=PALETTE["card"])
        entry.pack(fill="x", ipady=2)
        return var

    def _add_combo(self, parent, label: str, values: Sequence[str], current: str) -> tk.StringVar:
        var = tk.StringVar(value=current)
        self._create_field_label(parent, label)
        combo = ttk.Combobox(parent, textvariable=var, values=list(values), state="readonly", font=("Segoe UI", 10))
        combo.pack(fill="x", pady=(0, 10), ipady=3)
        return var

    def _create_field_label(self, parent, text):
        tk.Label(parent, text=text, bg=PALETTE["bg"], fg=PALETTE["muted"],
                 font=("Segoe UI", 9, "bold"), anchor="w").pack(fill="x", pady=(0, 2))

    def _mic_choices(self) -> Sequence[str]:
        devices = list_input_devices()
        return devices or ["Default"]

    def _save(self):
        cfg = {
            "hotkey": self._hotkey.get(),
            "mode": self._mode.get(),
            "output_mode": self._output_mode.get(),
            "stt_backend": self._stt_backend.get(),
            "model_size": self._model_size.get(),
            "language": self._language.get(),
            "nemotron_target_lang": self._nemotron_target_lang.get(),
            "nemotron_latency_ms": int(self._nemotron_latency_ms.get()),
            "mic_device": self._mic_device.get() or None,
            "live_typing": self._live_typing.get(),
            "spoken_punctuation": self._spoken_punct.get(),
            "auto_paste_clipboard": self._auto_paste.get(),
        }
        self.cfg_manager.update(**cfg)
        if self.on_saved:
            try:
                self.on_saved()
            except Exception as exc:  # noqa: BLE001
                reverted = self.cfg_manager.load()
                self._stt_backend.set(reverted.stt_backend)
                messagebox.showerror(
                    "Could not load STT backend",
                    f"{exc}\n\nThe backend was reverted to {reverted.stt_backend}.",
                    parent=self.window,
                )
                return
        self.window.destroy()


def _mix_hex(base: str, target: str, t: float) -> str:
    def hex_to_rgb(h):
        h = h.lstrip("#")
        return tuple(int(h[i: i + 2], 16) for i in (0, 2, 4))
    br, bg, bb = hex_to_rgb(base)
    tr, tg, tb = hex_to_rgb(target)
    r = int(br + (tr - br) * t)
    g = int(bg + (tg - bg) * t)
    b = int(bb + (tb - bb) * t)
    return f"#{r:02x}{g:02x}{b:02x}"
