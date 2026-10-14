import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const wrapper = fileURLToPath(new URL('./cargo-ext.sh', import.meta.url));
const compact = fileURLToPath(new URL('./cargo-compact.toml', import.meta.url));
const gitBash = path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Git', 'bin', 'bash.exe');
const bash = process.env.PF_BASH || (process.platform === 'win32' && existsSync(gitBash) ? gitBash : 'bash');
function runWrapper(args, env = {}) {
  const directory = mkdtempSync(path.join(tmpdir(), 'photoforge-wrapper-'));
  const forward = (value) => value.replaceAll('\\', '/');
  try {
    const fake = path.join(directory, 'fake-cargo.sh');
    writeFileSync(fake, '#!/bin/bash\nprintf \'%s\\0\' "$@" > "$PF_FAKE_ARGS"\nprintf \'%s\' "${CARGO_INCREMENTAL-UNSET}" > "$PF_FAKE_INCREMENTAL"\nprintf \'%s\' "${CARGO_TARGET_DIR-UNSET}" > "$PF_FAKE_TARGET"\nexit "${PF_FAKE_EXIT:-0}"\n', { mode: 0o755 });
    const result = spawnSync(bash, [forward(wrapper), ...args], {
      encoding: 'utf8', timeout: 30_000,
      env: { ...process.env, PF_BUILD_MODE: '', PF_TARGET_DIR: '', CARGO_INCREMENTAL: '', CARGO_TARGET_DIR: '',
        PF_CARGO: forward(fake), PF_FAKE_ARGS: forward(path.join(directory, 'args')),
        PF_FAKE_INCREMENTAL: forward(path.join(directory, 'incremental')),
        PF_FAKE_TARGET: forward(path.join(directory, 'target')), ...env }
    });
    assert.ifError(result.error);
    return { ...result,
      args: existsSync(path.join(directory, 'args')) ? readFileSync(path.join(directory, 'args'), 'utf8').split('\0').slice(0, -1) : null,
      incremental: existsSync(path.join(directory, 'incremental')) ? readFileSync(path.join(directory, 'incremental'), 'utf8') : null,
      target: existsSync(path.join(directory, 'target')) ? readFileSync(path.join(directory, 'target'), 'utf8') : null
    };
  } finally {
    assert.equal(path.dirname(path.resolve(directory)), path.resolve(tmpdir()));
    assert.ok(path.basename(directory).startsWith('photoforge-wrapper-'));
    rmSync(directory, { recursive: true, force: true });
  }
}

test('default compact mode disables incremental even if caller enabled it', () => {
  const result = runWrapper(['test', '--all-features', '--', 'a filter'], { CARGO_INCREMENTAL: '1' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.incremental, '0');
  assert.deepEqual(result.args.slice(-3), ['--all-features', '--', 'a filter']);
  assert.equal(result.args[0], 'test');
  assert.equal(result.args[1], '--config');
  assert.equal(path.basename(result.args[2]), 'cargo-compact.toml');
});

test('full mode preserves caller profile and incremental configuration', () => {
  const result = runWrapper(['clippy', '--all-targets'], { PF_BUILD_MODE: 'full', CARGO_INCREMENTAL: '1' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.incremental, '1');
  assert.deepEqual(result.args, ['clippy', '--all-targets']);
});

test('explicit PF_TARGET_DIR wins over CARGO_TARGET_DIR without splitting spaces', () => {
  const result = runWrapper(['check'], { PF_TARGET_DIR: 'C:/a target/cache', CARGO_TARGET_DIR: 'C:/other' });
  assert.equal(result.target, 'C:/a target/cache');
});

test('Cargo exit codes propagate and invalid modes do not call Cargo', () => {
  assert.equal(runWrapper(['test'], { PF_FAKE_EXIT: '42' }).status, 42);
  const invalid = runWrapper(['test'], { PF_BUILD_MODE: 'comapct' });
  assert.equal(invalid.status, 64);
  assert.equal(invalid.args, null);
  assert.equal(runWrapper([]).status, 64);
});

test('compact policy contains only debug information and incremental settings', () => {
  const policy = readFileSync(compact, 'utf8');
  const assignments = policy.split(/\r?\n/).filter((line) => line.includes('=') && !line.startsWith('#'));
  assert.deepEqual(new Set(assignments.map((line) => line.split('=')[0].trim())), new Set(['debug', 'incremental']));
  assert.doesNotMatch(policy, /\[profile\.(release|bench)/);
  assert.equal((policy.match(/debug = "line-tables-only"/g) || []).length, 4);
  assert.equal((policy.match(/incremental = false/g) || []).length, 2);
});
