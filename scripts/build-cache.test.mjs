import assert from 'node:assert/strict';
import * as fs from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { inspectBuildCache, readBudget } from './build-cache.mjs';

const script = fileURLToPath(new URL('./build-cache.mjs', import.meta.url));
function fixture(run) {
  const directory = fs.mkdtempSync(path.join(tmpdir(), 'photoforge-cache-policy-'));
  try { return run(directory); }
  finally {
    assert.equal(path.dirname(path.resolve(directory)), path.resolve(tmpdir()));
    assert.ok(path.basename(directory).startsWith('photoforge-cache-policy-'));
    fs.rmSync(directory, { recursive: true, force: true });
  }
}
const fakeVolume = (bytes) => ({ ...fs, statfsSync: () => ({ bavail: BigInt(bytes), bsize: 1n }) });
const inspect = (base, options = {}) => inspectBuildCache({
  baseDir: path.join(base, 'crate'), targetDir: path.join(base, 'target'),
  io: fakeVolume(100 * 2 ** 30), ...options
});

test('a new cache is inspected without creating anything', () => fixture((base) => {
  const before = fs.readdirSync(base);
  const report = inspect(base);
  assert.equal(report.cacheBytes, '0');
  assert.equal(report.allowed, true);
  assert.deepEqual(fs.readdirSync(base), before);
}));

test('logical bytes include debug and release without modifying either', () => fixture((base) => {
  fs.mkdirSync(path.join(base, 'target/debug'), { recursive: true });
  fs.mkdirSync(path.join(base, 'target/release'));
  fs.writeFileSync(path.join(base, 'target/debug/object'), '12345');
  fs.writeFileSync(path.join(base, 'target/release/app'), 'abc');
  const report = inspect(base);
  assert.equal(report.cacheBytes, '8');
  assert.equal(report.files, 2);
  assert.equal(fs.readFileSync(path.join(base, 'target/release/app'), 'utf8'), 'abc');
}));

test('low free space refuses admission, even with an empty cache', () => fixture((base) => {
  const report = inspect(base, { io: fakeVolume(19 * 2 ** 30) });
  assert.equal(report.allowed, false);
  assert.match(report.violations[0], /Less than 20/);
}));

test('over-budget data is reported and never deleted', () => fixture((base) => {
  fs.mkdirSync(path.join(base, 'target'));
  const file = path.join(base, 'target/keep');
  fs.writeFileSync(file, 'x'.repeat(2048));
  assert.equal(inspect(base, { maxCacheGiB: .000001 }).allowed, false);
  assert.equal(fs.statSync(file).size, 2048);
}));

test('relative target paths are relative to Cargo crate directory', () => fixture((base) => {
  assert.equal(inspect(base, { targetDir: 'local-target' }).targetDir, path.join(base, 'crate/local-target'));
}));

test('filesystem roots and source-containing targets are rejected', () => fixture((base) => {
  for (const targetDir of [path.parse(base).root, base, path.join(base, 'crate')]) {
    assert.throws(() => inspect(base, { targetDir }), /filesystem root|source crate/);
  }
}));

test('linked cache directories refuse inspection rather than follow outside data', () => fixture((base) => {
  fs.mkdirSync(path.join(base, 'target'));
  fs.mkdirSync(path.join(base, 'outside'));
  fs.writeFileSync(path.join(base, 'outside/keep'), 'untouched');
  fs.symlinkSync(path.join(base, 'outside'), path.join(base, 'target/link'), process.platform === 'win32' ? 'junction' : 'dir');
  assert.throws(() => inspect(base), /Linked cache entry/);
  assert.equal(fs.readFileSync(path.join(base, 'outside/keep'), 'utf8'), 'untouched');
}));

test('failed volume or directory reads do not produce a passing check', () => fixture((base) => {
  assert.throws(() => inspect(base, { io: { ...fs, statfsSync() { throw new Error('denied'); } } }), /denied/);
  fs.mkdirSync(path.join(base, 'target'));
  assert.throws(() => inspect(base, { io: { ...fakeVolume(100 * 2 ** 30), readdirSync() { throw new Error('changed'); } } }), /changed/);
}));

test('invalid budgets cannot silently disable retention checks', () => {
  for (const value of ['', '0', '-1', 'NaN', 'Infinity', '1e9', '1000001', ' 20']) {
    assert.throws(() => readBudget(value, 20, 'budget'));
  }
  assert.equal(readBudget(undefined, 20, 'budget'), 20);
  assert.equal(readBudget('0.5', 20, 'budget'), .5);
});

test('CLI exits nonzero on refusal or invalid options; its command set has no deletion', () => fixture((base) => {
  const args = [script, 'check', '--target-dir', path.join(base, 'target'), '--base-dir', path.join(base, 'crate')];
  const result = spawnSync(process.execPath, args, { encoding: 'utf8', env: { ...process.env, PF_MIN_FREE_GIB: '1000000' } });
  assert.ifError(result.error);
  assert.equal(result.status, 3, result.stderr);
  assert.equal(JSON.parse(result.stdout).allowed, false);
  const invalid = spawnSync(process.execPath, [script, 'clean'], { encoding: 'utf8' });
  assert.equal(invalid.status, 2);
  assert.match(invalid.stderr, /Usage/);
  assert.deepEqual(fs.readdirSync(base), []);
}));

test('a linked entry-point path cannot silently skip the storage check', () => fixture((base) => {
  fs.mkdirSync(path.join(base, 'real'));
  fs.copyFileSync(script, path.join(base, 'real/build-cache.mjs'));
  fs.symlinkSync(path.join(base, 'real'), path.join(base, 'alias'), process.platform === 'win32' ? 'junction' : 'dir');
  const result = spawnSync(process.execPath, [path.join(base, 'alias/build-cache.mjs'), 'check',
    '--target-dir', path.join(base, 'target'), '--base-dir', path.join(base, 'crate')], {
    encoding: 'utf8', env: { ...process.env, PF_MIN_FREE_GIB: '1000000' }
  });
  assert.ifError(result.error);
  assert.equal(result.status, 3, result.stderr);
  assert.equal(JSON.parse(result.stdout).allowed, false);
}));
