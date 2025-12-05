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
 */

import chalk from 'chalk'

/**
 * Log an action with a value
 * [action] value
 */
export function log(action: string, value: string): void {
  console.log(`${chalk.cyan(`[${action}]`)} ${value}`)
}

/**
 * Log an error
 * [action] message (cyan bracket, red message)
 */
export function error(action: string, message: string): void {
  console.log(`${chalk.cyan(`[${action}]`)} ${chalk.red(message)}`)
}

/**
 * Log a warning
 * ● [name] message - if message provided
 * ● message - if only one arg
 */
export function warn(name: string, message?: string): void {
  if (message !== undefined) {
    console.log(`${chalk.yellow('●')} ${chalk.cyan(`[${name}]`)} ${message}`)
  } else {
    console.log(`${chalk.yellow('●')} ${name}`)
  }
}

/**
 * Log a status line with indicator
 * ● [name] message (dot indicates status)
 */
export function status(name: string, message: string, ok: boolean): void {
  const dot = ok ? chalk.green('●') : chalk.red('○')
  console.log(`${dot} ${chalk.cyan(`[${name}]`)} ${ok ? chalk.gray(message) : chalk.red(message)}`)
}

/**
 * Print a section header
 */
export function header(title: string): void {
  console.log()
  console.log(chalk.bold(title))
  console.log(chalk.gray('─'.repeat(40)))
}

/**
 * Print a blank line
 */
export function blank(): void {
  console.log()
}

/**
 * Success message
 * ✓ message
 */
export function success(message: string): void {
  console.log(`${chalk.green('✓')} ${message}`)
}

/**
 * Failure message
 * ✗ message
 */
export function fail(message: string): void {
  console.log(`${chalk.red('✗')} ${message}`)
}

/**
 * Info line with label
 * label     value
 */
export function info(label: string, value: string): void {
  console.log(`  ${label.padEnd(10)} ${chalk.cyan(value)}`)
}

/**
 * Hint in gray
 */
export function hint(message: string): void {
  console.log(chalk.gray(`  ${message}`))
}

/**
 * Detail line with arrow
 */
export function detail(message: string): void {
  console.log(`    ${chalk.gray('→')} ${message}`)
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
  console.log(`  ${chalk.gray('→')} ${description}: ${chalk.cyan(command)}`)
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
  console.log(`${chalk.yellow('⚠')} ${chalk.cyan(`[${component}]`)} ${chalk.yellow(message)}`)
}

// Namespace export for cleaner imports
export const out = {
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
