//! Logix Engine: FPGA operations as MCP tools over stdin/stdout.
//! The command line is described in docs/command-line.md.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use rmcp::ServiceExt;

use logix_engine::packages::{PackageManager, manifest, remote_config};
use logix_engine::state::{self, EngineState};
use logix_engine::{catalog, logging, paths, server};

/// Logix Engine: builds and tests FPGA designs and programs boards. A host launches it as a
/// child process and talks MCP with it over stdin and stdout.
#[derive(Parser, Debug)]
#[command(name = "logix-engine", version = state::ENGINE_VERSION)]
struct Args {
    /// The host's engine folder, as an absolute path. Its last folder name
    /// must contain "engine", such as ~/.myhost/engine.
    #[arg(long, value_name = "absolute path")]
    engine_folder: String,

    /// Turn off automatic package download and repair.
    #[arg(long)]
    no_auto_download: bool,

    /// How much goes into the log file.
    #[arg(long, value_name = "level", default_value = "info")]
    log_level: LogLevel,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl From<LogLevel> for log::LevelFilter {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
        }
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("logix-engine: {message}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let command_line: Vec<String> = std::env::args().skip(1).collect();
    // The diagram's layout worker: the engine runs itself this way to lay
    // out diagrams in a process of its own (internal, not for hosts).
    if let [option, bundle] = command_line.as_slice()
        && option == logix_engine::diagram::layout::WORKER_OPTION
    {
        return logix_engine::diagram::layout::worker_main(Path::new(bundle));
    }
    // The driver helper: the engine runs itself this way, as an
    // administrator after Windows asks the user, to change a USB driver
    // (internal, not for hosts; src/elevated.rs).
    if let [option, request] = command_line.as_slice()
        && option == logix_engine::elevated::OPTION
    {
        return logix_engine::elevated::helper_main(Path::new(request));
    }
    let args = Args::parse();

    let resolved =
        match paths::check_engine_folder(&args.engine_folder, paths::home_folder().as_deref()) {
            Ok(p) => p,
            Err(e) => return fail(&e),
        };
    if let Err(e) = std::fs::create_dir_all(&resolved) {
        return fail(&format!(
            "can't create the engine folder {:?}: {e}",
            args.engine_folder
        ));
    }
    let log_path = match logging::init(&resolved, args.log_level.into()) {
        Ok(p) => p,
        Err(e) => return fail(&format!("can't create the log file: {e}")),
    };
    // Reported under the engine folder as the host gave it.
    let log_file = PathBuf::from(&args.engine_folder)
        .join("logs")
        .join(log_path.file_name().unwrap_or_default());

    // The packages come from the engine's remote config, or from the
    // diagnostic override: a version folder's URL, or a local folder that
    // must exist. The channel override must be a channel name.
    if let Err(e) = remote_config::channel_variable() {
        return fail(&e);
    }
    if let Ok(Some(channel)) = remote_config::channel_variable() {
        log::warn!(
            "Using the channel {channel} ({})",
            remote_config::CHANNEL_VAR
        );
    }
    let source = match remote_config::ConfigSource::from_env() {
        Some(o) => {
            let is_url = o.location.starts_with("https://") || o.location.starts_with("http://");
            if !is_url && !Path::new(&o.location).is_dir() {
                return fail(&format!(
                    "{}: no such folder: {}",
                    remote_config::DEV_REMOTE_CONFIG_VAR,
                    o.location
                ));
            }
            log::warn!(
                "Using the remote config {} ({})",
                o.location,
                remote_config::DEV_REMOTE_CONFIG_VAR
            );
            o
        }
        None => remote_config::ConfigSource::regular(),
    };
    let packages = PackageManager::with_remote_config(
        &resolved,
        source,
        !args.no_auto_download,
        &state::platform(),
        manifest::GITHUB,
    );
    // A catalog folder to read instead of the downloaded package: a
    // diagnostic setting. A bad value is an error, never ignored.
    if let Some(folder) = std::env::var_os(catalog::DEV_CATALOG_VAR).filter(|v| !v.is_empty()) {
        if let Err(e) = catalog::load(Path::new(&folder), "local", true) {
            return fail(&format!("{}: {e}", catalog::DEV_CATALOG_VAR));
        }
        log::warn!(
            "Using the board catalog in {} ({})",
            Path::new(&folder).display(),
            catalog::DEV_CATALOG_VAR
        );
    }
    let state = EngineState {
        engine_folder: args.engine_folder.clone(),
        engine_folder_resolved: resolved,
        packages,
        auto_download: !args.no_auto_download,
        command_line,
        log_file,
    };
    log::info!(
        "Logix Engine {} ({}) started, pid {}, arguments {:?}",
        state::ENGINE_VERSION,
        state::platform(),
        std::process::id(),
        state.command_line
    );

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => return fail(&format!("can't start: {e}")),
    };
    let code = runtime.block_on(async {
        let service = match server::EngineServer::new(state)
            .serve(rmcp::transport::stdio())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                log::error!("MCP session failed to start: {e}");
                return ExitCode::FAILURE;
            }
        };
        match service.waiting().await {
            Ok(reason) => {
                log::info!("MCP session ended: {reason:?}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                log::error!("MCP session failed: {e}");
                ExitCode::FAILURE
            }
        }
    });
    log::logger().flush();
    code
}
