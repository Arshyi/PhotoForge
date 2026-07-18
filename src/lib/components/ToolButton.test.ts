import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import ToolButton from './ToolButton.svelte';

describe('ToolButton', () => {
  it('exposes its accessible label and disabled state', async () => {
    const onclick = vi.fn();
    const view = render(ToolButton, {
      props: { label: 'Export', icon: '⇩', disabled: true, onclick }
    });
    const button = view.getByRole('button', { name: 'Export' }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    await fireEvent.click(button);
    expect(onclick).not.toHaveBeenCalled();
  });
});
