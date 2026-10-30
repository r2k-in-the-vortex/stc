'use strict';

// Headless VS Code smoke runner for the packaged VSIX.
//
// The runner downloads a VS Code build with `@vscode/test-electron`, installs
// the VSIX into a throwaway extensions directory with the downloaded `code`
// CLI, and launches the editor against the sample workspace with
// `--extensionTestsPath` pointing at `suite.js`. It never assumes a display;
// CI wraps the whole command in `xvfb-run`.
//
// Environment variables:
//   STC_VSIX_PATH            VSIX to install (default: the package's versioned VSIX)
//   STC_SAMPLE_WORKSPACE     sample project to open (default: samples/workspace)
//   STC_VSCODE_VERSION       VS Code version to download (default: stable)
//   STC_ELECTRON_TIMEOUT_MS  overall timeout for the test run (default: 600000)

const { spawn } = require('node:child_process');
const { mkdirSync, rmSync, statSync, writeFileSync } = require('node:fs');
const path = require('node:path');

const {
  downloadAndUnzipVSCode,
  resolveCliPathFromVSCodeExecutablePath,
} = require('@vscode/test-electron');

const extensionRoot = path.resolve(__dirname, '..', '..');
const cachePath = path.join(extensionRoot, '.vscode-test');
const sampleWorkspace = path.resolve(
  process.env.STC_SAMPLE_WORKSPACE ?? path.join(extensionRoot, 'samples', 'workspace'),
);
const vsixPath = path.resolve(process.env.STC_VSIX_PATH ?? defaultVsixPath());
const vscodeVersion = process.env.STC_VSCODE_VERSION ?? 'stable';
const runTimeoutMs = positiveInteger(
  process.env.STC_ELECTRON_TIMEOUT_MS,
  10 * 60 * 1000,
  'STC_ELECTRON_TIMEOUT_MS',
);
const profileDir = path.join(cachePath, 'smoke');
const extensionsDir = path.join(profileDir, 'extensions');
const userDataDir = path.join(profileDir, 'user-data');
const devExtensionDir = path.join(profileDir, 'dev-extension');
const suitePath = path.join(__dirname, 'suite.js');

/** The VSIX `npm run package` writes next to `package.json`. */
function defaultVsixPath() {
  const { version } = require(path.join(extensionRoot, 'package.json'));
  return path.join(extensionRoot, `stc-vscode-${version}.vsix`);
}

function positiveInteger(value, fallback, name) {
  if (value === undefined || value === '') {
    return fallback;
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    fail(`${name} must be a positive integer, got ${JSON.stringify(value)}`);
  }
  return parsed;
}

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function requireFile(candidate, description, hint) {
  let stats;
  try {
    stats = statSync(candidate);
  } catch {
    fail(`${description} was not found at ${candidate}${hint ? ` (${hint})` : ''}`);
  }
  if (!stats.isFile()) {
    fail(`${description} at ${candidate} is not a file`);
  }
}

function requireDirectory(candidate, description) {
  let stats;
  try {
    stats = statSync(candidate);
  } catch {
    fail(`${description} was not found at ${candidate}`);
  }
  if (!stats.isDirectory()) {
    fail(`${description} at ${candidate} is not a directory`);
  }
}

/**
 * Runs a child process and fails the smoke if it exits non-zero or exceeds
 * `timeoutMs`. The timeout kills the whole process tree so VS Code cannot
 * leave an extension host behind.
 */
function run(command, args, options) {
  const { timeoutMs, shell = false, env = process.env } = options ?? {};
  return new Promise((resolve, reject) => {
    const detached = process.platform !== 'win32';
    const child = spawn(command, args, {
      stdio: 'inherit',
      shell,
      detached,
      env,
      windowsHide: true,
    });
    let settled = false;
    let timedOut = false;
    const timer =
      timeoutMs === undefined
        ? undefined
        : setTimeout(() => {
            timedOut = true;
            killTree(child);
          }, timeoutMs);
    const finish = (error) => {
      if (settled) {
        return;
      }
      settled = true;
      if (timer !== undefined) {
        clearTimeout(timer);
      }
      if (error !== undefined) {
        reject(error);
      } else {
        resolve();
      }
    };
    child.on('error', (error) => finish(error));
    child.on('exit', (code, signal) => {
      if (timedOut) {
        finish(new Error(`${command} timed out after ${timeoutMs}ms`));
      } else if (code !== 0) {
        finish(new Error(`${command} exited with ${code === null ? signal : `code ${code}`}`));
      } else {
        finish();
      }
    });
  });
}

