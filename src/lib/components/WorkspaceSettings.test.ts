import { fireEvent, render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import WorkspaceSettings from './WorkspaceSettings.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

describe('WorkspaceSettings', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.mocked(invoke).mockReset().mockImplementation(async (_command, payload) => (payload as { layout?: unknown; bindings?: unknown }).layout ?? (payload as { bindings?: unknown }).bindings);
  });

  it('renders workspace editor and recent workspace section', () => {
    render(WorkspaceSettings);
    expect(screen.getByText('Workspace layout')).toBeTruthy();
    expect(screen.getByText('Recent workspaces')).toBeTruthy();
  });

  it.each(['Name', 'Left panel', 'Right panel', 'UI scale', 'Collapsed sections'])('labels %s workspace field', (name) => {
    render(WorkspaceSettings); expect(screen.getByText(name)).toBeTruthy();
  });

  it('has keyboard-accessible high contrast control', () => {
    render(WorkspaceSettings); expect(screen.getByRole('checkbox', { name: /High contrast/ })).toBeTruthy();
  });

  it('renders all default configurable shortcuts', () => {
    render(WorkspaceSettings); expect(screen.getAllByLabelText(/shortcut$/)).toHaveLength(10);
  });

  it.each(['Open image', 'Export image', 'Undo', 'Redo', 'Compare', 'Zoom in', 'Zoom out', 'Pixel inspector', 'Crop', 'Straighten'])('renders %s shortcut', (action) => {
    render(WorkspaceSettings); expect(screen.getByLabelText(`${action} shortcut`)).toBeTruthy();
  });

  it('detects edited shortcut conflict', async () => {
    render(WorkspaceSettings);
    const open = screen.getByLabelText('Open image shortcut');
    const save = screen.getByLabelText('Export image shortcut');
    await fireEvent.input(save, { target: { value: (open as HTMLInputElement).value } });
    expect(screen.getByText(/both use/)).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Save shortcuts' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('saves a validated workspace locally', async () => {
    render(WorkspaceSettings);
    await fireEvent.click(screen.getByRole('button', { name: 'Save current workspace' }));
    expect(localStorage.getItem('photoforge.workspaces.v1')).toContain('Professional Editing');
  });

  it('supports settings import and export', () => {
    render(WorkspaceSettings);
    expect(screen.getByRole('button', { name: 'Export layouts & shortcuts' })).toBeTruthy();
    expect(screen.getByText('Import settings')).toBeTruthy();
  });
});
