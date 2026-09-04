import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import StatusBar from './StatusBar.svelte';

describe('StatusBar activity', () => {
  it.each([
    [{ exporting: true, refining: true, opening: true, rendering: true }, 'Export in progress'],
    [{ refining: true, opening: true, rendering: true }, 'Refining selection'],
    [{ opening: true, rendering: true }, 'Opening image'],
    [{ rendering: true }, 'Rendering preview'],
    [{ isCurrent: false }, 'Preview queued'],
    [{}, 'Preview current']
  ])('reports one truthful prioritized activity %#', (props, label) => {
    const view = render(StatusBar, { props });
    expect(view.getByText(label)).toBeTruthy();
    expect(view.container.querySelector('footer')?.getAttribute('aria-busy')).toBe(label === 'Preview current' ? 'false' : 'true');
  });
});
