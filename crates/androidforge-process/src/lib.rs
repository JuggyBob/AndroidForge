//! Process execution primitives shared by every AndroidForge integration.
//!
//! The important properties of this crate are:
//!
//! * arguments are passed as a vector, never concatenated into a shell command;
//! * output is captured as bytes and converted to text without the Windows
//!   console code page ever being involved;
//! * every foreground command has a timeout;
//! * timeouts terminate the process tree on Windows, not only the direct child;
//! * output capture is bounded so a chatty device command cannot exhaust memory.

mod detached;
mod encoding;
mod process_util;
mod runner;
mod shell;

pub use detached::{DetachedSpec, spawn_detached};
pub use encoding::{decode_lossless, decode_text, looks_like_text};
pub use process_util::{kill_tree, process_exists};
pub use runner::{CommandOutput, CommandSpec, ProcessError, ProcessResult, Runner};
pub use shell::{escape_input_text, shell_quote, shell_quote_join};

/// Exit code used when a process could not be spawned at all.
pub const SPAWN_FAILURE_EXIT_CODE: i32 = 127;
