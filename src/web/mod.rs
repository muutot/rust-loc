//! The local web front end: an axum server on the loopback interface, a
//! browser page for the four panes and a tray icon that opens it.
//!
//! The server only ever listens on `127.0.0.1`. Every request must carry a
//! per-run token (header or query) and name the loopback host, so another
//! page or process on the machine cannot drive a scan.

pub mod api;
pub mod assets;
pub mod dto;
#[cfg(windows)]
pub mod tray;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{watch, Notify};

use crate::authors;
use crate::scan;
use crate::settings::Settings;

/// The port the server tries first; occupied ports walk upward.
pub const DEFAULT_PORT: u16 = 8422;
/// How far the port may walk before giving up.
pub const PORT_ATTEMPTS: u16 = 20;

/// Where a scan currently is. Sent over a watch channel, so a reader that
/// joins late sees the current state without replaying history.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScanStatus {
    Idle,
    Scanning {
        stage: &'static str,
        done: usize,
        total: usize,
        path: String,
    },
    Done {
        elapsed: f64,
    },
    Failed {
        error: String,
    },
}

/// Where the author pass currently is. It runs on demand after a scan, from
/// the Authors pane, the way the native shell schedules it.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthorsStatus {
    Idle,
    Blaming {
        done: usize,
        total: usize,
    },
    Ready,
}

/// One `/api/events` frame: both progress channels in a single JSON object.
#[derive(Clone, Serialize)]
pub struct StatusSnapshot {
    pub scan: ScanStatus,
    pub authors: AuthorsStatus,
}

/// Everything the handlers read. The watch senders push progress; the mutexes
/// hold the finished result until the next scan replaces it.
pub struct AppState {
    pub token: String,
    pub scan: watch::Sender<ScanStatus>,
    pub authors: watch::Sender<AuthorsStatus>,
    pub result: Mutex<Option<Arc<dto::ScanResult>>>,
    pub blame: Mutex<Option<Arc<authors::Authors>>>,
    pub blame_running: AtomicBool,
    /// True while a scan runs: a second POST /api/scan is refused, not queued.
    /// An atomic because the check-and-set must be one step.
    pub scanning: AtomicBool,
    pub settings: Mutex<Settings>,
}

impl AppState {
    pub fn new(token: String) -> Arc<AppState> {
        let settings = crate::settings::load_settings();
        let (scan_tx, _) = watch::channel(ScanStatus::Idle);
        let (authors_tx, _) = watch::channel(AuthorsStatus::Idle);
        Arc::new(AppState {
            token,
            scan: scan_tx,
            authors: authors_tx,
            result: Mutex::new(None),
            blame: Mutex::new(None),
            blame_running: AtomicBool::new(false),
            scanning: AtomicBool::new(false),
            settings: Mutex::new(settings),
        })
    }

    /// The current status pair, for the polling endpoint.
    pub fn snapshot(self: &Arc<Self>) -> StatusSnapshot {
        StatusSnapshot {
            scan: self.scan.borrow().clone(),
            authors: self.authors.borrow().clone(),
        }
    }
}

/// What one run of the server needs to know.
pub struct ServerOptions {
    pub first_port: u16,
    pub token: String,
    /// Reports the port that actually bound, or the error that stopped the
    /// server. The tray thread waits on this before it draws its icon.
    pub port_tx: std::sync::mpsc::Sender<Result<u16, String>>,
    /// Fired to stop the server.
    pub shutdown: Arc<Notify>,
}

/// Bind a loopback port, walking upward past whatever is taken, then serve
/// until the shutdown handle fires. Returns the port that bound; the server
/// keeps running on this runtime after the future returns, so callers either
/// park the runtime (tray mode) or await their own stop signal (headless).
pub async fn bind_and_serve(opts: ServerOptions) -> Result<u16, String> {
    let mut port = opts.first_port;
    let listener = loop {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => break listener,
            Err(_) if port < opts.first_port.saturating_add(PORT_ATTEMPTS) => port += 1,
            Err(error) => {
                let message = format!("no free port from {} onward: {error}", opts.first_port);
                let _ = opts.port_tx.send(Err(message.clone()));
                return Err(message);
            }
        }
    };
    write_web_json(port, &opts.token);
    let _ = opts.port_tx.send(Ok(port));

    let state = AppState::new(opts.token);
    let app = router(state);
    let shutdown = opts.shutdown;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown.notified().await;
            })
            .await;
    });
    Ok(port)
}

