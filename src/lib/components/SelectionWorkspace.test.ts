import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import { createSelectionState, setActiveMask } from '../selections/state';
import SelectionWorkspace from './SelectionWorkspace.svelte';

function props() {
  return {
    state: createSelectionState('doc'),
    onstatechange: vi.fn(),
    onoperation: vi.fn(),
    onnamedaction: vi.fn(),
    onimport: vi.fn(),
    onundo: vi.fn(),
    onredo: vi.fn(),
    oncancel: vi.fn()
  };
}

describe('SelectionWorkspace', () => {
  it('switches tools and composition modes', async () => {
    const value = props();
    render(SelectionWorkspace, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Polygon lasso' }));
    expect(value.onstatechange).toHaveBeenLastCalledWith(expect.objectContaining({ tool: 'polygon' }), undefined);
    await fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    expect(value.onstatechange).toHaveBeenLastCalledWith(expect.objectContaining({ mode: 'add' }), undefined);
  });

  it('disables masked adjustment scope without an active selection', () => {
    render(SelectionWorkspace, { props: props() });
    expect((screen.getByRole('option', { name: 'Inside selection' }) as HTMLOptionElement).disabled).toBe(true);
    expect(screen.getByText(/No active selection/)).toBeTruthy();
  });

  it('runs mask operations and saves the active mask', async () => {
    const value = props();
    value.state = setActiveMask(value.state, {
      version: 1,
      width: 1,
      height: 1,
      encoding: 'base64_u8',
      data: '/w',
      checksum: 'fnv1a64:0123456789abcdef'
    }, {
      width: 1,
      height: 1,
      selectedPixels: 1,
      fullySelectedPixels: 1,
      averageCoverage: 1,
      bounds: [0, 0, 1, 1],
      memoryBytes: 1
    });
    render(SelectionWorkspace, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Invert' }));
    expect(value.onoperation).toHaveBeenCalledWith({ type: 'invert' });
    await fireEvent.click(screen.getByRole('button', { name: 'Save active' }));
    expect(value.onnamedaction).toHaveBeenCalledWith('create', undefined, '');
  });

  it('exposes refinement controls honestly as mask-only processing', async () => {
    const value = props();
    value.state.activeMask = {
      version: 1,
      width: 1,
      height: 1,
      encoding: 'base64_u8',
      data: '/w',
      checksum: 'fnv1a64:0123456789abcdef'
    };
    render(SelectionWorkspace, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: /Refine selection/ }));
    expect(screen.getByText(/alter only mask coverage/i)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Apply refinement' }));
    expect(value.onoperation).toHaveBeenCalledWith(expect.objectContaining({ type: 'refine' }));
  });
});
