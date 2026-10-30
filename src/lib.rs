//! Logix Engine library: everything except the command line, which is in
//! main.rs. See docs/ for what the engine does.

// ToolError is large, but tool errors are rare and returned once per call,
// so boxing it would only add noise.
#![allow(clippy::result_large_err)]

pub mod errors;
pub mod logging;
pub mod paths;
pub mod server;
pub mod state;
pub mod text;
pub mod tools;
