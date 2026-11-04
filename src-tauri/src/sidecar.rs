//! The Python sidecar (`sidecar/main.py`): one JSON object per line on its stdin and stdout.
//!
//! Requests carry an `id` and get an answer with the same `id`. Lines with `event` are device
//! events, forwarded to the main window as the Tauri event `device-event`. The sidecar writes
//! its log on stderr, which goes to the stderr of the runner.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct Sidecar {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicU64,
}

/// Debug: the Python sources through uv. Release: the PyInstaller executable that the
/// bundle places next to the runner executable.
fn command() -> Result<Command, String> {
    if cfg!(debug_assertions) {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("sidecar");
        let mut command = Command::new("uv");
        command.args(["run", "--quiet", "--project"]).arg(&dir).arg("python").arg(dir.join("main.py"));
        Ok(command)
    } else {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let name = format!("genropy-sidecar{}", std::env::consts::EXE_SUFFIX);
        Ok(Command::new(exe.parent().unwrap().join(name)))
    }
}

impl Sidecar {
    pub fn start(app: AppHandle, main_window: &'static str, config: &Path) -> Result<Self, String> {
        let mut command = command()?;
        let mut child = command
            .arg("--config")
            .arg(config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("cannot start the sidecar ({:?}): {e}", command.get_program()))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let pending: Pending = Arc::default();
        let reader_pending = pending.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Value>(&line) {
                    Ok(message) => dispatch(&app, main_window, &reader_pending, message),
                    Err(e) => eprintln!("[runner] sidecar line is not JSON ({e}): {line}"),
                }
            }
            eprintln!("[runner] sidecar exited");
            for (_, reply) in reader_pending.lock().unwrap().drain() {
                let _ = reply.send(Err("sidecar exited".to_string()));
            }
        });
        Ok(Self { child: Mutex::new(child), stdin: Mutex::new(stdin), pending, next_id: AtomicU64::new(1) })
    }

    pub async fn call(&self, module: &str, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, reply);
        let line = json!({"id": id, "module": module, "method": method, "params": params}).to_string();
        let written = {
            let mut stdin = self.stdin.lock().unwrap();
            writeln!(stdin, "{line}").and_then(|_| stdin.flush())
        };
        if let Err(e) = written {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("cannot write to the sidecar: {e}"));
        }
        answer.await.unwrap_or_else(|_| Err("sidecar exited".to_string()))
    }

    pub fn stop(&self) {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn dispatch(app: &AppHandle, main_window: &str, pending: &Pending, message: Value) {
    if let Some(id) = message.get("id").and_then(Value::as_u64) {
        let Some(reply) = pending.lock().unwrap().remove(&id) else {
            eprintln!("[runner] sidecar answer with unknown id {id}");
            return;
        };
        let result = match message.get("error") {
            Some(error) => Err(format!(
                "{}: {}",
                error.get("code").and_then(Value::as_str).unwrap_or("error"),
                error.get("message").and_then(Value::as_str).unwrap_or("")
            )),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = reply.send(result);
    } else if message.get("event").is_some() {
        if let Err(e) = app.emit_to(main_window, "device-event", message) {
            eprintln!("[runner] cannot emit device-event: {e}");
        }
    } else if let Some(modules) = message.get("modules") {
        eprintln!("[runner] sidecar ready, modules: {modules}");
    } else {
        eprintln!("[runner] sidecar line not understood: {message}");
    }
}
