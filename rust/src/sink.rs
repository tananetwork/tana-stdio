//! Central audit log sink for tana-stdio.
//!
//! Every formatting call also appends one NDJSON record to a local spool file.
//! A background thread (and a size-threshold trigger) gzips batches and POSTs
//! them to `https://$DEKA_LOG_SINK/ingest` with a bearer token. On 2xx the
//! flushed prefix is truncated from the spool; on failure lines are retained
//! and retried. The spool is capped at `DEKA_LOG_SPOOL_CAP_MB`, dropping the
//! oldest lines and logging a running drop-count. The caller is never blocked.
//!
//! This module is only compiled when the `sink` feature is enabled.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::{thread, time::Duration};

const DEFAULT_SPOOL_PATH: &str = "/var/log/deka/spool";
const DEFAULT_FLUSH_SECS: u64 = 5;
const DEFAULT_CAP_MB: u64 = 64;
const BATCH_THRESHOLD_BYTES: u64 = 256 * 1024;

static LOGGER: OnceLock<Mutex<Option<Arc<LogSink>>>> = OnceLock::new();

#[derive(Clone, Debug)]
pub(crate) struct LogConfig {
    /// Ingest host or URL (e.g. `logs.tana.gg` or `https://logs.tana.gg`).
    pub sink: Option<String>,
    /// Bearer token for the ingest endpoint.
    pub token: Option<String>,
    /// Local spool file path.
    pub spool_path: PathBuf,
    /// Whether to also print formatted lines to stderr.
    pub stdout: bool,
    /// Background flush interval, seconds.
    pub flush_secs: u64,
    /// Spool size cap in bytes.
    pub cap_bytes: u64,
    // ---- metadata enrichment (attached to every record) ----
    pub host: String,
    pub user: String,
    pub binary: String,
    pub pid: u32,
    pub shop_id: Option<String>,
    pub shard: Option<String>,
}

#[derive(serde::Serialize)]
struct LogRecord<'a> {
    ts: String,
    level: &'a str,
    component: &'a str,
    action: &'a str,
    msg: &'a str,
    host: &'a str,
    user: &'a str,
    binary: &'a str,
    pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    shop_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shard: Option<&'a str>,
}

pub(crate) struct LogSink {
    config: LogConfig,
    /// Serializes all read/modify/write access to the spool file.
    spool_lock: Mutex<()>,
    /// Guards against spawning multiple concurrent threshold flushers.
    flush_pending: Mutex<bool>,
    /// Running total of dropped lines due to the spool cap.
    dropped_count: Mutex<u64>,
}

impl LogConfig {
    pub(crate) fn from_env() -> Self {
        Self {
            sink: nonempty_env("DEKA_LOG_SINK"),
            token: nonempty_env("DEKA_LOG_TOKEN"),
            spool_path: env::var("DEKA_LOG_SPOOL")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(DEFAULT_SPOOL_PATH)),
            stdout: env::var("DEKA_LOG_STDOUT").map(|v| v == "1").unwrap_or(false),
            flush_secs: env_u64("DEKA_LOG_FLUSH_SECS", DEFAULT_FLUSH_SECS).max(1),
            cap_bytes: env_u64("DEKA_LOG_SPOOL_CAP_MB", DEFAULT_CAP_MB)
                .max(1)
                .saturating_mul(1024 * 1024),
            host: detect_host(),
            user: detect_user(),
            binary: detect_binary(),
            pid: std::process::id(),
            shop_id: nonempty_env("SHOP_ID").or_else(|| nonempty_env("DEKA_SHOP_ID")),
            shard: nonempty_env("DEKA_SHARD_SELF").or_else(|| nonempty_env("DEKA_SHARD")),
        }
    }
}

impl LogSink {
    fn new(config: LogConfig) -> Arc<Self> {
        let sink = Arc::new(Self {
            config,
            spool_lock: Mutex::new(()),
            flush_pending: Mutex::new(false),
            dropped_count: Mutex::new(0),
        });
        if sink.config.sink.is_some() {
            Self::spawn_flusher(Arc::clone(&sink));
        }
        sink
    }

