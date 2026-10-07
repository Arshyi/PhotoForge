import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import ResourceSettings from './ResourceSettings.svelte';
import { GIB, MIB } from '../resources/budget';
import { statusFixture } from '../resources/testing';
import type { ModeChange, ResourceStatus } from '../resources/types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

let status: ResourceStatus;
let change: (mode: unknown) => [ModeChange, ResourceStatus];
let failStatus: string | null;
let failChange: string | null;

beforeEach(() => {
  status = statusFixture();
  failStatus = null;
  failChange = null;
  change = (mode) => {
    const requested = mode as { mode: 'automatic' } | { mode: 'manual'; bytes: number };
    const bytes = requested.mode === 'manual' ? requested.bytes : 6 * GIB;
    const applied = bytes >= status.budget.bytes;
    const next = applied
      ? statusFixture({}, { mode: requested, bytes, basis: requested.mode === 'manual' ? 'manual' : 'measured' })
      : statusFixture({ reductionPending: true }, { mode: requested });
    status = next;
    return [{ budget: { ...next.budget, bytes: applied ? bytes : status.budget.bytes }, applied }, next];
  };
  vi.mocked(invoke).mockReset().mockImplementation(async (command, options) => {
    if (command === 'resource_status') {
      if (failStatus) throw new Error(failStatus);
      return status;
    }
    if (command === 'set_memory_budget') {
      if (failChange) throw new Error(failChange);
      return change((options as { mode: unknown }).mode);
    }
    return undefined;
  });
});

afterEach(() => vi.restoreAllMocks());

const settings = async () => {
  render(ResourceSettings);
  return screen.findByRole('region', { name: 'Memory' });
};

