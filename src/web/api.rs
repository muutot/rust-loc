//! The HTTP surface: one JSON API under `/api` and the embedded page under
//! everything else.
//!
//! Two guards run before any handler: the host guard refuses any request that
//! does not name the loopback interface (which kills DNS rebinding outright),
//! and the auth guard requires the per-run token on every `/api` call.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response, Sse};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;

use tokio::sync::watch;

use crate::web::dto::{self, ReportDto};
use crate::web::{AppState, AuthorsStatus, ScanStatus};

/// Query parameters parsed by hand in the guards: the middleware runs before
/// any extractor, and the token must be accepted from both the header a fetch
/// sends and the query an EventSource can send.
fn query_token(query: Option<&str>) -> Option<String> {
    for pair in query?.split('&') {
        // Skip malformed pairs instead of failing the whole query: a stray
        // trailing '&' must not invalidate a valid token.
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name == "token" {
            return Some(value.to_string());
        }
    }
    None
}

/// Refuse anything that does not name the loopback host. A browser page from
/// any other origin resolves its DNS names to other addresses, so a request
/// with a foreign Host header is a rebinding attempt, not a reader.
pub async fn host_guard(request: Request<axum::body::Body>, next: Next) -> Response {
    let host_ok = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| {
            // A bracketed IPv6 host keeps its colons; anything else carries a
            // port after the first colon. Compare case-insensitively.
            let bare = host.trim_start_matches('[');
            let bare = match bare.split_once(']') {
                Some((ipv6, _)) => ipv6,
                None => bare.split(':').next().unwrap_or(bare),
            };
            bare.eq_ignore_ascii_case("127.0.0.1")
                || bare.eq_ignore_ascii_case("localhost")
                || bare == "::1"
        })
        .unwrap_or(false);
    if host_ok {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "loopback only").into_response()
    }
}

/// Require the per-run token on every API call, by header or by query.
pub async fn auth_guard(
    State(state): State<Arc<AppState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let header_token = request
        .headers()
        .get("x-auth-token")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let given = header_token.or_else(|| query_token(request.uri().query()));
    if given.as_deref() == Some(state.token.as_str()) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "bad token").into_response()
    }
}

// -- settings --------------------------------------------------------------

/// The settings the web page edits. Scan-relevant only: the GUI's window
/// rectangle and tab state stay in the shared INI untouched.
#[derive(Deserialize, Serialize)]
pub struct ConfigDto {
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default = "yes")]
    pub gitignore: bool,
    #[serde(default = "yes")]
    pub default_ignores: bool,
    #[serde(default)]
    pub include_tests: bool,
    #[serde(default = "yes")]
    pub complexity: bool,
    #[serde(default)]
    pub uloc: bool,
    #[serde(default)]
    pub exclude_generated: bool,
    #[serde(default)]
    pub cocomo: bool,
    #[serde(default)]
    pub authors: bool,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub extra_dirs: Vec<String>,
    #[serde(default)]
    pub exclude_ext: Vec<String>,
    /// `--fail-if` gate specs, one per entry: `code>120000` fails when the
    /// code line count exceeds 120,000.
    #[serde(default)]
    pub fail_if: Vec<String>,
    #[serde(default)]
    pub max_file_size: u64,
    #[serde(default)]
    pub sources: Vec<String>,
    /// 0 = Chinese, 1 = English, the index the shared INI stores.
    #[serde(default)]
    pub lang: usize,
}

fn yes() -> bool {
    true
}

