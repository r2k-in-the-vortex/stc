// Pure helpers behind the VS Code Testing API provider.
//
// `stc test --list --message-format json` writes one object per discovered
// test with its package, source file, and 1-based declaration line/column
// (CLI and Manifest §1.6). A run with `--message-format json --exact` writes
// one result object per selected test; `-- --nocapture` interleaves the test
// host's own output, which this module attaches to the next result object.
// Nothing here imports `vscode`, so the parsers and argument builders can be
// unit-tested with the esbuild-in-memory pattern.

import { isAbsolute, resolve } from 'node:path';

/** One test reported by `stc test --list --message-format json`. */
export interface DiscoveredTest {
  /** POU name as written. */
  readonly name: string;
  /** Owning package, when the compiler reported one. */
  readonly package: string | undefined;
  /** Package-relative source path with forward slashes. */
  readonly file: string;
  /** 1-based line where the declaration (including its annotation) starts. */
  readonly line: number;
  /** 1-based column where the declaration starts. */
  readonly column: number;
}

/** The parsed `--list` output. */
export interface TestList {
  readonly tests: readonly DiscoveredTest[];
  /** Lines that cannot be used as test entries. */
  readonly problems: readonly string[];
}

/** One assertion failure or synthetic failure reported by the runner. */
export interface TestFailure {
  readonly message: string;
  readonly file: string | undefined;
  readonly line: number | undefined;
  readonly column: number | undefined;
}

/** One test result object from `stc test --message-format json`. */
export interface TestResult {
  readonly name: string;
  readonly status: TestStatus;
  readonly durationMs: number;
  readonly failures: readonly TestFailure[];
  readonly trap: string | undefined;
  /** Test-host stdout captured before this result, without the host line. */
  readonly output: string;
}

/** The stable test status spellings. */
export type TestStatus = 'pass' | 'fail' | 'trap' | 'timeout';

/** One compiler diagnostic printed on stdout in JSON mode. */
export interface TestDiagnostic {
  readonly code: string;
  readonly severity: string;
  readonly message: string;
  readonly file: string | undefined;
  readonly line: number | undefined;
  readonly column: number | undefined;
}

/** The parsed result stream. */
export interface TestResults {
  readonly results: readonly TestResult[];
  readonly diagnostics: readonly TestDiagnostic[];
  /** Result-shaped objects that could not be interpreted. */
  readonly problems: readonly string[];
}

/** One file-level group of discovered tests. */
export interface FileTestGroup {
  readonly file: string;
  readonly package: string | undefined;
  readonly tests: readonly DiscoveredTest[];
}

const STATUSES: ReadonlySet<string> = new Set(['pass', 'fail', 'trap', 'timeout']);

/**
 * Parses `stc test --list --message-format json`.
 *
 * Diagnostic objects (which share stdout in JSON mode) are ignored; an empty
 * output is a valid empty discovery.
 */
export function parseTestList(stdout: string): TestList {
  const tests: DiscoveredTest[] = [];
  const problems: string[] = [];
  for (const line of stdout.split(/\r?\n/)) {
    const text = line.trim();
    if (text === '') {
      continue;
    }
    const value = parseObject(text);
    if (value === undefined) {
      problems.push(`unrecognized --list output: ${truncate(text)}`);
      continue;
    }
    if (isDiagnostic(value)) {
      continue;
    }
    if (typeof value.name !== 'string' || value.name === '') {
      problems.push(`--list entry has no test name: ${truncate(text)}`);
      continue;
    }
    if (typeof value.file !== 'string' || value.file === '') {
      problems.push(
        `test ${value.name} has no source file; update stc so --list reports file, line, and column`,
      );
      continue;
    }
    if (!isPositiveInteger(value.line) || !isPositiveInteger(value.column)) {
      problems.push(
        `test ${value.name} has no 1-based line and column; update stc so --list reports them`,
      );
      continue;
    }
    const packageName =
      typeof value.package === 'string' && value.package !== '' ? value.package : undefined;
    tests.push({
      name: value.name,
      package: packageName,
      file: value.file,
      line: value.line,
      column: value.column,
    });
  }
  return { tests, problems };
}

