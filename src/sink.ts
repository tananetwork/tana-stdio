/**
 * Central audit log sink for @tananetwork/stdio.
 *
 * Every formatting call also appends one NDJSON record to a local spool file.
 * A background timer (and a size-threshold trigger) gzips batches and POSTs
 * them to `https://$DEKA_LOG_SINK/ingest` with a bearer token. On 2xx the
 * flushed prefix is truncated from the spool; on failure lines are retained
 * and retried. The spool is capped at `DEKA_LOG_SPOOL_CAP_MB`, dropping the
 * oldest lines and logging a running drop-count. The caller is never blocked.
 *
 * Built entirely on Node built-ins (fs, os, zlib, https/http) — no new deps.
 */

import * as fs from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import * as zlib from 'node:zlib'
import * as http from 'node:http'
import * as https from 'node:https'

const DEFAULT_SPOOL_PATH = '/var/log/deka/spool'
const DEFAULT_FLUSH_SECS = 5
const DEFAULT_CAP_MB = 64
const BATCH_THRESHOLD_BYTES = 256 * 1024

/** A single NDJSON audit record. Optional fields are omitted when absent. */
interface LogRecord {
  ts: string
  level: string
  component: string
  action: string
  msg: string
  host: string
  user: string
  binary: string
  pid: number
  shop_id?: string
  shard?: string
}

interface LogConfig {
  sink?: string
  token?: string
  spoolPath: string
  stdout: boolean
  flushSecs: number
  capBytes: number
  host: string
  user: string
  binary: string
  pid: number
  shopId?: string
  shard?: string
}

function nonemptyEnv(key: string): string | undefined {
  const value = process.env[key]
  if (value === undefined) return undefined
  const trimmed = value.trim()
  return trimmed.length > 0 ? value : undefined
}

function envU64(key: string, fallback: number): number {
  const raw = process.env[key]
  if (raw === undefined) return fallback
  const parsed = Number.parseInt(raw, 10)
  return Number.isFinite(parsed) ? parsed : fallback
}

function detectHost(): string {
  const env = nonemptyEnv('HOSTNAME')
  if (env !== undefined) return env
  const h = os.hostname()
  return h && h.length > 0 ? h : 'unknown'
}

function detectUser(): string {
  const env = nonemptyEnv('USER') ?? nonemptyEnv('LOGNAME')
  if (env !== undefined) return env
  try {
    const name = os.userInfo().username
    if (name && name.length > 0) return name
  } catch {
    // os.userInfo() can throw on some daemonized environments
  }
  return 'unknown'
}

function detectBinary(): string {
  // process.argv[1] is the entry script; fall back to argv[0] (the executable)
  // then process.title. Use the basename so we record e.g. `cli` not a path.
  const candidate = process.argv[1] ?? process.argv[0] ?? process.title
  if (candidate && candidate.length > 0) {
    const base = path.basename(candidate)
    if (base.length > 0) return base
  }
  return 'unknown'
}

function loadConfig(): LogConfig {
  return {
    sink: nonemptyEnv('DEKA_LOG_SINK'),
    token: nonemptyEnv('DEKA_LOG_TOKEN'),
    spoolPath: nonemptyEnv('DEKA_LOG_SPOOL') ?? DEFAULT_SPOOL_PATH,
    stdout: process.env.DEKA_LOG_STDOUT === '1',
    flushSecs: Math.max(1, envU64('DEKA_LOG_FLUSH_SECS', DEFAULT_FLUSH_SECS)),
    capBytes: Math.max(1, envU64('DEKA_LOG_SPOOL_CAP_MB', DEFAULT_CAP_MB)) * 1024 * 1024,
    host: detectHost(),
    user: detectUser(),
    binary: detectBinary(),
    pid: process.pid,
    shopId: nonemptyEnv('SHOP_ID') ?? nonemptyEnv('DEKA_SHOP_ID'),
    shard: nonemptyEnv('DEKA_SHARD_SELF') ?? nonemptyEnv('DEKA_SHARD'),
  }
}

