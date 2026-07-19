import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import LocalAiPrivacy from './LocalAiPrivacy.svelte';

describe('LocalAiPrivacy', () => {
  it('names the dedicated Local AI Privacy page', () => {
    const view = render(LocalAiPrivacy);
    expect(view.getByRole('heading', { name: 'Local AI Privacy' })).toBeTruthy();
  });

  it('states that no images or cloud services receive data', () => {
    const view = render(LocalAiPrivacy);
    expect(view.getByText('No images leave the computer.')).toBeTruthy();
    expect(view.getByText('No cloud services are contacted.')).toBeTruthy();
  });

  it('lists excluded sensitive inputs explicitly', () => {
    const view = render(LocalAiPrivacy);
    expect(view.getByText(/paths, usernames, configuration/)).toBeTruthy();
    expect(view.getByText(/environment variables are never sent/)).toBeTruthy();
  });

  it('shows that validation and review precede deterministic execution', () => {
    const view = render(LocalAiPrivacy);
    const flow = view.getByLabelText('Local planner security flow');
    expect(flow.textContent).toContain('Strict validation');
    expect(flow.textContent).toContain('Review');
    expect(flow.textContent).toContain('Deterministic engine');
  });
});