/// Start the server on its own thread with its own runtime, which parks for
/// the rest of the process so the spawned server task stays alive. The caller
/// keeps the main thread for the tray message loop.
pub fn start_server(opts: ServerOptions) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = opts.port_tx.send(Err(format!("tokio runtime: {error}")));
                return;
            }
        };
        if let Err(error) = runtime.block_on(bind_and_serve(opts)) {
            eprintln!("rust-loc-web: {error}");
        }
        // Hold the runtime open: the server task lives on it.
        runtime.block_on(std::future::pending::<()>());
    })
}

/// Run the server in the foreground on the current thread; Ctrl-C stops it.
/// This is the `--no-tray` path and the only path on platforms without a tray.
pub async fn serve_forever(opts: ServerOptions, token: &str) -> Result<u16, String> {
    let shutdown = opts.shutdown.clone();
    let port = bind_and_serve(opts).await?;
    println!("rust-loc-web: http://127.0.0.1:{port}/?token={token}");
    let _ = tokio::signal::ctrl_c().await;
    shutdown.notify_waiters();
    // Give the graceful shutdown a moment, then let the caller exit.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    Ok(port)
}

/// The file a second launch reads to find the first one: the bound port and
/// the token that run accepts.
pub fn web_json_path() -> PathBuf {
    crate::settings::settings_path().with_file_name("web.json")
}

fn write_web_json(port: u16, token: &str) {
    let path = web_json_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = format!("{{\"port\":{port},\"token\":\"{token}\"}}");
    let _ = std::fs::write(path, body);
}

pub fn read_web_json() -> Option<(u16, String)> {
    let text = std::fs::read_to_string(web_json_path()).ok()?;
    let port = text
        .split("\"port\":")
        .nth(1)?
        .split([',', '}'])
        .next()?
        .trim()
        .parse()
        .ok()?;
    let token = text
        .split("\"token\":\"")
        .nth(1)?
        .split('"')
        .next()?
        .to_string();
    Some((port, token))
}

/// A per-run token. On Windows it comes from the system RNG; elsewhere from
/// hashing the clock and the process id, which is enough to distinguish runs.
pub fn generate_token() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::BCryptGenRandom;
        use windows_sys::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG;
        let mut bytes = [0u8; 16];
        let ok = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if ok == 0 {
            return bytes.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    // Fallback: a FNV-1a fold of the wall clock and pid. Not cryptographic,
    // but it separates runs, which is what the token needs to do.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for value in [
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::process::id() as u128,
    ] {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}{:016x}", hash.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// Open the page in the default browser.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let verb = to_wide("open");
        let file = to_wide(url);
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL as i32,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