    fn build_record<'a>(
        &'a self,
        level: &'a str,
        component: &'a str,
        action: &'a str,
        msg: &'a str,
    ) -> LogRecord<'a> {
        LogRecord {
            ts: chrono::Utc::now().to_rfc3339(),
            level,
            component,
            action,
            msg,
            host: &self.config.host,
            user: &self.config.user,
            binary: &self.config.binary,
            pid: self.config.pid,
            shop_id: self.config.shop_id.as_deref(),
            shard: self.config.shard.as_deref(),
        }
    }

    /// Append a single NDJSON record to the spool. No-op when no central sink
    /// is configured (local-dev stdout fallback path).
    pub(crate) fn append(&self, level: &str, component: &str, action: &str, msg: &str) {
        if self.config.sink.is_none() {
            return;
        }

        let record = self.build_record(level, component, action, msg);
        let Ok(mut line) = serde_json::to_string(&record) else {
            return;
        };
        line.push('\n');

        let Ok(_guard) = self.spool_lock.lock() else {
            return;
        };
        if let Some(parent) = self.config.spool_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.spool_path)
            .and_then(|mut file| file.write_all(line.as_bytes()))
            .is_ok()
        {
            self.enforce_cap_locked();
        }
    }

    /// stdout is opt-in via DEKA_LOG_STDOUT=1. With no central sink we fall
    /// back to stdout so local dev still sees output.
    pub(crate) fn stdout_enabled(&self) -> bool {
        self.config.sink.is_none() || self.config.stdout
    }

    fn flush_once(&self) -> bool {
        let Some(sink) = self.config.sink.as_deref() else {
            return false;
        };

        // Snapshot a whole-line prefix of the spool under the lock.
        let batch = {
            let Ok(_guard) = self.spool_lock.lock() else {
                return false;
            };
            read_file_prefix(&self.config.spool_path, BATCH_THRESHOLD_BYTES).unwrap_or_default()
        };
        if batch.is_empty() {
            return false;
        }

        let Ok(gzipped) = gzip_bytes(&batch) else {
            return false;
        };
        let url = if sink.starts_with("http://") || sink.starts_with("https://") {
            format!("{}/ingest", sink.trim_end_matches('/'))
        } else {
            format!("https://{}/ingest", sink)
        };

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(self.config.flush_secs.min(30)))
            .build();
        let mut req = agent
            .post(&url)
            .set("Content-Type", "application/x-ndjson")
            .set("Content-Encoding", "gzip");
        if let Some(token) = self.config.token.as_deref() {
            req = req.set("Authorization", &format!("Bearer {}", token));
        }
        // ureq returns Err for non-2xx; either way we only truncate on success.
        match req.send_bytes(&gzipped) {
            Ok(_) => {}
            Err(_) => return false,
        }

        // On success, advance the offset by truncating exactly the flushed
        // prefix — never dropping lines appended while the POST was in flight.
        let Ok(_guard) = self.spool_lock.lock() else {
            return false;
        };
        truncate_flushed_prefix(&self.config.spool_path, &batch).is_ok()
    }

    /// Enforce the spool size cap by dropping the oldest whole lines. Records a
    /// running drop-count which is itself appended to the spool so it ships to
    /// the central sink. Must be called while holding `spool_lock`.
    fn enforce_cap_locked(&self) {
        let Ok(metadata) = fs::metadata(&self.config.spool_path) else {
            return;
        };
        if metadata.len() <= self.config.cap_bytes {
            return;
        }

        let Ok(contents) = fs::read(&self.config.spool_path) else {
            return;
        };
        let keep_from = find_keep_start(&contents, self.config.cap_bytes as usize);
        let dropped = contents[..keep_from]
            .iter()
            .filter(|b| **b == b'\n')
            .count() as u64;
        let mut kept = contents[keep_from..].to_vec();
        let dropped_total = self
            .dropped_count
            .lock()
            .map(|mut total| {
                *total = total.saturating_add(dropped);
                *total
            })
            .unwrap_or(dropped);
        let msg = format!("dropped_count={} dropped_now={}", dropped_total, dropped);
        let drop_record = self.build_record("warn", "stdio", "spool_drop", &msg);
        if let Ok(mut line) = serde_json::to_string(&drop_record) {
            line.push('\n');
            kept.extend_from_slice(line.as_bytes());
        }
        let _ = fs::write(&self.config.spool_path, kept);
    }

    fn spawn_flusher(sink: Arc<Self>) {
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(sink.config.flush_secs));
            let _ = sink.flush_once();
        });
    }
}