impl ConfigDto {
    fn of(settings: &crate::settings::Settings) -> ConfigDto {
        ConfigDto {
            roots: crate::settings::parse_paths(&settings.roots),
            gitignore: settings.gitignore,
            default_ignores: settings.default_ignores,
            include_tests: settings.include_tests,
            complexity: settings.complexity,
            uloc: settings.uloc,
            exclude_generated: settings.exclude_generated,
            cocomo: settings.cocomo,
            authors: settings.authors,
            exclude: crate::settings::parse_list(&settings.exclude),
            extra_dirs: settings.extra_dirs.clone(),
            exclude_ext: settings
                .exclude_ext
                .split(',')
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect(),
            // A gate spec may carry spaces ("complexity >= 5000"), so only
            // commas and semicolons separate entries.
            fail_if: settings
                .fail_if
                .split([',', ';', '\r', '\n'])
                .map(|spec| spec.trim().to_string())
                .filter(|spec| !spec.is_empty())
                .collect(),
            max_file_size: settings.max_file_size.trim().parse().unwrap_or(0),
            sources: crate::settings::sources_of(&settings.sources)
                .slug()
                .split(',')
                .map(str::to_string)
                .filter(|slug| !slug.is_empty())
                .collect(),
            lang: settings.lang,
        }
    }

    fn apply(&self, settings: &mut crate::settings::Settings) {
        settings.roots = self.roots.join(";");
        settings.gitignore = self.gitignore;
        settings.default_ignores = self.default_ignores;
        settings.include_tests = self.include_tests;
        settings.complexity = self.complexity;
        settings.uloc = self.uloc;
        settings.exclude_generated = self.exclude_generated;
        settings.cocomo = self.cocomo;
        settings.authors = self.authors;
        settings.exclude = self.exclude.join(",");
        settings.extra_dirs = self.extra_dirs.clone();
        settings.exclude_ext = self
            .exclude_ext
            .iter()
            .map(|name| name.trim().trim_start_matches('.').to_string())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        settings.fail_if = self
            .fail_if
            .iter()
            .map(|spec| spec.trim().to_string())
            .filter(|spec| !spec.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        settings.max_file_size = self.max_file_size.to_string();
        settings.sources = crate::settings::sources_slugs(
            crate::sources::Sources::parse(&self.sources.join(","))
                .unwrap_or(crate::sources::Sources::RUST),
        );
        settings.lang = self.lang;
    }
}

pub async fn config_get(State(state): State<Arc<AppState>>) -> Json<ConfigDto> {
    let settings = state.settings.lock().unwrap().clone();
    Json(ConfigDto::of(&settings))
}

pub async fn config_put(
    State(state): State<Arc<AppState>>,
    Json(config): Json<ConfigDto>,
) -> Json<ConfigDto> {
    let mut settings = state.settings.lock().unwrap().clone();
    config.apply(&mut settings);
    crate::settings::save_settings(&settings);
    *state.settings.lock().unwrap() = settings;
    config_get(State(state)).await
}

// -- scanning --------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct ScanRequest {
    /// Roots for this scan only; the saved ones stay untouched.
    #[serde(default)]
    pub roots: Vec<String>,
    /// Extra CLI-style flags for this scan only, e.g. `--hotspots`.
    #[serde(default)]
    pub args: Vec<String>,
}

