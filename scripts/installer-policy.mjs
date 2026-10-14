// Regression lint of our controlled template, not a general XML/security validator.
// Built MSI tables and real install/uninstall remain separate acceptance evidence.
import { readFileSync, realpathSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const upgradeCode = 'da34c5f7-e5bb-583b-93f8-1f4e4065dc14';
function attributes(tag) {
  return Object.fromEntries([...tag.matchAll(/([\w]+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
}
function requirePolicy(value, reason) { if (!value) throw new Error(reason); }

export function validateInstallerPolicy(template, config) {
  requirePolicy(config.bundle?.windows?.wix?.template === 'installer/photoforge.wxs', 'The per-machine template must be selected.');
  requirePolicy(config.bundle.windows.wix.upgradeCode?.toLowerCase() === upgradeCode, 'The existing MSI upgrade code must be preserved.');
  requirePolicy(config.bundle.windows.allowDowngrades === false, 'Future installers must refuse downgrades.');
  requirePolicy(config.bundle.windows.nsis?.installMode === 'currentUser', 'NSIS must remain per-user.');
  requirePolicy(config.bundle.resources?.['installer/Tauri-MIT.txt'] === 'licenses/Tauri-MIT.txt', 'The upstream template notice must be bundled separately.');
  requirePolicy(attributes(template.match(/<Package\b[^>]*>/)?.[0] || '').InstallScope === 'perMachine', 'The MSI must be per-machine.');
  const property = template.match(/<Property\b([^>]*\bId="INSTALLDIR"[^>]*)>([\s\S]*?)<\/Property>/);
  requirePolicy(property && attributes(property[1]).Secure === 'yes', 'INSTALLDIR must be a secure property.');
  const searches = [...property[2].matchAll(/<RegistrySearch\b([^>]*)\/>/g)].map((m) => attributes(m[1]));
  requirePolicy(searches.length === 1, 'INSTALLDIR must have exactly one machine-only path search.');
  requirePolicy(searches[0].Root === 'HKLM' && searches[0].Name === 'InstallDir' && searches[0].Win64 === '$(var.Win64)', 'INSTALLDIR must not inherit a per-user or wrong-view path.');
  const productState = template.slice(0, template.indexOf('{{#if install_webview}}'));
  requirePolicy(!/<Registry(?:Search|Key|Value)\b[^>]*Root="HKCU"/.test(productState), 'MSI product state must never read/write the NSIS user hive.');
  requirePolicy(!/<RemoveFolder\b[^>]*Id="DesktopFolder"/.test(template), 'Uninstall must not remove the shared Desktop directory.');
  for (const id of ['RegistryEntries', 'ApplicationShortcutDesktop', 'ApplicationShortcut', 'CMP_UninstallShortcut']) {
    const component = template.match(new RegExp(`<Component\\b([^>]*\\bId="${id}"[^>]*)>([\\s\\S]*?)<\\/Component>`));
    requirePolicy(component && attributes(component[1]).Win64 === '$(var.Win64)', `${id} must use the native registry view.`);
    requirePolicy(/<Registry(?:Key|Value)\b[^>]*Root="HKLM"/.test(component[2]), `${id} must have a machine-hive registry key path.`);
  }
  const legacy = [...template.matchAll(/<UpgradeVersion\b([^>]*)\/>/g)].map((m) => attributes(m[1])).find((a) => a.Property === 'PF_LEGACY_MSI');
  requirePolicy(legacy?.OnlyDetect === 'yes' && legacy.Maximum === '0.15.1' && legacy.IncludeMaximum === 'yes', 'Unsafe legacy MSI versions must be detected, not automatically removed.');
  requirePolicy(/<Condition\b[^>]*>[\s\S]*?Installed OR NOT PF_LEGACY_MSI[\s\S]*?<\/Condition>/.test(template), 'Legacy migration needs an explicit launch-condition refusal.');
  requirePolicy(template.includes('Source="{{main_binary_path}}"') && template.includes('{{resources}}') && template.includes('{{#each resource_file_ids'), 'Application and resources must remain included.');
  return { sourcePolicy: 'pass', upgradeCode, scope: 'perMachine', nativeLifecycleVerified: false };
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    const root = fileURLToPath(new URL('../', import.meta.url));
    const config = JSON.parse(readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
    const template = readFileSync(path.join(root, 'src-tauri/installer/photoforge.wxs'), 'utf8');
    process.stdout.write(`${JSON.stringify(validateInstallerPolicy(template, config))}\n`);
  } catch (error) { process.stderr.write(`Installer source policy failed: ${error.message}\n`); process.exitCode = 1; }
}
