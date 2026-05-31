import { test, expect } from 'bun:test'
import * as fs from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import * as zlib from 'node:zlib'
import * as http from 'node:http'
import { _sinkForTest, LogSink } from '../src/sink.ts'

function tmpSpool(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tana-stdio-'))
  return path.join(dir, 'spool.ndjson')
}

function readLines(p: string): string[] {
  if (!fs.existsSync(p)) return []
  return fs
    .readFileSync(p, 'utf8')
    .split('\n')
    .filter((l) => l.length > 0)
}

test('spool append writes valid NDJSON with all metadata fields', () => {
  const spoolPath = tmpSpool()
  const sink: LogSink = _sinkForTest({ spoolPath, sink: 'http://127.0.0.1:9', token: 'test-token' })

  sink.append('warn', 'router', 'resolve', 'tenant missing')

  const lines = readLines(spoolPath)
  expect(lines.length).toBe(1)
  const rec = JSON.parse(lines[0])
  expect(rec.ts).toContain('T')
  expect(rec.level).toBe('warn')
  expect(rec.component).toBe('router')
  expect(rec.action).toBe('resolve')
  expect(rec.msg).toBe('tenant missing')
  expect(rec.host).toBe('test-host')
  expect(rec.user).toBe('test-user')
  expect(rec.binary).toBe('test-binary')
  expect(rec.pid).toBe(4242)
  expect(rec.shop_id).toBe('shop_alpha')
  expect(rec.shard).toBe('phobos')
})

test('optional fields omitted when shop_id/shard absent', () => {
  const spoolPath = tmpSpool()
  const sink = _sinkForTest({
    spoolPath,
    sink: 'http://127.0.0.1:9',
    shopId: undefined,
    shard: undefined,
  })
  sink.append('info', 'build', 'build', 'no tenant')
  const rec = JSON.parse(readLines(spoolPath)[0])
  expect('shop_id' in rec).toBe(false)
  expect('shard' in rec).toBe(false)
  expect(rec.pid).toBe(4242)
  expect(rec.user).toBe('test-user')
})

test('stdout is gated: off by default, on with DEKA_LOG_STDOUT-equivalent', () => {
  const off = _sinkForTest({ spoolPath: tmpSpool(), sink: 'http://127.0.0.1:9', stdout: false })
  expect(off.stdoutEnabled()).toBe(false)

  const on = _sinkForTest({ spoolPath: tmpSpool(), sink: 'http://127.0.0.1:9', stdout: true })
  expect(on.stdoutEnabled()).toBe(true)
})

test('sink unset falls back to stdout and skips spool', () => {
  const spoolPath = tmpSpool()
  const sink = _sinkForTest({ spoolPath, sink: undefined, stdout: false })
  sink.append('info', 'local', 'dev', 'hello')
  expect(sink.stdoutEnabled()).toBe(true)
  expect(fs.existsSync(spoolPath)).toBe(false)
})

test('cap drops oldest lines and records a drop-count', () => {
  const spoolPath = tmpSpool()
  const sink = _sinkForTest({
    spoolPath,
    sink: 'http://127.0.0.1:9',
    capBytes: 500,
  })
  for (let i = 0; i < 20; i++) {
    sink.append('info', 'cap', 'write', `line-${String(i).padStart(2, '0')}-${'x'.repeat(80)}`)
  }
  const contents = fs.readFileSync(spoolPath, 'utf8')
  expect(fs.statSync(spoolPath).size).toBeLessThan(1024)
  expect(contents.includes('line-00')).toBe(false)
  expect(contents.includes('dropped_count=')).toBe(true)
})

test('flush gzip-POSTs the batch with bearer auth and truncates on 2xx', async () => {
  const spoolPath = tmpSpool()
  const received: { headers: http.IncomingHttpHeaders; body: string }[] = []

  const server = http.createServer((req, res) => {
    const chunks: Buffer[] = []
    req.on('data', (c) => chunks.push(c))
    req.on('end', () => {
      const buf = Buffer.concat(chunks)
      const body = zlib.gunzipSync(buf).toString('utf8')
      received.push({ headers: req.headers, body })
      res.writeHead(204)
      res.end()
    })
  })
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve))
  const addr = server.address()
  const port = typeof addr === 'object' && addr ? addr.port : 0

  const sink = _sinkForTest({
    spoolPath,
    sink: `http://127.0.0.1:${port}`,
    token: 'test-token',
  })
  sink.append('info', 'build', 'build', 'one')
  sink.append('error', 'build', 'build', 'two')

  const ok = await sink.flushOnce()
  server.close()

  expect(ok).toBe(true)
  expect(received.length).toBe(1)
  expect(received[0].headers['authorization']).toBe('Bearer test-token')
  expect(received[0].headers['content-encoding']).toBe('gzip')
  expect(received[0].body.split('\n').filter((l) => l.length > 0).length).toBe(2)
  expect(fs.readFileSync(spoolPath, 'utf8')).toBe('')
})

test('unreachable sink retains lines (flush returns false)', async () => {
  const spoolPath = tmpSpool()
  const sink = _sinkForTest({ spoolPath, sink: 'http://127.0.0.1:9', flushSecs: 1 })
  sink.append('info', 'net', 'down', 'still here')
  const ok = await sink.flushOnce()
  expect(ok).toBe(false)
  expect(readLines(spoolPath).length).toBe(1)
})