/**
 * Collects and parses the result stream of one `stc test` run.
 *
 * Lines are processed as they arrive, so a caller can report each result
 * before the process exits. A test host's raw `stc-test-result` line is
 * dropped; the driver's stable result object is authoritative.
 */
export class ResultStream {
  private buffer = '';
  private pending: string[] = [];
  private readonly collected: TestResult[] = [];
  private readonly collectedDiagnostics: TestDiagnostic[] = [];
  private readonly collectedProblems: string[] = [];

  public constructor(private readonly onResult?: (result: TestResult) => void) {}

  /** Feeds one stdout chunk; complete lines are parsed immediately. */
  public push(chunk: string): void {
    this.buffer += chunk;
    for (;;) {
      const newline = this.buffer.indexOf('\n');
      if (newline < 0) {
        return;
      }
      const line = this.buffer.slice(0, newline);
      this.buffer = this.buffer.slice(newline + 1);
      this.handleLine(line);
    }
  }

  /** Finishes parsing, including a final line without a newline. */
  public finish(): TestResults {
    if (this.buffer !== '') {
      this.handleLine(this.buffer);
      this.buffer = '';
    }
    return {
      results: this.collected,
      diagnostics: this.collectedDiagnostics,
      problems: this.collectedProblems,
    };
  }

  private handleLine(raw: string): void {
    const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
    if (line.trim() === '') {
      this.pending.push(line);
      return;
    }
    const value = parseObject(line.trim());
    if (value === undefined) {
      this.pending.push(line);
      return;
    }
    if (value.kind === 'stc-test-result') {
      // The test host's result line duplicates the driver's stable object.
      return;
    }
    if (isResultShaped(value)) {
      const status = value.status;
      if (typeof value.name !== 'string' || typeof status !== 'string' || !STATUSES.has(status)) {
        this.collectedProblems.push(`unrecognized test result: ${truncate(line.trim())}`);
        return;
      }
      const result = parseResult(value, this.pending.join('\n'));
      this.pending = [];
      this.collected.push(result);
      this.onResult?.(result);
      return;
    }
    if (isDiagnostic(value)) {
      this.collectedDiagnostics.push(parseDiagnostic(value));
      return;
    }
    // A test that prints JSON is still test output.
    this.pending.push(line);
  }
}

/** Parses a complete result stream (the `stdout` of one `stc test` run). */
export function parseTestResults(stdout: string): TestResults {
  const stream = new ResultStream();
  stream.push(stdout);
  return stream.finish();
}

/** Parses compiler diagnostics printed on stdout in JSON mode. */
export function parseDiagnostics(stdout: string): TestDiagnostic[] {
  const diagnostics: TestDiagnostic[] = [];
  for (const line of stdout.split(/\r?\n/)) {
    const value = parseObject(line.trim());
    if (value !== undefined && isDiagnostic(value)) {
      diagnostics.push(parseDiagnostic(value));
    }
  }
  return diagnostics;
}

/** Groups discovered tests by source file, preserving discovery order. */
export function groupTestsByFile(tests: readonly DiscoveredTest[]): FileTestGroup[] {
  const groups = new Map<string, { package: string | undefined; tests: DiscoveredTest[] }>();
  for (const test of tests) {
    const existing = groups.get(test.file);
    if (existing === undefined) {
      groups.set(test.file, { package: test.package, tests: [test] });
    } else {
      existing.tests.push(test);
    }
  }
  return [...groups].map(([file, group]) => ({
    file,
    package: group.package,
    tests: group.tests,
  }));
}

/** Builds `stc test --list` arguments for one project manifest. */
export function testListArguments(manifest: string): string[] {
  return [
    'test',
    '--manifest-path',
    manifest,
    '--message-format',
    'json',
    '--color',
    'never',
    '--diagnostic-paths',
    'absolute',
    '--list',
  ];
}

/**
 * Builds the arguments that run exactly the selected tests.
 *
 * `--exact` makes the positional filters complete-name matches, so selecting
 * one test never runs another test whose name merely contains it. `--exact`
 * is ASCII case-insensitive, like the substring filters. `-- --nocapture`
 * interleaves each test host's output so it can be attached to its result;
 * diagnostics use absolute paths so build errors point at real files.
 */
