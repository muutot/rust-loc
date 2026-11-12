//! rust-loc-web: the local web front end.
//!
//! Starts the analysis server on the loopback interface, puts an icon in the
//! system tray, and opens the page in the browser on demand. A second launch
//! does not start a second server — it opens the first one's page instead.
//!
//! Flags:
//!   --port <N>     first port to try (default 8422, then walks upward)
//!   --no-tray      run in the foreground, print the URL, exit on Ctrl-C
//!   --dump-i18n <dir>  write the zh/en string catalogs as JSON and exit
//!   --version      print the version

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use rust_loc::web::{self, ServerOptions};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let mut port = web::DEFAULT_PORT;
    let mut no_tray = false;
    let mut dump_dir: Option<PathBuf> = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--port" => {
                port = arguments.next().and_then(|v| v.parse().ok()).unwrap_or(port);
            }
            "--no-tray" => no_tray = true,
            "--dump-i18n" => dump_dir = arguments.next().map(PathBuf::from),
            "--version" => {
                println!("rust-loc-web {}", rust_loc::VERSION);
                return;
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            other => {
                eprintln!("unknown flag: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }

    if let Some(dir) = dump_dir {
        match web::dump_i18n(&dir) {
            Ok(()) => println!("wrote zh.json and en.json to {}", dir.display()),
            Err(error) => {
                eprintln!("dump-i18n failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    // One server per machine: a second launch wakes the first one's page.
    #[cfg(windows)]
    {
        if let Some(url) = already_running() {
            web::open_url(&url);
            return;
        }
        let _guard = single_instance_lock();
        if no_tray {
            run_foreground(port);
        } else {
            run_with_tray(port);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = no_tray;
        run_foreground(port);
    }
}

fn print_help() {
    println!(
        "rust-loc-web {} — code statistics in the browser\n\n\
         usage: rust-loc-web [--port N] [--no-tray]\n\n\
         flags:\n  \
         --port <N>        first port to try [default {}]\n  \
         --no-tray         run in the foreground, print the URL, Ctrl-C exits\n  \
         --dump-i18n <dir> write the zh/en string catalogs as JSON\n  \
         -h, --help        this text\n  \
         -V, --version     version",
        rust_loc::VERSION,
        web::DEFAULT_PORT,
    );
}

#[cfg(windows)]
fn already_running() -> Option<String> {
    let (port, token) = web::read_web_json()?;
    // The recorded server may have died since; a quick connect tells.
    let alive = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
    alive.then(|| format!("http://127.0.0.1:{port}/?token={token}"))
}

#[cfg(windows)]
fn single_instance_lock() -> windows_sys::Win32::Foundation::HANDLE {
    use windows_sys::Win32::System::Threading::CreateMutexW;
    unsafe {
        let name = web::to_wide("Local\\rust-loc-web");
        CreateMutexW(std::ptr::null(), 0, name.as_ptr())
    }
}

/// Tray mode: the server runs on its own thread; the main thread pumps the
/// tray messages until the reader quits from the menu.
#[cfg(windows)]
fn run_with_tray(port: u16) {
    let token = web::generate_token();
    let (port_tx, port_rx) = mpsc::channel();
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let server = web::start_server(ServerOptions {
        first_port: port,
        token: token.clone(),
        port_tx,
        shutdown: shutdown.clone(),
    });
    match port_rx.recv() {
        Ok(Ok(port)) => {
            let code = web::tray::run_with_tray(port, &token);
            shutdown.notify_waiters();
            let _ = server.join();
            std::process::exit(code);
        }
        Ok(Err(error)) => {
            eprintln!("rust-loc-web: {error}");
            std::process::exit(1);
        }
        Err(_) => std::process::exit(1),
    }
}

/// Foreground mode: bind, print the URL, wait for Ctrl-C.
fn run_foreground(port: u16) {
    let token = web::generate_token();
    let (port_tx, port_rx) = mpsc::channel();
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("rust-loc-web: tokio runtime: {error}");
            std::process::exit(1);
        }
    };
    let url_token = token.clone();
    let result = runtime.block_on(web::serve_forever(
        ServerOptions {
            first_port: port,
            token,
            port_tx,
            shutdown,
        },
        &url_token,
    ));
    match result {
        Ok(_) => {}
        Err(_) => {
            // The port error already went through the channel in detail.
            if let Ok(Err(message)) = port_rx.try_recv() {
                eprintln!("rust-loc-web: {message}");
            }
            std::process::exit(1);
        }
    }
}