/// Trigger an out-of-band flush when the spool crosses the batch threshold.
/// Non-blocking: spawns at most one flusher at a time.
pub(crate) fn maybe_flush_for_threshold(sink: &Arc<LogSink>) {
    if sink.config.sink.is_none() {
        return;
    }
    let Ok(metadata) = fs::metadata(&sink.config.spool_path) else {
        return;
    };
    if metadata.len() < BATCH_THRESHOLD_BYTES {
        return;
    }
    let Ok(mut pending) = sink.flush_pending.lock() else {
        return;
    };
    if *pending {
        return;
    }
    *pending = true;
    let sink = Arc::clone(sink);
    thread::spawn(move || {
        let _ = sink.flush_once();
        if let Ok(mut pending) = sink.flush_pending.lock() {
            *pending = false;
        }
    });
}

/// Process-wide lazily-initialized logger built from the environment.
pub(crate) fn logger() -> Arc<LogSink> {
    let lock = LOGGER.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().expect("stdio logger lock poisoned");
    if let Some(logger) = guard.as_ref() {
        return Arc::clone(logger);
    }
    let logger = LogSink::new(LogConfig::from_env());
    *guard = Some(Arc::clone(&logger));
    logger
}

fn nonempty_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn detect_host() -> String {
    nonempty_env("HOSTNAME").unwrap_or_else(|| {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    })
}

