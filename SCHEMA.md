# tana-stdio audit log schema

Both the Rust crate (`tana-stdio`) and the npm package (`@tananetwork/stdio`)
emit the **same** central audit log format. Every formatting call
(`log`, `error`, `warn`, `status`, `success`, `fail`, `info`, `header`,
`diagnostic`, `debug`, …) appends exactly **one JSON object per line** (NDJSON)
to a local spool file, which a background flusher gzips and POSTs to a central
`/ingest` endpoint.

## Record format

One JSON object per line, UTF-8, newline-terminated:

```json
{"ts":"2026-05-31T00:46:12.345Z","level":"warn","component":"router","action":"resolve","msg":"tenant missing","host":"phobos","user":"sami","binary":"cli","pid":40231,"shop_id":"shop_beta","shard":"phobos"}
```

### Fields

| Field      | Type    | Required | Description |
|------------|---------|----------|-------------|
| `ts`       | string  | yes      | RFC 3339 / ISO-8601 timestamp in UTC. (Rust: `chrono::Utc::now().to_rfc3339()`; TS: `new Date().toISOString()`.) |
| `level`    | string  | yes      | One of `error`, `warn`, `info`, `debug`, `status`. |
| `component`| string  | yes      | Logical subsystem / action name passed by the caller (e.g. `router`, `build`, `database`). |
| `action`   | string  | yes      | Specific verb for the event (e.g. `resolve`, `ok`, `fail`, `warn`, `spool_drop`). |
| `msg`      | string  | yes      | Human-readable message body. |
| `host`     | string  | yes      | Hostname. `HOSTNAME` env, else `os.hostname()` (TS) / `hostname` command (Rust), else `unknown`. |
| `user`     | string  | yes      | Unix user. `USER`/`LOGNAME` env, else `os.userInfo().username` (TS) / `id -un` (Rust), else `unknown`. |
| `binary`   | string  | yes      | Basename of the running executable/entry script. `current_exe()` basename (Rust) / `basename(argv[1]‖argv[0]‖process.title)` (TS), else `unknown`. |
| `pid`      | number  | yes      | Process id. |
| `shop_id`  | string  | no       | Tenant id, from `SHOP_ID` or `DEKA_SHOP_ID`. **Omitted** when unset. |
| `shard`    | string  | no       | Shard self-id, from `DEKA_SHARD_SELF` or `DEKA_SHARD`. **Omitted** when unset. |

`shop_id` and `shard` are the only optional fields; they are omitted entirely
(not emitted as `null`) when their environment variables are unset.

> Lineage: issue #506 (deka fork) captured `ts/level/component/action/msg/host/shop_id/shard`.
> This Phase-1 port into the canonical `tana-stdio` library **adds** `user`, `binary`, and `pid`.

## Environment / wire contract

These names are the contract shared with the demon `/ingest` receiver and
cloud-init. They are **not renamed** here.

| Variable                | Default              | Meaning |
|-------------------------|----------------------|---------|
| `DEKA_LOG_SINK`         | (unset → stdout)     | Ingest host or full URL. `https://` is assumed if no scheme is present. |
| `DEKA_LOG_TOKEN`        | (none)               | Sent as `Authorization: Bearer <token>`. |
| `DEKA_LOG_SPOOL`        | `/var/log/deka/spool`| Local NDJSON spool file path. |
| `DEKA_LOG_STDOUT`       | `0`                  | `1` also prints formatted lines to the console (opt-in). |
| `DEKA_LOG_FLUSH_SECS`   | `5`                  | Background flush interval (seconds; min 1). |
| `DEKA_LOG_SPOOL_CAP_MB` | `64`                 | Spool size cap (MB; min 1). Oldest lines dropped past the cap. |

## Wire request

- `POST https://$DEKA_LOG_SINK/ingest`
- `Content-Type: application/x-ndjson`
- `Content-Encoding: gzip`
- `Authorization: Bearer $DEKA_LOG_TOKEN`
- Body: gzip-compressed NDJSON batch (whole lines only; a batch is capped at
  256 KiB of pre-gzip spool bytes).

## Behavior

- **Append-always.** Every call appends one record to the spool (when
  `DEKA_LOG_SINK` is set). stdout is opt-in via `DEKA_LOG_STDOUT=1`.
- **Local-dev fallback.** If `DEKA_LOG_SINK` is unset, nothing is spooled and
  output goes to the console so local development still works.
- **Non-blocking flush.** A background timer flushes every
  `DEKA_LOG_FLUSH_SECS`; a 256 KiB threshold also triggers an out-of-band
  flush. The caller is never blocked on the network.
- **Retain-on-failure.** A flush only truncates the spool after a `2xx`
  response, and truncates **exactly** the flushed prefix — lines appended while
  a POST was in flight are preserved.
- **Cap + drop-count.** When the spool exceeds `DEKA_LOG_SPOOL_CAP_MB`, the
  oldest whole lines are dropped and a `{"action":"spool_drop","msg":"dropped_count=N dropped_now=M"}`
  record is appended so the loss is itself auditable downstream.
