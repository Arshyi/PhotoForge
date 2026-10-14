import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { validateInstallerPolicy } from './installer-policy.mjs';

const template = readFileSync(new URL('../src-tauri/installer/photoforge.wxs', import.meta.url), 'utf8');
const config = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
test('machine-only template preserves NSIS scope, upgrade identity and resources', () => {
  assert.equal(validateInstallerPolicy(template, config).sourcePolicy, 'pass');
});
test('a current-user path-search regression is caught', () => {
  const mutant = template.replace('Id="PreviousMachineInstallDir" Root="HKLM"', 'Id="PreviousMachineInstallDir" Root="HKCU"');
  assert.notEqual(mutant, template);
  assert.throws(() => validateInstallerPolicy(mutant, config), /per-user/);
});
test('removing secure path handling or native registry view is caught', () => {
  assert.throws(() => validateInstallerPolicy(template.replace('Id="INSTALLDIR" Secure="yes"', 'Id="INSTALLDIR"'), config), /secure/);
  assert.throws(() => validateInstallerPolicy(template.replace('Guid="*" Win64="$(var.Win64)"', 'Guid="*"'), config), /native registry view/);
});
test('a shortcut that writes user state is caught', () => {
  const mutant = template.replace('<RegistryValue Root="HKLM"', '<RegistryValue Root="HKCU"');
  assert.throws(() => validateInstallerPolicy(mutant, config), /user hive/);
});
test('shared Desktop directory removal is caught', () => {
  const mutant = template.replace('<Directory Id="DesktopFolder" Name="Desktop" />', '<Directory Id="DesktopFolder" Name="Desktop"><RemoveFolder Id="DesktopFolder" On="uninstall" /></Directory>');
  assert.throws(() => validateInstallerPolicy(mutant, config), /shared Desktop/);
});
test('automatic unsafe legacy migration or loss of its refusal is caught', () => {
  assert.throws(() => validateInstallerPolicy(template.replace('OnlyDetect="yes"', 'OnlyDetect="no"'), config), /legacy MSI/);
  assert.throws(() => validateInstallerPolicy(template.replace('Installed OR NOT PF_LEGACY_MSI', '1'), config), /launch-condition/);
});
test('accidental upgrade-code, installer-scope or notice changes are caught', () => {
  assert.throws(() => validateInstallerPolicy(template.replace('Id="AUTOLAUNCHAPP" Secure="yes"', 'Id="AUTOLAUNCHAPP" Secure="yes" Value="0"'), config), /auto-launch/);
  for (const mutate of [
    (copy) => { copy.bundle.windows.wix.upgradeCode = '00000000-0000-0000-0000-000000000000'; },
    (copy) => { copy.bundle.windows.nsis.installMode = 'perMachine'; },
    (copy) => { delete copy.bundle.resources['installer/Tauri-MIT.txt']; },
    (copy) => { copy.bundle.windows.allowDowngrades = true; }
  ]) {
    const copy = structuredClone(config); mutate(copy);
    assert.throws(() => validateInstallerPolicy(template, copy));
  }
});
test('machine shortcuts require advertisement and one required executable feature', () => {
  assert.throws(() => validateInstallerPolicy(template.replace('Advertise="yes"', 'Advertise="no"'), config), /advertised shortcut/);
  assert.throws(() => validateInstallerPolicy(template.replace('<ComponentRef Id="Path"/>', ''), config), /required MainProgram/);
  assert.throws(() => validateInstallerPolicy(template.replace('<ComponentRef Id="Path"/>', '<ComponentRef Id="Path"/><ComponentRef Id="Path"/>'), config), /required MainProgram/);
});
