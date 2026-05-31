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
//!
//! ## Central audit sink (feature `sink`, default on)
//!
//! When the `sink` feature is enabled (the default), every log/error/warn/
//! status/etc. call ALSO appends one NDJSON record to a local spool file and a
//! background thread gzips batches and POSTs them to a central `/ingest`
//! endpoint. Configuration is entirely environment-driven:
//!
//! - `DEKA_LOG_SINK`         host or URL of the ingest endpoint (e.g. `logs.tana.gg`)
//! - `DEKA_LOG_TOKEN`        bearer token sent as `Authorization: Bearer ...`
//! - `DEKA_LOG_SPOOL`        spool file path (default `/var/log/deka/spool`)
//! - `DEKA_LOG_STDOUT`       set to `1` to also print to stderr (opt-in)
//! - `DEKA_LOG_FLUSH_SECS`   background flush interval, seconds (default 5)
//! - `DEKA_LOG_SPOOL_CAP_MB` spool size cap in MB (default 64, oldest dropped)
//!
//! If `DEKA_LOG_SINK` is unset, output falls back to stderr so local dev still
//! works. Disable the sink entirely (zero dependencies) with
//! `--no-default-features`.

use std::env;
use std::sync::OnceLock;

#[cfg(feature = "sink")]
mod sink;

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
// Emit core — fans a single call out to the audit sink (if the
// `sink` feature is enabled) and to stderr (when stdout is enabled).
// ============================================================

/// Emit a structured record. `display` is the human-formatted line printed to
/// stderr; the (level, component, action, msg) tuple is what lands in the
/// NDJSON spool. When the `sink` feature is off this just prints `display`.
fn emit_structured(level: &str, component: &str, action: &str, msg: &str, display: &str) {
    #[cfg(feature = "sink")]
    {
        let logger = sink::logger();
        logger.append(level, component, action, msg);
        sink::maybe_flush_for_threshold(&logger);
        if logger.stdout_enabled() {
            eprintln!("{}", display);
        }
    }

    #[cfg(not(feature = "sink"))]
    {
        let _ = (level, component, action, msg);
        eprintln!("{}", display);
    }
}

/// Emit a raw (already-formatted) line with no structured metadata beyond the
/// default component/action.
fn emit_line(line: &str) {
    emit_structured("info", "stdio", "raw", line, line);
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
        emit_structured(
            "info",
            action,
            action,
            message,
            &format!("[{}] {}", action, message),
        );
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
    emit_structured(
        "error",
        action,
        action,
        message,
        &format!("[{}] {}", action, message),
    );
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
    emit_structured(
        "warn",
        name,
        "warn",
        message,
        &format!("[warn] [{}] {}", name, message),
    );
}

/// Log a simple warning without component name
/// Format: `[warn] message`
pub fn warn_simple(message: &str) {
    emit_structured(
        "warn",
        "stdio",
        "warn",
        message,
        &format!("[warn] {}", message),
    );
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
        emit_structured(
            "status",
            name,
            "ok",
            message,
            &format!("[ok] [{}] {}", name, message),
        );
    } else {
        emit_structured(
            "error",
            name,
            "fail",
            message,
            &format!("[fail] [{}] {}", name, message),
        );
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
    emit_line("");
    emit_line(title);
    emit_line(&"-".repeat(40));
}

/// Print a blank line
pub fn blank() {
    emit_line("");
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
    emit_structured(
        "status",
        "stdio",
        "ok",
        message,
        &format!("[ok] {}", message),
    );
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
    emit_structured(
        "error",
        "stdio",
        "fail",
        message,
        &format!("[fail] {}", message),
    );
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
    emit_structured(
        "info",
        label,
        "info",
        value,
        &format!("  {:<10} {}", label, value),
    );
}

/// Hint in subdued format
/// Format: `  message`
pub fn hint(message: &str) {
    emit_line(&format!("  {}", message));
}

/// Detail line with arrow
/// Format: `    -> message`
pub fn detail(message: &str) {
    emit_line(&format!("    -> {}", message));
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
    emit_line(&format!("  -> {}: {}", description, command));
}

/// Diagnostic warning
/// Format: `[warn] [component] message`
pub fn diagnostic(component: &str, message: &str) {
    emit_structured(
        "warn",
        component,
        "diagnostic",
        message,
        &format!("[warn] [{}] {}", component, message),
    );
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
        emit_structured(
            "debug",
            action,
            action,
            message,
            &format!("[{}] {}", action, message),
        );
    }
}

/// Print a raw line (no extra formatting).
pub fn raw(message: &str) {
    emit_line(message);
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
            $crate::log($action, &format!($($arg)*));
        }
    };
}

/// Error with format string support
#[macro_export]
macro_rules! errorf {
    ($action:expr, $($arg:tt)*) => {
        $crate::error($action, &format!($($arg)*));
    };
}

/// Debug with format string support (only shown when LOG_LEVEL=debug)
#[macro_export]
macro_rules! debugf {
    ($action:expr, $($arg:tt)*) => {
        if $crate::log_level() >= $crate::LogLevel::Debug {
            $crate::debug($action, &format!($($arg)*));
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
