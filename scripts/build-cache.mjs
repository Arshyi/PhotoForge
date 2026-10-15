// Read-only, conservative storage admission. Never cleans anything; not a disk quota.
import * as fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const GiB = 2 ** 30;
const maxEntries = 2_000_000;

export function readBudget(value, fallback, name) {
  if (value === undefined) return fallback;
  if (!/^(?:\d+(?:\.\d+)?|\.\d+)$/.test(String(value))) throw new Error(`${name} must be a positive GiB number.`);
  const result = Number(value);
  if (!Number.isFinite(result) || result <= 0 || result > 1_000_000) throw new Error(`${name} must be greater than zero and at most 1000000 GiB.`);
  return result;
}

export function inspectBuildCache({ targetDir, baseDir, minFreeGiB = 20, maxCacheGiB = 64, io = fs }) {
  minFreeGiB = readBudget(minFreeGiB, 20, 'PF_MIN_FREE_GIB');
  maxCacheGiB = readBudget(maxCacheGiB, 64, 'PF_CACHE_MAX_GIB');
  if (!targetDir || !baseDir) throw new Error('Explicit target and crate base directories are required.');
  const base = path.resolve(baseDir);
  const target = path.resolve(base, targetDir);
  const relativeBase = path.relative(target, base);
  if (target === path.parse(target).root || relativeBase === '' ||
      (!relativeBase.startsWith(`..${path.sep}`) && relativeBase !== '..' && !path.isAbsolute(relativeBase))) {
    throw new Error('The build target must not be a filesystem root or contain the source crate.');
  }
  let availableAt = target;
  while (!io.existsSync(availableAt)) {
    const parent = path.dirname(availableAt);
    if (parent === availableAt) throw new Error('No existing filesystem ancestor for the target.');
    availableAt = parent;
  }
  if (!io.lstatSync(availableAt).isDirectory()) throw new Error('The target or its existing ancestor is not a directory.');
  const statfs = io.statfsSync(availableAt, { bigint: true });
  const freeBytes = BigInt(statfs.bavail) * BigInt(statfs.bsize);
  let cacheBytes = 0n;
  let files = 0;
  let directories = 0;
  let entries = 0;
  const pending = io.existsSync(target) ? [target] : [];
  while (pending.length) {
    const directory = pending.pop();
    const stat = io.lstatSync(directory);
    if (stat.isSymbolicLink()) throw new Error(`Linked build directory cannot be budgeted safely: ${directory}`);
    if (!stat.isDirectory()) throw new Error(`Build directory changed during inspection: ${directory}`);
    directories++;
    for (const entry of io.readdirSync(directory, { withFileTypes: true })) {
      if (++entries > maxEntries) throw new Error('Build cache inspection exceeded its entry limit; no complete budget check is available.');
      const entryPath = path.join(directory, entry.name);
      const entryStat = io.lstatSync(entryPath, { bigint: true });
      if (entryStat.isSymbolicLink()) throw new Error(`Linked cache entry cannot be budgeted safely: ${entryPath}`);
      if (entryStat.isDirectory()) pending.push(entryPath);
      else if (entryStat.isFile()) { files++; cacheBytes += entryStat.size; }
      else throw new Error(`Unsupported cache entry: ${entryPath}`);
    }
  }
  const violations = [];
  if (Number(freeBytes) < minFreeGiB * GiB) violations.push(`Less than ${minFreeGiB} GiB free on the build volume.`);
  if (Number(cacheBytes) > maxCacheGiB * GiB) violations.push(`Retained build output exceeds ${maxCacheGiB} GiB logical.`);
  return {
    targetDir: target,
    measurement: 'logical file lengths, no hard-link deduplication; pre/post admission, not a filesystem quota',
    cacheBytes: cacheBytes.toString(), freeBytes: freeBytes.toString(),
    cacheGiB: Number(cacheBytes) / GiB, freeGiB: Number(freeBytes) / GiB,
    files, directories, minFreeGiB, maxCacheGiB, violations, allowed: violations.length === 0
  };
}

export function main(args, env = process.env) {
  if (args[0] !== 'check' || args.length !== 5 || args[1] !== '--target-dir' || args[3] !== '--base-dir') {
    throw new Error('Usage: node scripts/build-cache.mjs check --target-dir <directory> --base-dir <crate-directory>');
  }
  const report = inspectBuildCache({
    targetDir: args[2], baseDir: args[4],
    minFreeGiB: readBudget(env.PF_MIN_FREE_GIB, 20, 'PF_MIN_FREE_GIB'),
    maxCacheGiB: readBudget(env.PF_CACHE_MAX_GIB, 64, 'PF_CACHE_MAX_GIB')
  });
  process.stdout.write(`${JSON.stringify(report)}\n`);
  return report.allowed ? 0 : 3;
}

if (process.argv[1] && fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url))) {
  try { process.exitCode = main(process.argv.slice(2)); }
  catch (error) { process.stderr.write(`Build storage check failed: ${error.message}\n`); process.exitCode = 2; }
}