describe('the memory settings', () => {
  it('shows what the machine has and what PhotoForge may spend, from the backend', async () => {
    const region = await settings();
    const facts = within(region).getByRole('group', { name: /Memory budget: 6\.00 GB/ });
    expect(facts).toBeTruthy();
    const computer = within(region).getByLabelText('This computer');
    const fact = (label: string) => within(computer).getByText(label).parentElement!.textContent!.replace(label, '').trim();
    expect(fact('Installed memory')).toBe('16.0 GB');
    expect(fact('Free now')).toBe('9.00 GB');
    expect(fact('PhotoForge is using')).toBe('280.0 MB'.replace('280.0', '280.0'));
    expect(fact('Free disk space for the cache')).toBe('120.0 GB');
    expect(within(region).getByTestId('basis').textContent).toMatch(/Worked out from this computer/);
    expect(within(region).getByTestId('limits').textContent).toMatch(/about 101 MP/);
  });

  it('says what it could not measure instead of showing zero, and never claims to measure video memory', async () => {
    status = statusFixture({ system: null, process: null, diskFreeBytes: null }, { basis: 'unmeasured', reserveBytes: 0 });
    const region = await settings();
    const computer = within(region).getByLabelText('This computer');
    expect(within(computer).getAllByText('Could not be measured')).toHaveLength(3);
    expect(within(computer).getByText('Free disk space for the cache').parentElement!.textContent).toContain('Unknown');
    expect(within(computer).getByText('Video memory').parentElement!.textContent).toContain('Not measured');
    expect(within(region).getByText(/not reliably measurable/)).toBeTruthy();
    expect(within(region).getByTestId('basis').textContent).toMatch(/could not measure this computer/);
  });

  it('starts on the mode in force, with Apply off until something changes', async () => {
    const region = await settings();
    expect((within(region).getByRole('radio', { name: /Automatic/ }) as HTMLInputElement).checked).toBe(true);
    expect((within(region).getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(region).queryByLabelText('Budget')).toBeNull();
  });

  it('offers a manual budget over exactly the range the backend accepts, and raises it at once', async () => {
    const region = await settings();
    await fireEvent.click(within(region).getByRole('radio', { name: /Manual/ }));
    const slider = within(region).getByLabelText('Budget') as HTMLInputElement;
    expect(Number(slider.min)).toBe(512 * MIB);
    expect(Number(slider.max)).toBeGreaterThan(14 * GIB);
    expect(Number(slider.max)).toBeLessThanOrEqual(14.4 * GIB);
    expect(slider.getAttribute('aria-valuetext')).toBe('6 gigabytes');
    await fireEvent.change(within(region).getByLabelText('Budget in gigabytes'), { target: { value: '8' } });
    const apply = within(region).getByRole('button', { name: 'Apply' }) as HTMLButtonElement;
    expect(apply.disabled).toBe(false);
    await fireEvent.click(apply);
    await waitFor(() => expect(within(region).getByText('Memory budget is now 8.00 GB.')).toBeTruthy());
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('set_memory_budget', { mode: { mode: 'manual', bytes: 8 * GIB } });
    expect((within(region).getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('keeps a typed figure inside the range, whatever was typed', async () => {
    const region = await settings();
    await fireEvent.click(within(region).getByRole('radio', { name: /Manual/ }));
    const typed = within(region).getByLabelText('Budget in gigabytes') as HTMLInputElement;
    await fireEvent.change(typed, { target: { value: '9999' } });
    await fireEvent.click(within(region).getByRole('button', { name: 'Apply' }));
    const sent = vi.mocked(invoke).mock.calls.find(([name]) => name === 'set_memory_budget')![1] as { mode: { bytes: number } };
    expect(sent.mode.bytes).toBeLessThanOrEqual(14.4 * GIB);
    expect(sent.mode.bytes).toBeGreaterThan(14 * GIB);
  });

  it('explains that a lower budget waits for the next document, and says so after saving it', async () => {
    const region = await settings();
    await fireEvent.click(within(region).getByRole('radio', { name: /Manual/ }));
    await fireEvent.change(within(region).getByLabelText('Budget in gigabytes'), { target: { value: '2' } });
    expect(within(region).getByText(/applies when you next open a document/)).toBeTruthy();
    await fireEvent.click(within(region).getByRole('button', { name: 'Apply' }));
    expect(await within(region).findByText(/Saved\. The budget stays at 6\.00 GB while this document is open/)).toBeTruthy();
    expect(within(region).getByText(/A lower budget is saved but not in force yet/)).toBeTruthy();
  });

  it('warns when a manual figure leaves little for Windows, without refusing it', async () => {
    const region = await settings();
    await fireEvent.click(within(region).getByRole('radio', { name: /Manual/ }));
    await fireEvent.change(within(region).getByLabelText('Budget in gigabytes'), { target: { value: '14' } });
    expect(within(region).getByRole('note').textContent).toMatch(/leaves only 2\.00 GB of 16\.0 GB/);
    expect((within(region).getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('returns to automatic', async () => {
    status = statusFixture({}, { mode: { mode: 'manual', bytes: 8 * GIB }, basis: 'manual', bytes: 8 * GIB });
    const region = await settings();
    expect((within(region).getByRole('radio', { name: /Manual/ }) as HTMLInputElement).checked).toBe(true);
    expect((within(region).getByLabelText('Budget') as HTMLInputElement).value).toBe(String(8 * GIB));
    await fireEvent.click(within(region).getByRole('radio', { name: /Automatic/ }));
    await fireEvent.click(within(region).getByRole('button', { name: 'Apply' }));
    await waitFor(() => expect(vi.mocked(invoke)).toHaveBeenCalledWith('set_memory_budget', { mode: { mode: 'automatic' } }));
  });

  it('reports a failure to measure, and a failure to save, as alerts that leave the last good figures up', async () => {
    failStatus = 'The machine could not be queried.';
    render(ResourceSettings);
    expect((await screen.findByRole('alert')).textContent).toContain('could not be queried');
    cleanupAll();

    failStatus = null;
    const region = await settings();
    failChange = 'The settings file could not be written.';
    await fireEvent.click(within(region).getByRole('radio', { name: /Manual/ }));
    await fireEvent.change(within(region).getByLabelText('Budget in gigabytes'), { target: { value: '8' } });
    await fireEvent.click(within(region).getByRole('button', { name: 'Apply' }));
    expect((await within(region).findByRole('alert')).textContent).toContain('could not be written');
    expect(within(region).getByRole('group', { name: /Memory budget: 6\.00 GB/ })).toBeTruthy();
  });

  it('measures again on request', async () => {
    const region = await settings();
    status = statusFixture({ system: { totalPhysical: 16 * GIB, availablePhysical: 3 * GIB } });
    await fireEvent.click(within(region).getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(within(region).getByLabelText('This computer').textContent).toContain('3.00 GB'));
    expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === 'resource_status')).toHaveLength(2);
  });

  it('labels its controls for a screen reader', async () => {
    const region = await settings();
    await fireEvent.click(within(region).getByRole('radio', { name: /Manual/ }));
    expect(within(region).getByRole('group', { name: /Memory budget/ })).toBeTruthy();
    for (const control of region.querySelectorAll('input, select')) {
      const labelled = Boolean(
        control.getAttribute('aria-label') || control.closest('label') || (control.id && region.querySelector(`label[for="${control.id}"]`))
      );
      expect(labelled, control.outerHTML).toBe(true);
    }
  });
});

function cleanupAll() {
  document.body.innerHTML = '';
}
