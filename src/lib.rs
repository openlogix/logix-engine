//! Logix Engine library: everything except the command line, which is in
//! main.rs. See docs/ for what the engine does.

// ToolError is large, but tool errors are rare and returned once per call,
// so boxing it would only add noise.
#![allow(clippy::result_large_err)]

pub mod board;
pub mod build;
pub mod catalog;
pub mod commands;
pub mod custom_boards;
pub mod devices;
pub mod diagram;
pub mod display_paths;
pub mod drivers;
pub mod errors;
pub mod format;
pub mod json_pos;
pub mod logging;
pub mod mcp_progress;
pub mod output_rules;
pub mod packages;
pub mod paths;
pub mod program;
pub mod progress;
pub mod resources;
pub mod runner;
pub mod server;
pub mod sim;
pub mod state;
pub mod text;
pub mod tools;
