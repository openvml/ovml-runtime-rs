use std::sync::Arc;
use std::time::Instant;

use ovml_runtime::config::Config;
use ovml_runtime::discovery::{find_free_port, remove_discovery, write_discovery};
use ovml_runtime::engine::RuntimeEngine;
use ovml_runtime::server::{create_router, AppState};
use ovml_runtime::VERSION;

const USAGE: &str = "\
ovml-runtime - device-side TTS runtime for OpenVML Studio

Usage: ovml-runtime [options]

Options:
  --host <host>            Bind host (default 127.0.0.1)
  --port <port>            Preferred port (default 47821)
  --token <token>          Pairing token (generated when omitted)
  --data-dir <path>        Data directory (default ~/.openvml/runtime)
  --concurrency <n>        Synthesis concurrency (default cores - 1)
  --cache-mb <n>           Audio cache limit in MB (default 512)
  --no-cache               Disable the audio cache
  --no-edge                Disable the Edge engine
  --piper-catalog <path>   Override the bundled Piper catalog
  -h, --help               Print this help
  --version                Print the version
";

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,ort=warn"))
        .init();

    init_onnx_runtime();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("ovml-runtime {VERSION}");
        return;
    }
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return;
    }

    let config = match Config::from_args(&args) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("ovml-runtime: {err}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(err) = run(config).await {
        eprintln!("ovml-runtime: {err}");
        std::process::exit(1);
    }
}

/// Load a bundled ONNX Runtime library next to the executable when the user
/// did not provide `ORT_DYLIB_PATH`.  This keeps release archives
/// self-contained: ship the binary together with `libonnxruntime`.
fn init_onnx_runtime() {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else {
        return;
    };
    let names: &[&str] = if cfg!(windows) {
        &["onnxruntime.dll", "libonnxruntime.dll"]
    } else if cfg!(target_os = "macos") {
        &["libonnxruntime.dylib"]
    } else {
        &["libonnxruntime.so", "libonnxruntime.so.1"]
    };
    for subdir in ["", "lib"] {
        let base = if subdir.is_empty() {
            dir.to_path_buf()
        } else {
            dir.join(subdir)
        };
        for name in names {
            let path = base.join(name);
            if !path.is_file() {
                continue;
            }
            match ort::init_from(&path) {
                Ok(builder) => {
                    builder.commit();
                    log::info!("ONNX Runtime loaded from {}", path.display());
                    return;
                }
                Err(err) => {
                    log::warn!("failed to load ONNX Runtime from {}: {err}", path.display())
                }
            }
        }
    }
}

async fn run(config: Config) -> Result<(), String> {
    let port = find_free_port(&config.host, config.port, 20).map_err(|err| err.to_string())?;
    let engine = Arc::new(RuntimeEngine::new(config.clone())?);
    let engines: Vec<String> = engine
        .engine_ids()
        .iter()
        .map(|id| id.to_string())
        .collect();
    let discovery = write_discovery(&config, port, engines.clone())
        .map_err(|err| format!("write discovery: {err}"))?;

    let state = AppState {
        engine: Arc::clone(&engine),
        token: config.token.clone(),
        started_at: Instant::now(),
    };
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind((config.host.as_str(), port))
        .await
        .map_err(|err| format!("bind {}:{}: {err}", config.host, port))?;
    log::info!(
        "ovml-runtime {VERSION} listening on {}:{} engines={:?} discovery={}",
        config.host,
        port,
        engines,
        discovery.display()
    );

    let serve = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());
    let result = serve.await.map_err(|err| err.to_string());
    remove_discovery(&config);
    result
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            stream.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    log::info!("shutting down");
}
