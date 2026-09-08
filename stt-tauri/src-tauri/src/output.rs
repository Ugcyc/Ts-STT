use crate::{features::settled_prefix, focus::Destination};
use enigo::{Enigo, Key, KeyboardControllable};
use std::{
    sync::{mpsc, Arc, Condvar, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter};

struct Pending {
    text: Option<String>,
    finish: Option<mpsc::Sender<Result<(), String>>>,
    cancelled: bool,
}

pub struct OutputWorker {
    pending: Arc<(Mutex<Pending>, Condvar)>,
}

impl OutputWorker {
    pub fn new(
        delay: u32,
        stable: bool,
        destination: Result<Option<Destination>, String>,
        app: Option<AppHandle>,
    ) -> Self {
        let pending = Arc::new((
            Mutex::new(Pending {
                text: None,
                finish: None,
                cancelled: false,
            }),
            Condvar::new(),
        ));
        let shared = pending.clone();
        thread::spawn(move || {
            let mut typed = String::new();
            let mut failure = destination.as_ref().err().cloned();
            let target = destination.ok().flatten();
            let mut enigo = Enigo::new();
            loop {
                let (text, finish) = {
                    let (lock, wake) = &*shared;
                    let mut p = lock.lock().unwrap();
                    while p.text.is_none() && !p.cancelled {
                        p = wake.wait(p).unwrap();
                    }
                    if p.cancelled {
                        return;
                    }
                    (p.text.take().unwrap_or_default(), p.finish.take())
                };
                let text = if stable && finish.is_none() {
                    settled_prefix(&text)
                } else {
                    text
                };
                if failure.is_none() {
                    let prefix = typed
                        .chars()
                        .zip(text.chars())
                        .take_while(|(a, b)| a == b)
                        .count();
                    let remove = typed.chars().count().saturating_sub(prefix);
                    let check = || -> Result<(), String> {
                        if shared.0.lock().unwrap().cancelled {
                            return Err("Insertion cancelled.".into());
                        }
                        if target.as_ref().is_some_and(|d| !d.unchanged()) {
                            return Err("Insertion paused because focus or manual input changed. Copy the completed transcript from the app.".into());
                        }
                        Ok(())
                    };
                    let result = (|| {
                        for _ in 0..remove {
                            check()?;
                            enigo.key_click(Key::Backspace);
                        }
                        let mut buffer = [0; 4];
                        for c in text.chars().skip(prefix) {
                            check()?;
                            enigo.key_sequence(c.encode_utf8(&mut buffer));
                            if delay > 0 {
                                thread::sleep(Duration::from_millis(delay as u64));
                            }
                        }
                        check()?;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        if let Some(handle) = &app {
                            let _ = handle.emit("dictation-warning", &e);
                        }
                        failure = Some(e);
                    } else {
                        typed = text;
                    }
                }
                if let Some(done) = finish {
                    let _ = done.send(failure.map_or(Ok(()), Err));
                    return;
                }
            }
        });
        Self { pending }
    }
    pub fn update(&self, text: String) {
        let (lock, wake) = &*self.pending;
        lock.lock().unwrap().text = Some(text);
        wake.notify_one();
    }
    pub fn finish(self, text: String) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        {
            let mut p = self.pending.0.lock().unwrap();
            p.text = Some(text);
            p.finish = Some(tx);
        }
        self.pending.1.notify_one();
        rx.recv()
            .map_err(|_| "Output worker stopped. Copy your transcript from the app.".to_string())?
    }
}
impl Drop for OutputWorker {
    fn drop(&mut self) {
        if let Ok(mut p) = self.pending.0.lock() {
            p.cancelled = true;
        }
        self.pending.1.notify_one();
    }
}