/** Offset of the first whole line to keep so the retained tail <= capBytes. */
function findKeepStart(contents: Buffer, capBytes: number): number {
  if (contents.length <= capBytes) return 0
  const start = contents.length - capBytes
  for (let i = start; i < contents.length; i++) {
    if (contents[i] === 0x0a) return i + 1
  }
  return start
}

/** Trim a buffer to the largest whole-line prefix (drop trailing partial). */
function wholeLinePrefix(buf: Buffer): Buffer {
  if (buf.length === 0) return buf
  if (buf[buf.length - 1] === 0x0a) return buf
  const idx = buf.lastIndexOf(0x0a)
  if (idx < 0) return Buffer.alloc(0)
  return buf.subarray(0, idx + 1)
}

export class LogSink {
  private readonly config: LogConfig
  private flushPending = false
  private droppedCount = 0
  private timer?: ReturnType<typeof setInterval>

  constructor(config: LogConfig) {
    this.config = config
    if (this.config.sink !== undefined) {
      this.timer = setInterval(() => {
        void this.flushOnce()
      }, this.config.flushSecs * 1000)
      // Do not keep the event loop alive solely for log flushing.
      if (typeof this.timer.unref === 'function') this.timer.unref()
    }
  }

  /** Expose config for tests / introspection. */
  hasSink(): boolean {
    return this.config.sink !== undefined
  }

  private buildRecord(level: string, component: string, action: string, msg: string): LogRecord {
    const record: LogRecord = {
      ts: new Date().toISOString(),
      level,
      component,
      action,
      msg,
      host: this.config.host,
      user: this.config.user,
      binary: this.config.binary,
      pid: this.config.pid,
    }
    if (this.config.shopId !== undefined) record.shop_id = this.config.shopId
    if (this.config.shard !== undefined) record.shard = this.config.shard
    return record
  }

  /** Append one NDJSON record. No-op when no central sink is configured. */
  append(level: string, component: string, action: string, msg: string): void {
    if (this.config.sink === undefined) return

    let line: string
    try {
      line = JSON.stringify(this.buildRecord(level, component, action, msg)) + '\n'
    } catch {
      return
    }

    try {
      const dir = path.dirname(this.config.spoolPath)
      fs.mkdirSync(dir, { recursive: true })
      fs.appendFileSync(this.config.spoolPath, line)
      this.enforceCap()
    } catch {
      // Spool write failures must never break the caller.
    }
  }

  /** stdout is opt-in (DEKA_LOG_STDOUT=1); with no sink fall back to stdout. */
  stdoutEnabled(): boolean {
    return this.config.sink === undefined || this.config.stdout
  }

  /** Trigger a non-blocking flush when the spool crosses the batch threshold. */
  maybeFlushForThreshold(): void {
    if (this.config.sink === undefined || this.flushPending) return
    let size = 0
    try {
      size = fs.statSync(this.config.spoolPath).size
    } catch {
      return
    }
    if (size < BATCH_THRESHOLD_BYTES) return
    this.flushPending = true
    void this.flushOnce().finally(() => {
      this.flushPending = false
    })
  }

  /** Enforce the spool cap by dropping oldest whole lines + a drop-count line. */
  private enforceCap(): void {
    let size = 0
    try {
      size = fs.statSync(this.config.spoolPath).size
    } catch {
      return
    }
    if (size <= this.config.capBytes) return

    let contents: Buffer
    try {
      contents = fs.readFileSync(this.config.spoolPath)
    } catch {
      return
    }
    const keepFrom = findKeepStart(contents, this.config.capBytes)
    let dropped = 0
    for (let i = 0; i < keepFrom; i++) {
      if (contents[i] === 0x0a) dropped++
    }
    this.droppedCount += dropped
    const kept = contents.subarray(keepFrom)

    const msg = `dropped_count=${this.droppedCount} dropped_now=${dropped}`
    let dropLine = ''
    try {
      dropLine = JSON.stringify(this.buildRecord('warn', 'stdio', 'spool_drop', msg)) + '\n'
    } catch {
      dropLine = ''
    }
    try {
      fs.writeFileSync(this.config.spoolPath, Buffer.concat([kept, Buffer.from(dropLine)]))
    } catch {
      // ignore
    }
  }