#[cfg(windows)]
pub fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The scan the POST /api/scan handler runs: settings become command line
/// flags, the CLI parser turns those into options, and the pipeline scans on
/// a blocking thread while the watch channel carries progress.
pub fn build_scan_args(settings: &Settings, roots: &[String], extra: &[String]) -> Vec<String> {
    let mut args: Vec<String> = roots.to_vec();
    if !settings.gitignore {
        args.push("--no-gitignore".into());
    }
    if !settings.default_ignores {
        args.push("--no-ignore-dirs".into());
    }
    for dir in &settings.extra_dirs {
        args.push("--ignore-dir".into());
        args.push(dir.clone());
    }
    if settings.include_tests {
        args.push("--include-tests".into());
    }
    if !settings.complexity {
        args.push("--no-complexity".into());
    }
    if settings.uloc {
        args.push("--uloc".into());
    }
    if settings.exclude_generated {
        args.push("--exclude-generated".into());
    }
    if settings.cocomo {
        args.push("--cocomo".into());
    }
    // Deliberately no --authors: the header Scan button scans files only;
    // blame is a separate action the Authors view triggers.
    // The tables the page draws read counters these switches collect; the
    // native shell turns them on for every scan as well.
    args.push("--functions".into());
    args.push("--top-functions".into());
    args.push("200".into());
    args.push("--smells".into());
    // The language breakdown the overview chart plots.
    args.push("--languages".into());
    // The file pairs that keep changing together; the overview plots them.
    args.push("--coupling".into());
    // The complexity x churn ranking the overview tabulates.
    args.push("--hotspots".into());
    // The copied-block listing the overview tabulates.
    args.push("--duplicates".into());
    // The maintainability card (Halstead volume, MI score) the quality view
    // shows; the counters come out of the same single pass.
    args.push("--metrics".into());
    // The quality gates the settings name; the overview shows whether each
    // one holds.
    for spec in settings
        .fail_if
        .split([',', ';', '\r', '\n'])
        .map(str::trim)
        .filter(|spec| !spec.is_empty())
    {
        args.push("--fail-if".into());
        args.push(spec.to_string());
    }
    for glob in crate::settings::parse_list(&settings.exclude) {
        args.push("--exclude".into());
        args.push(glob);
    }
    for ext in settings.exclude_ext.split(',') {
        let ext = ext.trim().trim_start_matches('.');
        if !ext.is_empty() {
            args.push("--exclude-ext".into());
            args.push(ext.to_string());
        }
    }
    if let Ok(size) = settings.max_file_size.trim().parse::<u64>() {
        if size > 0 {
            args.push("--max-file-size".into());
            args.push(size.to_string());
        }
    }
    let slugs = crate::settings::sources_slugs(crate::settings::sources_of(&settings.sources));
    args.push("--sources".into());
    args.push(slugs);
    // The page's own overrides last, so a flag here wins over the settings.
    args.extend(extra.iter().cloned());
    args
}

/// The roots a scan covers: the overrides the request named, else the saved
/// ones, else the working directory the process started in.
pub fn scan_roots(settings: &Settings, overrides: &[String]) -> Vec<String> {
    if !overrides.is_empty() {
        return overrides.to_vec();
    }
    let saved = crate::settings::parse_paths(&settings.roots);
    if saved.is_empty() {
        vec![std::env::current_dir()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string())]
    } else {
        saved
    }
}

/// Milliseconds since the epoch, for progress throttling.
fn millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run one scan to completion on a blocking thread, pushing throttled progress
/// into the watch channel as the pipeline walks the files.
pub fn run_scan_blocking(
    scan_tx: watch::Sender<ScanStatus>,
    args: Vec<String>,
    cwd: PathBuf,
) -> Result<dto::ScanResult, String> {
    use std::panic::AssertUnwindSafe;

    let invocation = crate::cli::parse(&args, &cwd)?;
    if invocation.roots.is_empty() {
        return Err("no scan roots given".to_string());
    }
    let started = std::time::Instant::now();
    let roots = invocation.roots.clone();
    let options = invocation.options.clone();
    // Progress goes out at most every 60 ms or at a stage boundary, so a
    // hundred thousand files cannot flood the browser.
    let last_push = std::sync::atomic::AtomicU64::new(0);
    let progress = |stage: scan::Stage, done: usize, total: usize, path: &Path| {
        let boundary = done >= total;
        let now = millis_now();
        if boundary
            || now.saturating_sub(last_push.load(Ordering::Relaxed)) > 60
        {
            last_push.store(now, Ordering::Relaxed);
            scan_tx.send_replace(ScanStatus::Scanning {
                stage: stage_name(stage),
                done,
                total,
                path: scan::posix(path),
            });
        }
    };
    let scan_tx_panic = scan_tx.clone();
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        report_scan(&roots, &options, &cwd, Some(&progress))
    }));
    match outcome {
        Ok(report) => {
            let mut result = dto::ScanResult {
                report,
                history: Vec::new(),
                elapsed: started.elapsed().as_secs_f64(),
                generated_at: millis_now() / 1000,
            };
            let root_names: Vec<String> = roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect();
            result.record_history(&root_names);
            Ok(result)
        }
        Err(payload) => {
            let detail = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "the scan panicked".to_string());
            let _ = scan_tx_panic;
            Err(detail)
        }
    }
}

