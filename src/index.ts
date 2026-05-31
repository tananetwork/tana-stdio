/**
 * @tananetwork/stdio
 *
 * Terminal output utilities for Tana projects.
 * Consistent formatting across CLI, engine, and plugins.
 *
 * Format: [action] value
 * - Cyan brackets for identifiers
 * - Green dot for success, red dot for failure
 * - Red text for errors, yellow for warnings
 *
 * ## Central audit sink
 *
 * Every log/error/warn/status/etc. call ALSO appends one NDJSON record to a
 * local spool file; a background timer gzips batches and POSTs them to a
 * central `/ingest` endpoint. Configuration is environment-driven:
 *
 * - `DEKA_LOG_SINK`         host or URL of the ingest endpoint
 * - `DEKA_LOG_TOKEN`        bearer token (`Authorization: Bearer ...`)
 * - `DEKA_LOG_SPOOL`        spool file path (default `/var/log/deka/spool`)
 * - `DEKA_LOG_STDOUT`       set to `1` to also print to the console (opt-in)
 * - `DEKA_LOG_FLUSH_SECS`   background flush interval, seconds (default 5)
 * - `DEKA_LOG_SPOOL_CAP_MB` spool size cap in MB (default 64, oldest dropped)
 *
 * If `DEKA_LOG_SINK` is unset, output falls back to the console so local dev
 * still works.
 */

import chalk from 'chalk'
import figlet from 'figlet'
import { logger } from './sink.js'

// Brand color - matches tana.network website (oklch 0.65 0.2 220)
const BRAND_BLUE = '\x1b[38;5;39m'
const BOLD = '\x1b[1m'
const RESET = '\x1b[0m'

/**
 * Fan a single call out to the central audit sink (NDJSON spool) and, when
 * stdout is enabled, to the console with the pretty `display` string.
 *
 * stdout is opt-in via DEKA_LOG_STDOUT=1; with no DEKA_LOG_SINK configured the
 * sink is inert and we always print (local-dev fallback).
 */
function emit(level: string, component: string, action: string, msg: string, display: string): void {
  const log = logger()
  log.append(level, component, action, msg)
  log.maybeFlushForThreshold()
  if (log.stdoutEnabled()) {
    console.log(display)
  }
}

/**
 * Generate ASCII art banner in Tana brand style
 * Uses "Terrace" font with brand blue color
 *
 * @example
 * const banner = ascii('box')
 * console.log(banner)
 */
export function ascii(text: string): string {
  const art = figlet.textSync(text, { font: 'Terrace' }).trimEnd()
  return `${BRAND_BLUE}${BOLD}\n${art}${RESET}\n`
}

/**
 * Log an action with a value
 * [action] value
 */
export function log(action: string, value: string): void {
  emit('info', action, action, value, `${chalk.cyan(`[${action}]`)} ${value}`)
}

/**
 * Log an error
 * [action] message (cyan bracket, red message)
 */
export function error(action: string, message: string): void {
  emit('error', action, action, message, `${chalk.cyan(`[${action}]`)} ${chalk.red(message)}`)
}

/**
 * Log a warning
 * ● [name] message - if message provided
 * ● message - if only one arg
 */
export function warn(name: string, message?: string): void {
  if (message !== undefined) {
    emit('warn', name, 'warn', message, `${chalk.yellow('●')} ${chalk.cyan(`[${name}]`)} ${message}`)
  } else {
    emit('warn', 'stdio', 'warn', name, `${chalk.yellow('●')} ${name}`)
  }
}

/**
 * Log a status line with indicator
 * ● [name] message (dot indicates status)
 */
export function status(name: string, message: string, ok: boolean): void {
  const dot = ok ? chalk.green('●') : chalk.red('○')
  const display = `${dot} ${chalk.cyan(`[${name}]`)} ${ok ? chalk.gray(message) : chalk.red(message)}`
  emit(ok ? 'status' : 'error', name, ok ? 'ok' : 'fail', message, display)
}

/**
 * Print a section header
 */
export function header(title: string): void {
  emit('info', 'stdio', 'raw', '', '')
  emit('info', 'stdio', 'header', title, chalk.bold(title))
  emit('info', 'stdio', 'raw', '─'.repeat(40), chalk.gray('─'.repeat(40)))
}

/**
 * Print a blank line
 */
export function blank(): void {
  emit('info', 'stdio', 'raw', '', '')
}

/**
 * Success message
 * ✓ message
 */
export function success(message: string): void {
  emit('status', 'stdio', 'ok', message, `${chalk.green('✓')} ${message}`)
}

/**
 * Failure message
 * ✗ message
 */
export function fail(message: string): void {
  emit('error', 'stdio', 'fail', message, `${chalk.red('✗')} ${message}`)
}

/**
 * Info line with label
 * label     value
 */
export function info(label: string, value: string): void {
  emit('info', label, 'info', value, `  ${label.padEnd(10)} ${chalk.cyan(value)}`)
}

/**
 * Hint in gray
 */
export function hint(message: string): void {
  emit('info', 'stdio', 'hint', message, chalk.gray(`  ${message}`))
}

/**
 * Detail line with arrow
 */
export function detail(message: string): void {
  emit('info', 'stdio', 'detail', message, `    ${chalk.gray('→')} ${message}`)
}

/**
 * Fatal error - logs and exits
 */
export function fatal(action: string, message: string): never {
  error(action, message)
  return process.exit(1)
}

/**
 * Suggest a next step
 *   → description: command
 */
export function nextStep(description: string, command: string): void {
  emit(
    'info',
    'stdio',
    'next_step',
    `${description}: ${command}`,
    `  ${chalk.gray('→')} ${description}: ${chalk.cyan(command)}`,
  )
}

/**
 * Suggest multiple next steps
 */
export function nextSteps(steps: Array<{ description: string; command: string }>): void {
  for (const step of steps) {
    nextStep(step.description, step.command)
  }
}

/**
 * Diagnostic warning - yellow indicator with issue description
 * ⚠ [component] message
 */
export function diagnostic(component: string, message: string): void {
  emit(
    'warn',
    component,
    'diagnostic',
    message,
    `${chalk.yellow('⚠')} ${chalk.cyan(`[${component}]`)} ${chalk.yellow(message)}`,
  )
}

// Namespace export for cleaner imports
export const out = {
  ascii,
  log,
  error,
  warn,
  status,
  header,
  blank,
  success,
  fail,
  info,
  hint,
  detail,
  fatal,
  nextStep,
  nextSteps,
  diagnostic,
}

// Default export
export default out