  /**
   * Read a whole-line batch, gzip it, POST to /ingest. On 2xx truncate exactly
   * the flushed prefix (never dropping lines appended during the request).
   * Resolves true on a successful flush, false otherwise. Never throws.
   */
  async flushOnce(): Promise<boolean> {
    const sink = this.config.sink
    if (sink === undefined) return false

    let raw: Buffer
    try {
      const fd = fs.openSync(this.config.spoolPath, 'r')
      try {
        const buf = Buffer.alloc(BATCH_THRESHOLD_BYTES)
        const read = fs.readSync(fd, buf, 0, BATCH_THRESHOLD_BYTES, 0)
        raw = buf.subarray(0, read)
      } finally {
        fs.closeSync(fd)
      }
    } catch {
      return false
    }
    const batch = wholeLinePrefix(raw)
    if (batch.length === 0) return false

    let gzipped: Buffer
    try {
      gzipped = zlib.gzipSync(batch)
    } catch {
      return false
    }

    const url =
      sink.startsWith('http://') || sink.startsWith('https://')
        ? `${sink.replace(/\/+$/, '')}/ingest`
        : `https://${sink}/ingest`

    const ok = await this.post(url, gzipped)
    if (!ok) return false

    // Advance offset: re-read and strip exactly the flushed prefix.
    try {
      const current = fs.readFileSync(this.config.spoolPath)
      if (current.length >= batch.length && current.subarray(0, batch.length).equals(batch)) {
        fs.writeFileSync(this.config.spoolPath, current.subarray(batch.length))
      }
    } catch {
      return false
    }
    return true
  }

  private post(url: string, body: Buffer): Promise<boolean> {
    return new Promise((resolve) => {
      let parsed: URL
      try {
        parsed = new URL(url)
      } catch {
        resolve(false)
        return
      }
      const transport = parsed.protocol === 'http:' ? http : https
      const headers: Record<string, string> = {
        'Content-Type': 'application/x-ndjson',
        'Content-Encoding': 'gzip',
        'Content-Length': String(body.length),
      }
      if (this.config.token !== undefined) {
        headers['Authorization'] = `Bearer ${this.config.token}`
      }
      const req = transport.request(
        {
          method: 'POST',
          hostname: parsed.hostname,
          port: parsed.port || (parsed.protocol === 'http:' ? 80 : 443),
          path: parsed.pathname + parsed.search,
          headers,
          timeout: Math.min(30, this.config.flushSecs) * 1000,
        },
        (res) => {
          const status = res.statusCode ?? 0
          // Drain so the socket frees up.
          res.on('data', () => {})
          res.on('end', () => resolve(status >= 200 && status < 300))
        },
      )
      req.on('error', () => resolve(false))
      req.on('timeout', () => {
        req.destroy()
        resolve(false)
      })
      req.end(body)
    })
  }
}

let loggerInstance: LogSink | undefined

/** Process-wide lazily-initialized logger built from the environment. */
export function logger(): LogSink {
  if (loggerInstance === undefined) {
    loggerInstance = new LogSink(loadConfig())
  }
  return loggerInstance
}

/** Test-only hook to build a sink from an explicit config. */
export function _sinkForTest(config: Partial<LogConfig> & { spoolPath: string }): LogSink {
  const full: LogConfig = {
    sink: undefined,
    token: undefined,
    stdout: false,
    flushSecs: 1,
    capBytes: 1024 * 1024,
    host: 'test-host',
    user: 'test-user',
    binary: 'test-binary',
    pid: 4242,
    shopId: 'shop_alpha',
    shard: 'phobos',
    ...config,
  }
  return new LogSink(full)
}