/// The pipeline entry, named so the closure above stays readable.
fn report_scan(
    roots: &[PathBuf],
    options: &crate::options::Options,
    cwd: &Path,
    progress: Option<&scan::Progress<'_>>,
) -> crate::report::Report {
    crate::report::scan(roots, options, cwd, progress)
}

fn stage_name(stage: scan::Stage) -> &'static str {
    match stage {
        scan::Stage::Files => "files",
        scan::Stage::Blame => "blame",
    }
}

/// Blame every business file on a blocking thread, progress and all.
pub fn run_authors_blocking(
    authors_tx: watch::Sender<AuthorsStatus>,
    report: &crate::report::Report,
) -> authors::Authors {
    use std::panic::AssertUnwindSafe;

    let targets = report.blame_targets();
    let total = targets.len();
    let last_push = std::sync::atomic::AtomicU64::new(0);
    let progress = |_stage: scan::Stage, done: usize, _total: usize, _path: &Path| {
        let now = millis_now();
        if done >= total || now.saturating_sub(last_push.load(Ordering::Relaxed)) > 60 {
            last_push.store(now, Ordering::Relaxed);
            authors_tx.send_replace(AuthorsStatus::Blaming { done, total });
        }
    };
    let options = report.options.clone();
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        authors::Authors::collect_targets(&targets, &options, Some(&progress))
    }));
    match outcome {
        Ok(blame) => blame,
        Err(_) => authors::Authors::default(),
    }
}

/// Write the language catalogs as JSON files the web page fetches.
pub fn dump_i18n(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let mut zh = serde_json::Map::new();
    let mut en = serde_json::Map::new();
    for (key, english, chinese) in crate::i18n::Strings::pairs() {
        zh.insert(
            key.to_string(),
            serde_json::Value::String(chinese.to_string()),
        );
        en.insert(
            key.to_string(),
            serde_json::Value::String(english.to_string()),
        );
    }
    for (name, map) in [("zh.json", zh), ("en.json", en)] {
        let text = serde_json::to_string_pretty(&map).unwrap_or_default();
        std::fs::write(dir.join(name), text).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn router(state: Arc<AppState>) -> axum::Router {
    use axum::routing::{get, post};
    use axum::Router;

    let api = Router::new()
        .route("/config", get(api::config_get).put(api::config_put))
        .route("/scan", post(api::scan_post))
        .route("/status", get(api::status_get))
        .route("/events", get(api::events))
        .route("/report", get(api::report_get))
        .route("/report/authors", get(api::authors_get).post(api::authors_post))
        .route("/export", get(api::export_get))
        .route("/fs/list", get(api::fs_list))
        .route("/fs/drives", get(api::fs_drives))
        .route("/languages", get(api::languages_get))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            api::auth_guard,
        ))
        .with_state(state.clone());
    Router::new()
        .nest("/api", api)
        .layer(axum::middleware::from_fn(api::host_guard))
        .fallback(assets::static_fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn settings_become_cli_flags() {
        let mut s = settings();
        s.roots = "B:/proj".into();
        s.uloc = true;
        s.authors = true;
        s.exclude = "vendor, target/gen".into();
        s.sources = "rust,python".into();
        let args = build_scan_args(&s, &scan_roots(&s, &[]), &[]);
        let joined = args.join(" ");
        assert!(joined.starts_with("B:/proj"));
        assert!(joined.contains("--uloc"));
        assert!(!joined.contains("--authors"));
        assert!(joined.contains("--exclude vendor"));
        assert!(joined.contains("--exclude target/gen"));
        assert!(joined.contains("--sources rust,python"));
        assert!(joined.contains("--functions"));
    }

    #[test]
    fn roots_fall_back_to_the_saved_ones_then_cwd() {
        let mut s = settings();
        s.roots = "B:/one;B:/two".into();
        assert_eq!(
            scan_roots(&s, &[]),
            vec!["B:/one".to_string(), "B:/two".to_string()]
        );
        assert_eq!(scan_roots(&s, &["C:/three".into()]), vec!["C:/three"]);
        s.roots = String::new();
        assert_eq!(scan_roots(&s, &[]).len(), 1);
    }

    #[test]
    fn web_json_round_trips() {
        write_web_json(9999, "abc");
        assert_eq!(read_web_json(), Some((9999, "abc".to_string())));
    }
}
