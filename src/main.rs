//! Logix Engine: FPGA operations as MCP tools over stdin/stdout.
//! The command line is described in docs/command-line.md.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, ValueEnum};
use rmcp::ServiceExt;

use logix_engine::packages::{PackageManager, manifest};
use logix_engine::state::{self, EngineState};
use logix_engine::{logging, paths, server};

/// Logix Engine: builds, tests and programs FPGAs. A host launches it as a
/// child process and talks MCP with it over stdin and stdout.
#[derive(Parser, Debug)]
#[command(name = "logix-engine", version = state::ENGINE_VERSION)]
struct Args {
    /// The host's engine folder, as an absolute path. Its last folder name
    /// must contain "engine", such as ~/.myhost/engine.
    #[arg(long, value_name = "absolute path")]
    engine_folder: String,

    /// Turn off automatic package install and repair.
    #[arg(long)]
    no_auto_install: bool,

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

    let packages = PackageManager::new(
        &resolved,
        Arc::new(manifest::builtin().clone()),
        &state::platform(),
        manifest::GITHUB,
    );
    let state = EngineState {
        engine_folder: args.engine_folder.clone(),
        engine_folder_resolved: resolved,
        packages,
        auto_install: !args.no_auto_install,
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
