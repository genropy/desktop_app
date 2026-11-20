//! The Python sidecar (`sidecar/main.py`): one JSON object per line on its stdin and stdout.
//!
//! Requests carry an `id` and get an answer with the same `id`. Lines with `event` are device
//! events, forwarded to the main window as the Tauri event `device-event`. The sidecar writes
//! its log on stderr, which goes to the stderr of the runner.
//!
//! Built with the feature `sidecar`, which only the "industry" build has (machine-side devices).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};
use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Emitter, Manager, State, Url, Webview};
use tokio::sync::oneshot;

use crate::modules::{self, Installed};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// One running sidecar process. A restart replaces it; the calls still waiting on the old one fail.
struct Process {
    child: Child,
    stdin: ChildStdin,
    pending: Pending,
}

pub struct Sidecar {
    /// Window labels that already received `device_call`.
    labels: Mutex<std::collections::HashSet<String>>,
    app: AppHandle,
    config: PathBuf,
    process: Mutex<Process>,
    /// The downloaded modules loaded by the running process.
    installed: Mutex<Vec<Installed>>,
    next_id: AtomicU64,
    /// Held while modules are downloaded or removed, so two updates never write the same folder.
    pub updating: tokio::sync::Mutex<()>,
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

/// Starts the process with `--module <name> <folder>` for each verified downloaded module.
fn spawn(app: &AppHandle, config: &Path, installed: &[Installed]) -> Result<Process, String> {
    let mut command = command()?;
    command.arg("--config").arg(config);
    for module in installed {
        command.arg("--module").arg(&module.name).arg(&module.path);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot start the sidecar ({:?}): {e}", command.get_program()))?;
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let pending: Pending = Arc::default();
    let reader_pending = pending.clone();
    let app = app.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Value>(&line) {
                Ok(message) => dispatch(&app, &reader_pending, message),
                Err(e) => eprintln!("[runner] sidecar line is not JSON ({e}): {line}"),
            }
        }
        eprintln!("[runner] sidecar exited");
        for (_, reply) in reader_pending.lock().unwrap().drain() {
            let _ = reply.send(Err("sidecar exited".to_string()));
        }
    });
    Ok(Process { child, stdin, pending })
}

impl Sidecar {
    pub fn start(app: AppHandle, config: &Path) -> Result<Self, String> {
        let installed = modules::load(&app);
        let process = spawn(&app, config, &installed)?;
        Ok(Self {
            labels: Mutex::default(),
            app,
            config: config.to_path_buf(),
            process: Mutex::new(process),
            installed: Mutex::new(installed),
            next_id: AtomicU64::new(1),
            updating: tokio::sync::Mutex::new(()),
        })
    }

    /// Stops the process and starts a new one with the modules installed now.
    pub fn restart(&self) -> Result<(), String> {
        let installed = modules::load(&self.app);
        let mut process = self.process.lock().unwrap();
        let _ = process.child.kill();
        let _ = process.child.wait();
        *process = spawn(&self.app, &self.config, &installed)?;
        *self.installed.lock().unwrap() = installed;
        Ok(())
    }

    pub fn installed(&self) -> Vec<Installed> {
        self.installed.lock().unwrap().clone()
    }

    pub async fn call(&self, module: &str, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        let line = json!({"id": id, "module": module, "method": method, "params": params}).to_string();
        {
            let mut process = self.process.lock().unwrap();
            process.pending.lock().unwrap().insert(id, reply);
            let written = writeln!(process.stdin, "{line}").and_then(|_| process.stdin.flush());
            if let Err(e) = written {
                process.pending.lock().unwrap().remove(&id);
                return Err(format!("cannot write to the sidecar: {e}"));
            }
        }
        answer.await.unwrap_or_else(|_| Err("sidecar exited".to_string()))
    }

    pub fn stop(&self) {
        let mut process = self.process.lock().unwrap();
        let _ = process.child.kill();
        let _ = process.child.wait();
    }
}

fn dispatch(app: &AppHandle, pending: &Pending, message: Value) {
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
        let Some((label, doc)) = crate::documents::active(app) else { return };
        let Some(window) = app.get_webview_window(&label) else { return };
        if window.url().ok().map(|u| u.origin()) != tauri::Url::parse(&doc.url).ok().map(|u| u.origin()) { return; }
        if let Err(e) = app.emit_to(&*label, "device-event", message) {
            eprintln!("[runner] cannot emit device-event: {e}");
        }
    } else if let Some(modules) = message.get("modules") {
        eprintln!("[runner] sidecar ready, modules: {modules}");
    } else {
        eprintln!("[runner] sidecar line not understood: {message}");
    }
}

/// Grants `device_call` and the events to the pages of `url`'s origin in the window `label`.
pub fn grant(app: &AppHandle, url: &Url, label: &str) -> tauri::Result<()> {
    let sidecar = app.state::<Sidecar>();
    let mut labels = sidecar.labels.lock().unwrap();
    if labels.contains(label) {
        return Ok(());
    }
    let origin = url.origin().ascii_serialization();
    let capability = CapabilityBuilder::new(format!("sidecar-{}", labels.len()))
        .remote(format!("{origin}/*"))
        .local(false)
        .window(label)
        .permission("allow-device-call")
        .permission("core:event:default");
    app.add_capability(capability)?;
    labels.insert(label.into());
    Ok(())
}

/// Calls `module.method(**params)` in the Python sidecar. A downloaded module answers only the origins of its
/// signed manifest.
#[tauri::command]
pub async fn device_call(
    sidecar: State<'_, Sidecar>,
    webview: Webview,
    module: String,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    let origin = webview.url().map_err(|e| e.to_string())?.origin().ascii_serialization();
    modules::may_call(&sidecar.installed(), &module, &origin)?;
    sidecar.call(&module, &method, params.unwrap_or(Value::Null)).await
}

/// Starts the sidecar.
pub fn setup(app: &AppHandle) -> Result<(), String> {
    app.manage(Sidecar::start(app.clone(), &crate::config_path(app)?)?);
    Ok(())
}

pub fn stop(app: &AppHandle) {
    app.state::<Sidecar>().stop();
}
