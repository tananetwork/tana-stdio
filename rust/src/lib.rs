//! # tana-stdio
//!
//! Terminal output utilities for Tana projects.
//! Consistent formatting across CLI, services, and tools.
//!
//! ## Format
//!
//! ```text
//! [action] message
//! ```
//!
//! ## Usage
//!
//! ```rust
//! use tana_stdio::{log, error, warn, success, fail};
//!
//! log("build", "compiling contract...");
//! success("build complete");
//! error("build", "compilation failed");
//! ```
//!
//! ## Log Levels
//!
//! Control output with `LOG_LEVEL` environment variable:
//! - `error` - Errors only
//! - `info` - Default (startup + important messages)
//! - `debug` - Verbose output

use std::env;
use std::sync::OnceLock;

/// Log level for tana services
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LogLevel {
    Error = 0,
    Info = 1,
    Debug = 2,
}

impl LogLevel {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "error" => LogLevel::Error,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Info,
        }
    }
}

static LOG_LEVEL: OnceLock<LogLevel> = OnceLock::new();

/// Get the current log level (cached from LOG_LEVEL env var)
pub fn log_level() -> LogLevel {
    *LOG_LEVEL.get_or_init(|| {
        env::var("LOG_LEVEL")
            .map(|s| LogLevel::from_str(&s))
            .unwrap_or(LogLevel::Info)
    })
}

/// Check if debug logging is enabled
pub fn is_debug() -> bool {
    log_level() >= LogLevel::Debug
}

/// Check if info logging is enabled
pub fn is_info() -> bool {
    log_level() >= LogLevel::Info
}

// ============================================================
// Core logging functions (match @tananetwork/stdio API)
// ============================================================

/// Log an action with a message
/// Format: `[action] message`
///
/// # Example
/// ```
/// tana_stdio::log("build", "compiling contract...");
/// // Output: [build] compiling contract...
/// ```
pub fn log(action: &str, message: &str) {
    if log_level() >= LogLevel::Info {
        eprintln!("[{}] {}", action, message);
    }
}

/// Log an error
/// Format: `[action] message`
///
/// # Example
/// ```
/// tana_stdio::error("build", "compilation failed");
/// // Output: [build] compilation failed
/// ```
pub fn error(action: &str, message: &str) {
    eprintln!("[{}] {}", action, message);
}

/// Log a warning
/// Format: `[warn] message` or `[name] message`
///
/// # Example
/// ```
/// tana_stdio::warn("cache", "stale entries detected");
/// // Output: [warn] [cache] stale entries detected
/// ```
pub fn warn(name: &str, message: &str) {
    eprintln!("[warn] [{}] {}", name, message);
}

/// Log a simple warning without component name
/// Format: `[warn] message`
pub fn warn_simple(message: &str) {
    eprintln!("[warn] {}", message);
}

/// Log a status line with success/failure indicator
/// Format: `[ok] message` or `[fail] message`
///
/// # Example
/// ```
/// tana_stdio::status("database", "connected", true);
/// // Output: [ok] [database] connected
/// ```
pub fn status(name: &str, message: &str, ok: bool) {
    if ok {
        eprintln!("[ok] [{}] {}", name, message);
    } else {
        eprintln!("[fail] [{}] {}", name, message);
    }
}

/// Print a section header
///
/// # Example
/// ```
/// tana_stdio::header("configuration");
/// // Output:
/// //
/// // configuration
/// // ----------------------------------------
/// ```
pub fn header(title: &str) {
    eprintln!();
    eprintln!("{}", title);
    eprintln!("{}", "-".repeat(40));
}

/// Print a blank line
pub fn blank() {
    eprintln!();
}

/// Success message
/// Format: `[ok] message`
///
/// # Example
/// ```
/// tana_stdio::success("build complete");
/// // Output: [ok] build complete
/// ```
pub fn success(message: &str) {
    eprintln!("[ok] {}", message);
}

/// Failure message
/// Format: `[fail] message`
///
/// # Example
/// ```
/// tana_stdio::fail("build failed");
/// // Output: [fail] build failed
/// ```
pub fn fail(message: &str) {
    eprintln!("[fail] {}", message);
}

/// Info line with label
/// Format: `  label     value`
///
/// # Example
/// ```
/// tana_stdio::info("port", "8506");
/// // Output:   port       8506
/// ```
pub fn info(label: &str, value: &str) {
    eprintln!("  {:<10} {}", label, value);
}

/// Hint in subdued format
/// Format: `  message`
pub fn hint(message: &str) {
    eprintln!("  {}", message);
}

/// Detail line with arrow
/// Format: `    -> message`
pub fn detail(message: &str) {
    eprintln!("    -> {}", message);
}

/// Suggest a next step
/// Format: `  -> description: command`
///
/// # Example
/// ```
/// tana_stdio::next_step("start the server", "npm run dev");
/// // Output:   -> start the server: npm run dev
/// ```
pub fn next_step(description: &str, command: &str) {
    eprintln!("  -> {}: {}", description, command);
}

/// Diagnostic warning
/// Format: `[warn] [component] message`
pub fn diagnostic(component: &str, message: &str) {
    eprintln!("[warn] [{}] {}", component, message);
}

// ============================================================
// Debug-level logging (only shown when LOG_LEVEL=debug)
// ============================================================

/// Debug log (only shown when LOG_LEVEL=debug)
///
/// # Example
/// ```
/// tana_stdio::debug("cache", "hit for key: user_123");
/// // Output (only if LOG_LEVEL=debug): [cache] hit for key: user_123
/// ```
pub fn debug(action: &str, message: &str) {
    if log_level() >= LogLevel::Debug {
        eprintln!("[{}] {}", action, message);
    }
}

// ============================================================
// Macros for convenient formatting
// ============================================================

/// Log with format string support
///
/// # Example
/// ```
/// tana_stdio::logf!("build", "compiled {} files in {}ms", 42, 150);
/// ```
#[macro_export]
macro_rules! logf {
    ($action:expr, $($arg:tt)*) => {
        if $crate::log_level() >= $crate::LogLevel::Info {
            eprintln!(concat!("[", $action, "] {}"), format!($($arg)*));
        }
    };
}

/// Error with format string support
#[macro_export]
macro_rules! errorf {
    ($action:expr, $($arg:tt)*) => {
        eprintln!(concat!("[", $action, "] {}"), format!($($arg)*));
    };
}

/// Debug with format string support (only shown when LOG_LEVEL=debug)
#[macro_export]
macro_rules! debugf {
    ($action:expr, $($arg:tt)*) => {
        if $crate::log_level() >= $crate::LogLevel::Debug {
            eprintln!(concat!("[", $action, "] {}"), format!($($arg)*));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_parsing() {
        assert_eq!(LogLevel::from_str("error"), LogLevel::Error);
        assert_eq!(LogLevel::from_str("info"), LogLevel::Info);
        assert_eq!(LogLevel::from_str("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::from_str("INFO"), LogLevel::Info);
        assert_eq!(LogLevel::from_str("unknown"), LogLevel::Info);
    }

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Error < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
    }
}