export function testRunArguments(manifest: string, tests: readonly string[]): string[] {
  return [
    'test',
    '--manifest-path',
    manifest,
    '--message-format',
    'json',
    '--color',
    'never',
    '--diagnostic-paths',
    'absolute',
    '--exact',
    ...tests,
    '--',
    '--nocapture',
  ];
}

/** The F-17 launch configuration that debugs one exact test. */
export interface TestDebugConfiguration {
  readonly type: 'stc';
  readonly request: 'launch';
  readonly name: string;
  readonly project: string;
  readonly test: string;
  readonly noBuild: false;
  readonly release: false;
  readonly allowWrite: false;
  readonly allowForce: false;
}

/** Builds the per-test debug launch for the F-17 `test` launch field. */
export function testDebugConfiguration(
  manifest: string,
  test: string,
): TestDebugConfiguration {
  return {
    type: 'stc',
    request: 'launch',
    name: `stc: Debug ${test}`,
    project: manifest,
    test,
    noBuild: false,
    release: false,
    allowWrite: false,
    allowForce: false,
  };
}

/** Resolves a package-relative result path against the manifest directory. */
export function resolveResultPath(root: string, file: string): string {
  return isAbsolute(file) ? file : resolve(root, file);
}

/** Formats one diagnostic for a run output channel or an error message. */
export function formatDiagnostic(diagnostic: TestDiagnostic): string {
  const location =
    diagnostic.file === undefined
      ? ''
      : ` (${diagnostic.file}${diagnostic.line === undefined ? '' : `:${diagnostic.line}`}${
          diagnostic.column === undefined ? '' : `:${diagnostic.column}`
        })`;
  return `${diagnostic.severity}[${diagnostic.code}]: ${diagnostic.message}${location}`;
}

function parseResult(value: Record<string, unknown>, output: string): TestResult {
  const failures: TestFailure[] = [];
  if (Array.isArray(value.failures)) {
    for (const entry of value.failures) {
      if (!isRecord(entry) || typeof entry.message !== 'string') {
        continue;
      }
      failures.push({
        message: entry.message,
        file: typeof entry.file === 'string' ? entry.file : undefined,
        line: isPositiveInteger(entry.line) ? entry.line : undefined,
        column: isPositiveInteger(entry.column) ? entry.column : undefined,
      });
    }
  }
  return {
    name: value.name as string,
    status: value.status as TestStatus,
    durationMs:
      typeof value.duration_ms === 'number' &&
      Number.isFinite(value.duration_ms) &&
      value.duration_ms >= 0
        ? value.duration_ms
        : 0,
    failures,
    trap: typeof value.trap === 'string' ? value.trap : undefined,
    output,
  };
}

function parseDiagnostic(value: Record<string, unknown>): TestDiagnostic {
  const span = isRecord(value.span) ? value.span : undefined;
  return {
    code: value.code as string,
    severity: value.severity as string,
    message: value.message as string,
    file: typeof value.file === 'string' ? value.file : undefined,
    line: span !== undefined && isPositiveInteger(span.line) ? span.line : undefined,
    column: span !== undefined && isPositiveInteger(span.column) ? span.column : undefined,
  };
}

function isDiagnostic(value: Record<string, unknown>): boolean {
  return (
    typeof value.code === 'string' &&
    typeof value.severity === 'string' &&
    typeof value.message === 'string'
  );
}

/**
 * Returns whether an object belongs to the stable result stream rather than
 * being output printed by a test. The stable shape always carries `status`
 * and `name`; the looser `name` check only recognizes objects that also have
 * a result-only field, so a test that prints `{"name": ...}` stays output.
 */
function isResultShaped(value: Record<string, unknown>): boolean {
  return (
    typeof value.status === 'string' ||
    (typeof value.name === 'string' &&
      ('duration_ms' in value || 'failures' in value || 'trap' in value))
  );
}

function parseObject(text: string): Record<string, unknown> | undefined {
  if (!text.startsWith('{')) {
    return undefined;
  }
  try {
    const value: unknown = JSON.parse(text);
    return isRecord(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isPositiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 1;
}

function truncate(text: string): string {
  return text.length <= 120 ? text : `${text.slice(0, 117)}...`;
}
