import assert from 'node:assert/strict';
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const gates = fileURLToPath(new URL('./gates.sh', import.meta.url));
const gitBash = path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Git', 'bin', 'bash.exe');
const bash = process.env.PF_BASH || (process.platform === 'win32' && existsSync(gitBash) ? gitBash : 'bash');
const expectedSteps = ['all', 'nogpu', 'noinference', 'noplugins']
  .flatMap((set) => [`${set}-clippy`, `${set}-test`]);

// Substitute only the cargo boundary. The real gate script must keep running
// every configuration and communicate failures to its caller, even if a cargo
// command succeeds without producing tests or prints a passing partial result.
const fakeCargo = `#!/bin/bash
sub="$1"; shift
set=all
case "$*" in
  *custom-protocol,inference,plugins*) set=nogpu ;;
  *custom-protocol,gpu,plugins*) set=noinference ;;
  *custom-protocol,gpu,inference*) set=noplugins ;;
esac
step="$set-$sub"
echo "$step" >> calls.txt
if [ "$sub" = test ] && [ "$step" != "$GATE_EMPTY_STEP" ]; then
  echo 'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s'
fi
if [ "$step" = "$GATE_FAIL_STEP" ]; then exit 42; fi
`;

function runGate(env = {}) {
  const directory = mkdtempSync(path.join(tmpdir(), 'photoforge-gates-'));
  try {
    copyFileSync(gates, path.join(directory, 'gates.sh'));
    writeFileSync(path.join(directory, 'cargo-ext.sh'), fakeCargo, { mode: 0o755 });
    const result = spawnSync(bash, ['gates.sh', 'logs'], {
      cwd: directory,
      encoding: 'utf8',
      timeout: 30_000,
      env: { ...process.env, GATE_FAIL_STEP: '', GATE_EMPTY_STEP: '', ...env }
    });
    assert.ifError(result.error);
    assert.ok(existsSync(path.join(directory, 'calls.txt')), `${result.stdout}\n${result.stderr}`);
    assert.deepEqual(readFileSync(path.join(directory, 'calls.txt'), 'utf8').trim().split(/\r?\n/), expectedSteps);
    return { ...result, progress: readFileSync(path.join(directory, 'logs', 'progress.txt'), 'utf8') };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

test('all eight successful steps produce a successful gate', () => {
  const result = runGate();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /2 passed, 0 failed, 1 binaries/);
});

test('a failed test command fails the gate despite a passing partial summary', () => {
  const result = runGate({ GATE_FAIL_STEP: 'nogpu-test' });
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.progress, /END nogpu-test rc=42/);
  assert.match(result.progress, /GATES-COMPLETE/);
});

test('a failed clippy command fails the gate even without diagnostic text', () => {
  const result = runGate({ GATE_FAIL_STEP: 'all-clippy' });
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.progress, /END all-clippy rc=42/);
});

test('a successful command without test results cannot qualify a configuration', () => {
  const result = runGate({ GATE_EMPTY_STEP: 'noplugins-test' });
  assert.equal(result.status, 1, result.stderr);
  assert.match(result.stdout, /0 passed, 0 failed, 0 binaries/);
});