fn detect_user() -> String {
    nonempty_env("USER")
        .or_else(|| nonempty_env("LOGNAME"))
        .or_else(|| {
            // Fall back to `id -un` (covers unix daemons whose env lacks USER).
            std::process::Command::new("id")
                .arg("-un")
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_binary() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Read a whole-line prefix of up to `max_bytes` from the spool. Trims any
/// trailing partial line so we never ship a half-written record.
fn read_file_prefix(path: &PathBuf, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    let mut buf = Vec::new();
    Read::by_ref(&mut file).take(max_bytes).read_to_end(&mut buf)?;
    if !buf.ends_with(b"\n") {
        if let Some(pos) = buf.iter().rposition(|b| *b == b'\n') {
            buf.truncate(pos + 1);
        } else {
            buf.clear();
        }
    }
    Ok(buf)
}

fn gzip_bytes(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

/// Remove exactly the flushed prefix from the spool, preserving any bytes that
/// were appended after the snapshot was taken.
fn truncate_flushed_prefix(path: &PathBuf, flushed: &[u8]) -> std::io::Result<()> {
    let current = fs::read(path)?;
    if current.starts_with(flushed) {
        fs::write(path, &current[flushed.len()..])?;
    }
    Ok(())
}

/// Find the byte offset of the first whole line to keep so that the retained
/// tail is at most `cap_bytes`.
fn find_keep_start(contents: &[u8], cap_bytes: usize) -> usize {
    if contents.len() <= cap_bytes {
        return 0;
    }
    let start = contents.len().saturating_sub(cap_bytes);
    contents[start..]
        .iter()
        .position(|b| *b == b'\n')
        .map(|idx| start + idx + 1)
        .unwrap_or(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn test_config(spool_path: PathBuf) -> LogConfig {
        LogConfig {
            sink: Some("http://127.0.0.1:9".to_string()),
            token: Some("test-token".to_string()),
            spool_path,
            stdout: false,
            flush_secs: 1,
            cap_bytes: 1024 * 1024,
            host: "test-host".to_string(),
            user: "test-user".to_string(),
            binary: "test-binary".to_string(),
            pid: 4242,
            shop_id: Some("shop_alpha".to_string()),
            shard: Some("phobos".to_string()),
        }
    }

    fn make_sink(config: LogConfig) -> LogSink {
        LogSink {
            config,
            spool_lock: Mutex::new(()),
            flush_pending: Mutex::new(false),
            dropped_count: Mutex::new(0),
        }
    }

    fn read_lines(path: &PathBuf) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn spool_append_writes_valid_ndjson_with_all_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("spool.ndjson");
        let sink = make_sink(test_config(path.clone()));

        sink.append("warn", "router", "resolve", "tenant missing");

        let lines = read_lines(&path);
        assert_eq!(lines.len(), 1);
        let value: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert!(value["ts"].as_str().unwrap().contains('T'));
        assert_eq!(value["level"], "warn");
        assert_eq!(value["component"], "router");
        assert_eq!(value["action"], "resolve");
        assert_eq!(value["msg"], "tenant missing");
        assert_eq!(value["host"], "test-host");
        assert_eq!(value["user"], "test-user");
        assert_eq!(value["binary"], "test-binary");
        assert_eq!(value["pid"], 4242);
        assert_eq!(value["shop_id"], "shop_alpha");
        assert_eq!(value["shard"], "phobos");
    }

    #[test]
    fn optional_fields_omitted_when_absent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("spool.ndjson");
        let mut config = test_config(path.clone());
        config.shop_id = None;
        config.shard = None;
        let sink = make_sink(config);

        sink.append("info", "build", "build", "no tenant context");

        let lines = read_lines(&path);
        let value: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert!(value.get("shop_id").is_none());
        assert!(value.get("shard").is_none());
        // required fields still present
        assert_eq!(value["pid"], 4242);
        assert_eq!(value["user"], "test-user");
    }

    #[test]
    fn flush_advances_offset_and_clears_flushed_lines() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("spool.ndjson");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            // HTTP header names are case-insensitive; normalize for assertions.
            let mut headers = String::new();
            let mut content_len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    content_len = value.trim().parse().unwrap();
                }
                headers.push_str(&line);
            }
            let mut body = vec![0; content_len];
            reader.read_exact(&mut body).unwrap();
            let mut decoder = GzDecoder::new(&body[..]);
            let mut decoded = String::new();
            decoder.read_to_string(&mut decoded).unwrap();
            tx.send((headers, decoded)).unwrap();
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });

        let mut config = test_config(path.clone());
        config.sink = Some(format!("http://{}", addr));
        let sink = make_sink(config);
        sink.append("info", "build", "build", "one");
        sink.append("error", "build", "build", "two");

        assert!(sink.flush_once());

        let (headers, decoded) = rx.recv().unwrap();
        let headers_lc = headers.to_ascii_lowercase();
        assert!(headers_lc.contains("authorization: bearer test-token"));
        assert!(headers_lc.contains("content-encoding: gzip"));
        assert_eq!(decoded.lines().count(), 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn demon_unreachable_retains_lines_and_cap_drops_oldest() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("spool.ndjson");
        let mut config = test_config(path.clone());
        config.cap_bytes = 500;
        config.sink = Some("http://127.0.0.1:9".to_string());
        let sink = make_sink(config);

        for idx in 0..20 {
            sink.append(
                "info",
                "cap",
                "write",
                &format!("line-{idx:02}-{}", "x".repeat(80)),
            );
        }

        // Unreachable demon -> flush fails -> lines retained, oldest dropped.
        assert!(!sink.flush_once());
        let contents = fs::read_to_string(&path).unwrap();
        assert!(fs::metadata(&path).unwrap().len() < 1024);
        assert!(!contents.contains("line-00"));
        assert!(contents.contains("dropped_count="));
    }

    #[test]
    fn stdout_is_gated_by_env_flag() {
        let mut config = test_config(PathBuf::from("/tmp/tana-stdio-test-spool"));
        config.stdout = false;
        assert!(!make_sink(config).stdout_enabled());

        let mut config = test_config(PathBuf::from("/tmp/tana-stdio-test-spool"));
        config.stdout = true;
        assert!(make_sink(config).stdout_enabled());
    }

    #[test]
    fn sink_unset_falls_back_to_stdout_and_skips_spool() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("spool.ndjson");
        let mut config = test_config(path.clone());
        config.sink = None;
        config.stdout = false;
        let sink = make_sink(config);

        sink.append("info", "local", "dev", "hello");

        assert!(sink.stdout_enabled());
        assert!(!path.exists());
    }
}