pub async fn scan_post(
    State(state): State<Arc<AppState>>,
    body: Option<Json<ScanRequest>>,
) -> Response {
    let request = body.map(|Json(inner)| inner).unwrap_or_default();
    let roots = crate::web::scan_roots(&state.settings.lock().unwrap(), &request.roots);
    if roots.is_empty() {
        return (StatusCode::BAD_REQUEST, "no scan roots").into_response();
    }
    let args =
        crate::web::build_scan_args(&state.settings.lock().unwrap(), &roots, &request.args);
    // The flag flips before the task starts and is cleared when it ends, so a
    // second request while one runs is refused, not queued.
    if state
        .scanning
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return (
            StatusCode::CONFLICT,
            "a scan is already running",
        )
            .into_response();
    }
    state.scan.send_replace(ScanStatus::Scanning {
        stage: "files",
        done: 0,
        total: 0,
        path: String::new(),
    });
    *state.blame.lock().unwrap() = None;
    state.authors.send_replace(crate::web::AuthorsStatus::Idle);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    tokio::task::spawn_blocking(move || {
        let outcome = crate::web::run_scan_blocking(state.scan.clone(), args, cwd);
        match outcome {
            Ok(result) => {
                // A scan that ran with authors carries the blame in the
                // report; publish it so the author endpoints serve it. The
                // report keeps its copy: the export renderers read authors
                // from the stored report, so taking would strip the section.
                if let Some(authors) = &result.report.authors {
                    *state.blame.lock().unwrap() = Some(Arc::new(authors.clone()));
                    state
                        .authors
                        .send_replace(crate::web::AuthorsStatus::Ready);
                }
                // The report must be visible before Done is announced: a
                // client reacting to Done reads it immediately.
                let elapsed = result.elapsed;
                *state.result.lock().unwrap() = Some(Arc::new(result));
                state.scan.send_replace(ScanStatus::Done { elapsed });
            }
            Err(error) => {
                state.scan.send_replace(ScanStatus::Failed { error });
            }
        }
        state
            .scanning
            .store(false, std::sync::atomic::Ordering::SeqCst);
    });
    (StatusCode::ACCEPTED, "scan started").into_response()
}

// -- status ----------------------------------------------------------------

pub async fn status_get(State(state): State<Arc<AppState>>) -> Json<crate::web::StatusSnapshot> {
    Json(state.snapshot())
}

/// The progress stream: one JSON snapshot per state change, then a comment
/// heartbeat so a quiet network path does not close the connection.
pub async fn events(State(state): State<Arc<AppState>>) -> Response {
    use axum::response::sse::{Event, KeepAlive};

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<Event, std::convert::Infallible>,
    >(32);
    let mut scan_rx = state.scan.subscribe();
    let mut authors_rx = state.authors.subscribe();
    tokio::spawn(async move {
        // One closure reads both receivers; the mutable borrows that await
        // changes end before it runs, so the two never overlap.
        let push = |scan_rx: &mut watch::Receiver<ScanStatus>,
                        authors_rx: &mut watch::Receiver<AuthorsStatus>| {
            let snapshot = crate::web::StatusSnapshot {
                scan: scan_rx.borrow_and_update().clone(),
                authors: authors_rx.borrow_and_update().clone(),
            };
            let data = serde_json::to_string(&snapshot).unwrap_or_default();
            tx.try_send(Ok(Event::default().data(data)))
                .is_ok()
        };
        push(&mut scan_rx, &mut authors_rx);
        loop {
            tokio::select! {
                changed = scan_rx.changed() => {
                    if changed.is_err() || !push(&mut scan_rx, &mut authors_rx) {
                        break;
                    }
                }
                changed = authors_rx.changed() => {
                    if changed.is_err() || !push(&mut scan_rx, &mut authors_rx) {
                        break;
                    }
                }
            }
        }
    });
    Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

// -- results ---------------------------------------------------------------

pub async fn report_get(State(state): State<Arc<AppState>>) -> Response {
    let result = state.result.lock().unwrap().clone();
    match result {
        Some(result) => Json(ReportDto::of(&result)).into_response(),
        None => (StatusCode::NOT_FOUND, "no scan yet").into_response(),
    }
}

pub async fn authors_get(State(state): State<Arc<AppState>>) -> Response {
    let blame = state.blame.lock().unwrap().clone();
    let Some(blame) = blame else {
        return (StatusCode::NOT_FOUND, "no blame yet").into_response();
    };
    // The month grid comes from the scan, not the blame pass; either being
    // absent just drops the grid from the payload.
    let activity = {
        let result = state.result.lock().unwrap();
        result
            .as_ref()
            .and_then(|scan| scan.report.activity.as_ref())
            .cloned()
    };
    Json(dto::AuthorsDto::of(&blame, activity.as_ref())).into_response()
}

/// Start the blame pass; the page polls GET /report/authors until it lands.
pub async fn authors_post(State(state): State<Arc<AppState>>) -> Response {
    let report = state.result.lock().unwrap().clone();
    let Some(result) = report else {
        return (StatusCode::NOT_FOUND, "no scan yet").into_response();
    };
    if state.blame_running.swap(true, Ordering::SeqCst) {
        return (StatusCode::CONFLICT, "blame is already running").into_response();
    }
    state
        .authors
        .send_replace(crate::web::AuthorsStatus::Blaming { done: 0, total: 0 });
    tokio::task::spawn_blocking(move || {
        let blame = crate::web::run_authors_blocking(state.authors.clone(), &result.report);
        *state.blame.lock().unwrap() = Some(Arc::new(blame));
        state.blame_running.store(false, Ordering::SeqCst);
        state.authors.send_replace(crate::web::AuthorsStatus::Ready);
    });
    (StatusCode::ACCEPTED, "blame started").into_response()
}

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
}

