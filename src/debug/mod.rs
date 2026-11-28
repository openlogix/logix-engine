//! The in-design debugger (docs/debugger.md): a build with debug_file inserts it into a
//! design, and the session tools talk to it on the board.

pub mod build;
pub mod config;
pub mod lock;
pub mod panel;
pub mod protocol;
pub mod session;
