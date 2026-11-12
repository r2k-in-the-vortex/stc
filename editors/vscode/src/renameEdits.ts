/**
 * Pure helpers for the VS Code tracked-rename controller.
 *
 * The private `stc/rename` response is a standard LSP `WorkspaceEdit`; the
 * controller must know what the edited text will look like after VS Code
 * applies it so it can confirm the apply before recording history. This module
 * converts LSP UTF-16 positions to string offsets, applies a file's edits, and
 * classifies edited URIs for refresh handling.
 *
 * This module is intentionally free of `vscode` imports so it can be
 * unit-tested with the existing esbuild test harness.
 */

/** An LSP position (UTF-16 line and character). */
export interface LspPosition {
  readonly line: number;
  readonly character: number;
}

/** An LSP range. */
export interface LspRange {
  readonly start: LspPosition;
  readonly end: LspPosition;
}

/** One LSP text edit. */
export interface LspTextEdit {
  readonly range: LspRange;
  readonly newText: string;
}

/** A `WorkspaceEdit.changes` object from the private rename response. */
export interface LspWorkspaceEdit {
  readonly changes?: Record<string, readonly LspTextEdit[]>;
}

/** How one edited URI participates in the post-apply refresh. */
export type EditedUriKind = 'source' | 'manifest' | 'profile';

/**
 * Applies one file's LSP edits to its current text.
 *
 * LSP positions are UTF-16 offsets, which is also JavaScript's string unit, so
 * a character offset is a string index. Edits are validated against overlap and
 * applied from the end so earlier offsets stay valid.
 *
 * Throws on an out-of-range position or overlapping edits.
 */
export function applyLspEdits(text: string, edits: readonly LspTextEdit[]): string {
  if (edits.length === 0) {
    return text;
  }
  const lineStarts = computeLineStarts(text);
  const ranges = edits.map((edit) => {
    const start = offsetAt(text, lineStarts, edit.range.start);
    const end = offsetAt(text, lineStarts, edit.range.end);
    if (start > end) {
      throw new Error(`rename edit starts after it ends (${start} > ${end})`);
    }
    return { start, end, newText: edit.newText };
  });
  ranges.sort((left, right) => left.start - right.start || left.end - right.end);
  for (let index = 1; index < ranges.length; index += 1) {
    const previous = ranges[index - 1];
    const current = ranges[index];
    if (current.start < previous.end) {
      throw new Error(
        `rename edits overlap (${previous.start}..${previous.end} and ` +
          `${current.start}..${current.end})`,
      );
    }
  }
  let result = text;
  for (let index = ranges.length - 1; index >= 0; index -= 1) {
    const range = ranges[index];
    result =
      result.slice(0, range.start) + range.newText + result.slice(range.end);
  }
  return result;
}

/**
 * Returns true only when the server advertised
 * `capabilities.experimental.stc.trackedRename === true`.
 */
export function shouldUseTrackedRename(initializeResult: unknown): boolean {
  const capabilities = objectProperty(initializeResult, 'capabilities');
  const experimental = objectProperty(capabilities, 'experimental');
  const stc = objectProperty(experimental, 'stc');
  return stc?.trackedRename === true;
}

/**
 * Classifies one edited URI as a source file, the root manifest, or the
 * selected profile.
 *
 * The manifest is identified by path so a differently named project manifest
 * would still classify correctly; every other edited TOML file is the profile.
 */
export function editedUriKind(uri: string, manifestPath: string): EditedUriKind {
  const path = uriPath(uri);
  if (/\.st$/i.test(path)) {
    return 'source';
  }
  if (samePath(path, manifestPath)) {
    return 'manifest';
  }
  return 'profile';
}

/** Returns the line start offsets; a trailing newline starts an empty line. */
function computeLineStarts(text: string): number[] {
  const starts = [0];
  for (let index = 0; index < text.length; index += 1) {
    if (text.charCodeAt(index) === 10) {
      starts.push(index + 1);
    }
  }
  return starts;
}

/** Converts one LSP position to a string offset, rejecting out-of-range input. */
function offsetAt(
  text: string,
  lineStarts: readonly number[],
  position: LspPosition,
): number {
  if (!Number.isSafeInteger(position.line) || position.line < 0) {
    throw new Error(`rename edit has an invalid line ${position.line}`);
  }
  if (position.line >= lineStarts.length) {
    throw new Error(
      `rename edit line ${position.line} is outside the file (${lineStarts.length} lines)`,
    );
  }
  if (!Number.isSafeInteger(position.character) || position.character < 0) {
    throw new Error(`rename edit has an invalid character ${position.character}`);
  }
  const start = lineStarts[position.line];
  const nextLine =
    position.line + 1 < lineStarts.length ? lineStarts[position.line + 1] : text.length;
  let contentEnd = nextLine;
  if (position.line + 1 < lineStarts.length) {
    // Exclude the newline, and the `\r` of a CRLF pair, from the line length.
    contentEnd -= 1;
    if (contentEnd > start && text.charCodeAt(contentEnd - 1) === 13) {
      contentEnd -= 1;
    }
  }
  const offset = start + position.character;
  if (offset > contentEnd) {
    throw new Error(
      `rename edit character ${position.character} is outside line ${position.line}`,
    );
  }
  return offset;
}

function objectProperty(value: unknown, name: string): Record<string, unknown> | undefined {
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }
  const property = (value as Record<string, unknown>)[name];
  if (typeof property !== 'object' || property === null) {
    return undefined;
  }
  return property as Record<string, unknown>;
}

/** Converts a file URI to a filesystem path; non-file URIs are kept verbatim. */
function uriPath(uri: string): string {
  let text = uri;
  if (text.startsWith('file:')) {
    try {
      const url = new URL(text);
      text = decodeURIComponent(url.pathname);
    } catch {
      // Fall back to the raw spelling below.
    }
  }
  if (/^\/[a-zA-Z]:/.test(text)) {
    text = text.slice(1);
  }
  return text.replace(/\\/g, '/');
}

function samePath(left: string, right: string): boolean {
  const normalizedLeft = normalizePath(left);
  const normalizedRight = normalizePath(right);
  return process.platform === 'win32'
    ? normalizedLeft.toLowerCase() === normalizedRight.toLowerCase()
    : normalizedLeft === normalizedRight;
}

function normalizePath(value: string): string {
  return value.replace(/\\/g, '/');
}
