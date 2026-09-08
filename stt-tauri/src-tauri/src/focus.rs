// Windows hooks observe only physical editing input. Injected Enigo keystrokes
// do not advance the epoch. Any manual edit latches output off for this session.
#[derive(Clone, Debug)]
pub struct Destination {
    window: isize,
    epoch: usize,
}

impl Destination {
    pub fn capture(enabled: bool) -> Result<Option<Self>, String> {
        if !enabled {
            return Ok(None);
        }
        platform::capture().map(Some)
    }
    pub fn unchanged(&self) -> bool {
        platform::unchanged(self)
    }
}

pub fn configure_hotkey(hotkey: &str) {
    platform::configure_hotkey(hotkey);
}

pub fn supported() -> bool {
    cfg!(windows)
}

#[cfg(not(windows))]
mod platform {
    use super::Destination;
    pub fn configure_hotkey(_: &str) {}
    pub fn capture() -> Result<Destination, String> {
        Err("Protected insertion is currently available on Windows. Your transcript will remain available to copy.".into())
    }
    pub fn unchanged(destination: &Destination) -> bool {
        let _ = (destination.window, destination.epoch);
        false
    }
}

#[cfg(windows)]
mod platform {
    use super::Destination;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, OnceLock,
        },
        thread,
    };
    static HOTKEY: AtomicUsize = AtomicUsize::new(0);
    static MODS: AtomicUsize = AtomicUsize::new(0);
    static EPOCH: AtomicUsize = AtomicUsize::new(0);
    static HOOKS: OnceLock<Result<(), String>> = OnceLock::new();
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Message {
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
        private: u32,
    }
    #[repr(C)]
    struct Keyboard {
        vk: u32,
        scan: u32,
        flags: u32,
        time: u32,
        extra: usize,
    }
    #[repr(C)]
    struct Mouse {
        pt: Point,
        data: u32,
        flags: u32,
        time: u32,
        extra: usize,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetForegroundWindow() -> isize;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn SetWindowsHookExW(
            id: i32,
            callback: unsafe extern "system" fn(i32, usize, isize) -> isize,
            module: isize,
            thread: u32,
        ) -> isize;
        fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
        fn UnhookWindowsHookEx(hook: isize) -> i32;
        fn GetMessageW(msg: *mut Message, hwnd: isize, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Message) -> i32;
        fn DispatchMessageW(msg: *const Message) -> isize;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
    }
    unsafe extern "system" fn keyboard(code: i32, wp: usize, lp: isize) -> isize {
        if code >= 0 && (wp == 0x100 || wp == 0x104) && (*(lp as *const Keyboard)).flags & 0x10 == 0
        {
            let vk = (*(lp as *const Keyboard)).vk as usize;
            let modifiers = usize::from(GetAsyncKeyState(0x11) < 0)
                | (usize::from(GetAsyncKeyState(0x10) < 0) << 1)
                | (usize::from(GetAsyncKeyState(0x12) < 0) << 2)
                | (usize::from(GetAsyncKeyState(0x5B) < 0 || GetAsyncKeyState(0x5C) < 0) << 3);
            let modifier_key = matches!(vk,0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
            if !modifier_key
                && !(vk == HOTKEY.load(Ordering::SeqCst)
                    && modifiers == MODS.load(Ordering::SeqCst))
            {
                EPOCH.fetch_add(1, Ordering::SeqCst);
            }
        }
        CallNextHookEx(0, code, wp, lp)
    }
    unsafe extern "system" fn mouse(code: i32, wp: usize, lp: isize) -> isize {
        if code >= 0
            && matches!(wp, 0x201 | 0x204 | 0x207 | 0x20A | 0x20B | 0x20E)
            && (*(lp as *const Mouse)).flags & 1 == 0
        {
            EPOCH.fetch_add(1, Ordering::SeqCst);
        }
        CallNextHookEx(0, code, wp, lp)
    }
    pub fn configure_hotkey(hotkey: &str) {
        let mut modifiers = 0;
        let mut key = 0;
        for part in hotkey.to_lowercase().split('+') {
            match part.trim() {
                "commandorcontrol" | "ctrl" | "control" => modifiers |= 1,
                "shift" => modifiers |= 2,
                "alt" | "option" => modifiers |= 4,
                "super" | "meta" | "command" | "cmd" => modifiers |= 8,
                "space" => key = 0x20,
                "enter" => key = 0x0D,
                "tab" => key = 9,
                "escape" => key = 0x1B,
                value if value.len() == 1 => {
                    key = value.as_bytes()[0].to_ascii_uppercase() as usize
                }
                value if value.starts_with('f') => {
                    if let Ok(n) = value[1..].parse::<usize>() {
                        if (1..=24).contains(&n) {
                            key = 0x6F + n;
                        }
                    }
                }
                _ => (),
            }
        }
        HOTKEY.store(key, Ordering::SeqCst);
        MODS.store(modifiers, Ordering::SeqCst);
    }
    pub fn capture() -> Result<Destination, String> {
        HOOKS
            .get_or_init(|| {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || unsafe {
                    let module = GetModuleHandleW(std::ptr::null());
                    let k = SetWindowsHookExW(13, keyboard, module, 0);
                    let m = SetWindowsHookExW(14, mouse, module, 0);
                    if k == 0 || m == 0 {
                        if k != 0 {
                            UnhookWindowsHookEx(k);
                        }
                        if m != 0 {
                            UnhookWindowsHookEx(m);
                        }
                        let _ = tx.send(Err(
                            "Could not monitor destination edits; insertion paused.".into(),
                        ));
                        return;
                    }
                    let _ = tx.send(Ok(()));
                    let mut msg: Message = std::mem::zeroed();
                    while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    UnhookWindowsHookEx(k);
                    UnhookWindowsHookEx(m);
                    EPOCH.fetch_add(1, Ordering::SeqCst);
                });
                rx.recv()
                    .unwrap_or_else(|_| Err("Input monitor did not start.".into()))
            })
            .clone()?;
        unsafe {
            let window = GetForegroundWindow();
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            if window == 0 || pid == std::process::id() {
                return Err("Focus your destination app and start dictation with the hotkey. The transcript remains available to copy.".into());
            }
            Ok(Destination {
                window,
                epoch: EPOCH.load(Ordering::SeqCst),
            })
        }
    }
    pub fn unchanged(d: &Destination) -> bool {
        unsafe { GetForegroundWindow() == d.window && EPOCH.load(Ordering::SeqCst) == d.epoch }
    }
}