function killTree(child) {
  if (child.pid === undefined) {
    return;
  }
  if (process.platform === 'win32') {
    spawn('taskkill', ['/pid', String(child.pid), '/t', '/f'], {
      stdio: 'ignore',
      windowsHide: true,
    });
    return;
  }
  try {
    process.kill(-child.pid, 'SIGKILL');
  } catch {
    child.kill('SIGKILL');
  }
}

/**
 * A fresh editor must not inherit the host's Electron/VS Code mode variables.
 * When this script is run from a VS Code integrated terminal (or another
 * extension host), `ELECTRON_RUN_AS_NODE` makes the downloaded executable run
 * as plain Node and treat the workspace path as a module.
 */
function childEnvironment() {
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  for (const key of Object.keys(env)) {
    if (key.startsWith('VSCODE_')) {
      delete env[key];
    }
  }
  return env;
}

async function main() {
  requireFile(
    vsixPath,
    'the VSIX',
    'run `npm run package` first or set STC_VSIX_PATH',
  );
  requireDirectory(sampleWorkspace, 'the sample workspace');
  requireFile(
    path.join(sampleWorkspace, 'stc.toml'),
    "the sample workspace's stc.toml",
    'set STC_SAMPLE_WORKSPACE to a project directory',
  );
  requireFile(suitePath, 'the extension test suite');

  // A fresh profile makes the run deterministic: no extension or user state
  // from an earlier run can be picked up.
  rmSync(profileDir, { recursive: true, force: true });
  mkdirSync(extensionsDir, { recursive: true });
  mkdirSync(userDataDir, { recursive: true });

  // VS Code only enters CLI extension-test mode when the window is also an
  // extension development host (`isExtensionDevelopment` plus
  // `extensionTestsLocationURI`). A dummy, entry-point-free development
  // extension satisfies that flag without shadowing the installed VSIX, so
  // the extension under test is the one `--install-extension` unpacked.
  mkdirSync(devExtensionDir, { recursive: true });
  writeFileSync(
    path.join(devExtensionDir, 'package.json'),
    `${JSON.stringify(
      {
        name: 'stc-smoke-host',
        publisher: 'stc-local',
        version: '0.0.0',
        engines: { vscode: '>=1.82.0' },
      },
      null,
      2,
    )}\n`,
  );

  console.log(`Downloading VS Code ${vscodeVersion} (cache: ${cachePath})`);
  const vscodeExecutablePath = await downloadAndUnzipVSCode({
    version: vscodeVersion,
    cachePath,
  });

  const cliPath = resolveCliPathFromVSCodeExecutablePath(vscodeExecutablePath);
  const cliIsShellScript = process.platform === 'win32' && cliPath.endsWith('.cmd');
  const cli = cliIsShellScript ? `"${cliPath}"` : cliPath;
  const env = childEnvironment();
  console.log(`Installing ${vsixPath} into ${extensionsDir}`);
  await run(
    cli,
    ['--install-extension', vsixPath, '--extensions-dir', extensionsDir, '--force'],
    { shell: cliIsShellScript, env, timeoutMs: 5 * 60 * 1000 },
  );

  const args = [
    sampleWorkspace,
    `--extensionDevelopmentPath=${devExtensionDir}`,
    `--extensionTestsPath=${suitePath}`,
    `--extensions-dir=${extensionsDir}`,
    `--user-data-dir=${userDataDir}`,
    '--disable-workspace-trust',
    '--disable-updates',
    '--skip-welcome',
    '--skip-release-notes',
    '--no-cached-data',
    // Mirrors the flags `@vscode/test-electron`'s runTests passes; a headless
    // runner has no GPU and no usable Chromium sandbox.
    '--no-sandbox',
    '--disable-gpu-sandbox',
    '--disable-gpu',
  ];
  console.log(`Running the extension tests against ${sampleWorkspace}`);
  await run(vscodeExecutablePath, args, { env, timeoutMs: runTimeoutMs });
  console.log('VS Code extension smoke passed');

  // Keep failure evidence (especially user-data/logs) for CI to upload.
  rmSync(profileDir, { recursive: true, force: true });
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