/// Re-render the stored report in one of the CLI formats and hand it over as
/// a download. The bytes come from the same renderers the CLI prints with.
pub async fn export_get(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ExportQuery>,
) -> Response {
    let format_name = query.format.as_deref().unwrap_or("text");
    let Some(format) = dto::export_format(format_name) else {
        return (StatusCode::BAD_REQUEST, "unknown format").into_response();
    };
    let result = state.result.lock().unwrap().clone();
    let Some(result) = result else {
        return (StatusCode::NOT_FOUND, "no scan yet").into_response();
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let body = result.report.render_as(format, &cwd);
    let (content_type, filename) = dto::export_content_type(format);
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    response
}

// -- file picker -----------------------------------------------------------

#[derive(Deserialize)]
pub struct FsQuery {
    pub path: Option<String>,
}

#[derive(Serialize)]
struct DirEntryDto {
    name: String,
    path: String,
}

#[derive(Serialize)]
struct DirListingDto {
    path: String,
    parent: Option<String>,
    entries: Vec<DirEntryDto>,
}

/// List the directories below `path`, nothing else: the picker needs to walk
/// the tree, never to read a file. Sorted, with hidden names kept (a picker
/// that hides `.git` hides where projects live).
pub async fn fs_list(Query(query): Query<FsQuery>) -> Response {
    let raw = query.path.unwrap_or_default();
    let path = std::path::PathBuf::from(&raw);
    if raw.is_empty() || !path.is_dir() {
        return (StatusCode::BAD_REQUEST, "not a directory").into_response();
    }
    let Ok(read) = std::fs::read_dir(&path) else {
        return (StatusCode::FORBIDDEN, "unreadable directory").into_response();
    };
    let mut entries: Vec<DirEntryDto> = Vec::new();
    for entry in read.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            entries.push(DirEntryDto {
                path: entry.path().to_string_lossy().into_owned(),
                name: entry.file_name().to_string_lossy().into_owned(),
            });
        }
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().into_owned());
    Json(DirListingDto {
        path: path.to_string_lossy().into_owned(),
        parent,
        entries,
    })
    .into_response()
}

/// The drive letters a Windows picker starts from; other systems start at /.
pub async fn fs_drives() -> Json<Vec<String>> {
    #[cfg(windows)]
    {
        let mut drives = Vec::new();
        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            if std::path::Path::new(&root).is_dir() {
                drives.push(root);
            }
        }
        Json(drives)
    }
    #[cfg(not(windows))]
    {
        Json(vec!["/".to_string()])
    }
}

// -- languages -------------------------------------------------------------

#[derive(Serialize)]
pub struct LangDto {
    slug: String,
    name: &'static str,
    extension: &'static str,
}

/// The languages the settings page ticks, in the order the table draws them.
pub async fn languages_get() -> Json<Vec<LangDto>> {
    Json(crate::settings::SOURCE_LANGS
        .iter()
        .map(|lang| LangDto {
            slug: lang.slug().to_string(),
            name: lang.name(),
            extension: lang.extension(),
        })
        .collect())
}
