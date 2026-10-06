import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import OversizedSourceDialog from './OversizedSourceDialog.svelte';
import { admissionFixture } from '../source/testing';
import type { SourceAdmission, SourcePreview } from '../source/types';

afterEach(cleanup);

const PREVIEW: SourcePreview = { dataUrl: 'data:image/png;base64,AAAA', width: 600, height: 450 };

function open(admission: SourceAdmission, loadPreview = vi.fn().mockResolvedValue(PREVIEW)) {
  const onopen = vi.fn();
  const oncancel = vi.fn();
  const cancelPreview = vi.fn().mockResolvedValue(undefined);
  const view = render(OversizedSourceDialog, { admission, onopen, oncancel, loadPreview, cancelPreview });
  return { ...view, onopen, oncancel, loadPreview, cancelPreview };
}

describe('oversized source dialog', () => {
  it('explains the refusal with real figures and offers a region and a reduced copy', () => {
    open(admissionFixture());
    const text = screen.getByTestId('explanation').textContent ?? '';
    // The whole-image cost and the budget are stated, not hidden.
    expect(text).toMatch(/4\.7 GB|4\.66 GB|5\.0 GB|4\.\d+ GB/);
    expect(text).toMatch(/2\.\d+ GB|2\.4 GB|2\.6 GB/);
    expect(screen.getByRole('button', { name: /Choose Region/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Open Reduced Copy/ })).toBeTruthy();
    expect(screen.getByText(/12,000 × 9,000 px/)).toBeTruthy();
  });

  /** Only options that genuinely exist are shown. */
  it('never offers opening whole, or out-of-core, as a button', () => {
    open(admissionFixture());
    expect(screen.queryByRole('button', { name: /Open Full Resolution/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /Out-of-Core/ })).toBeNull();
    // It is explained instead, so the absence is not a mystery.
    expect(screen.getByText(/out of core\?/)).toBeTruthy();
    expect(screen.getByText(/is not available: every stage holds complete buffers/)).toBeTruthy();
  });

  it('offers only a reduced copy for a format that cannot open a part at a time', () => {
    open(
      admissionFixture({
        verdict: { kind: 'reducedCopyRecommended' },
        options: [{ kind: 'openReduced', maxScale: 0.3, decode: { kind: 'dctScaled' } }]
      })
    );
    expect(screen.queryByRole('button', { name: /Choose Region/ })).toBeNull();
    expect(screen.getByRole('button', { name: /Open Reduced Copy/ })).toBeTruthy();
    expect(screen.getByTestId('explanation').textContent).toMatch(/cannot be opened a part at a time/);
  });

  it('says plainly when nothing bounded fits', () => {
    open(
      admissionFixture({
        verdict: { kind: 'insufficientResources', shortfall: 'budget' },
        options: []
      })
    );
    expect(screen.getByTestId('no-options')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Choose Region|Reduced Copy/ })).toBeNull();
    expect(screen.getByTestId('explanation').textContent).toMatch(/Raising the budget/);
  });

  it('distinguishes a momentary shortage of free memory from a limit', () => {
    open(
      admissionFixture({
        verdict: { kind: 'insufficientResources', shortfall: 'momentary' },
        options: []
      })
    );
    const text = screen.getByTestId('explanation').textContent ?? '';
    expect(text).toMatch(/not enough free memory right now/);
    expect(text).toMatch(/Closing other applications/);
  });

  it('refuses a file whose header describes an impossible image, and says why', () => {
    for (const [refusal, pattern] of [
      [{ kind: 'empty' as const }, /no pixels/],
      [
        { kind: 'implausibleDimensions' as const, width: 4_000_000_000, height: 4_000_000_000 },
        /no image can have/
      ],
      [{ kind: 'impossibleExpansion' as const, claimedBytes: 4e10, fileBytes: 60 }, /not a valid image/]
    ] as const) {
      cleanup();
      open(admissionFixture({ verdict: { kind: 'unsafe', refusal }, options: [] }));
      expect(screen.getByTestId('explanation').textContent).toMatch(pattern);
      expect(screen.queryByRole('button', { name: /Choose Region|Reduced Copy/ })).toBeNull();
    }
  });

  describe('choosing a region', () => {
    it('loads a bounded preview and shows the selector', async () => {
      // A promise that stays pending, so the loading state can be observed.
      let resolve!: (value: SourcePreview) => void;
      const loadPreview = vi.fn().mockReturnValue(new Promise<SourcePreview>((r) => (resolve = r)));
      open(admissionFixture(), loadPreview);
      await fireEvent.click(screen.getByRole('button', { name: /Choose Region/ }));
      expect(loadPreview).toHaveBeenCalledExactlyOnceWith(String.raw`C:\scans\wall.png`, 1400);
      expect(screen.getByText(/Preparing a bounded preview/)).toBeTruthy();
      // The selector is usable while the preview is still being made.
      expect(screen.getByRole('button', { name: /Working region/ })).toBeTruthy();
      resolve(PREVIEW);
      await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
      expect(screen.queryByText(/Preparing a bounded preview/)).toBeNull();
    });

    it('opens the chosen region', async () => {
      const { onopen } = open(admissionFixture());
      await fireEvent.click(screen.getByRole('button', { name: /Choose Region/ }));
      await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
      await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
      expect(onopen).toHaveBeenCalledOnce();
      const selection = onopen.mock.calls[0][0];
      expect(selection.kind).toBe('region');
      // The default is the largest region that fits, which is within the offer.
      expect(selection.rect.width * selection.rect.height).toBeLessThanOrEqual(20_000_000);
      expect(selection.rect.width * selection.rect.height).toBeGreaterThan(19_000_000);
    });

    it('still lets the user enter a region when the preview cannot be made', async () => {
      const { onopen } = open(admissionFixture(), vi.fn().mockRejectedValue(new Error('not enough memory')));
      await fireEvent.click(screen.getByRole('button', { name: /Choose Region/ }));
      await waitFor(() => expect(screen.getByText(/not enough memory/)).toBeTruthy());
      expect(screen.getByText(/Enter the region numerically/)).toBeTruthy();
      await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
      expect(onopen).toHaveBeenCalledOnce();
    });

    it('stops the preview when the user goes back, because it holds the job gate', async () => {
      let resolve!: (value: SourcePreview) => void;
      const slow = vi.fn().mockReturnValue(new Promise<SourcePreview>((r) => (resolve = r)));
      const { cancelPreview } = open(admissionFixture(), slow);
      await fireEvent.click(screen.getByRole('button', { name: /Choose Region/ }));
      await fireEvent.click(screen.getByRole('button', { name: 'Back' }));
      expect(cancelPreview).toHaveBeenCalledOnce();
      expect(screen.getByRole('button', { name: /Choose Region/ })).toBeTruthy();
      // A late result for a screen that is gone must not resurrect it.
      resolve(PREVIEW);
      await Promise.resolve();
      expect(screen.queryByAltText(/Reduced preview/)).toBeNull();
    });

    it('stops the preview when the dialog is cancelled', async () => {
      const slow = vi.fn().mockReturnValue(new Promise<SourcePreview>(() => undefined));
      const { cancelPreview, oncancel } = open(admissionFixture(), slow);
      await fireEvent.click(screen.getByRole('button', { name: /Choose Region/ }));
      await fireEvent.click(screen.getByRole('button', { name: 'Cancel opening this image' }));
      expect(cancelPreview).toHaveBeenCalled();
      expect(oncancel).toHaveBeenCalledOnce();
    });
  });

  describe('opening a reduced copy', () => {
    it('states that the copy has less resolution than the file', async () => {
      open(admissionFixture());
      await fireEvent.click(screen.getByRole('button', { name: /Open Reduced Copy/ }));
      expect(screen.getByText(/less resolution than the file/)).toBeTruthy();
    });

    it('offers the largest scale that fits and only rounder scales below it', async () => {
      open(admissionFixture());
      await fireEvent.click(screen.getByRole('button', { name: /Open Reduced Copy/ }));
      const labels = screen.getAllByRole('radio').map((radio) => radio.parentElement?.textContent?.trim());
      expect(labels[0]).toMatch(/Largest that fits \(40%\)/);
      // The fixture's maximum is 40%, so 50% must not be offered.
      expect(labels).not.toContain('50%');
      expect(labels).toContain('25%');
      expect(labels).toContain('10%');
    });

    it('shows the resulting size and opens exactly that', async () => {
      const { onopen } = open(admissionFixture());
      await fireEvent.click(screen.getByRole('button', { name: /Open Reduced Copy/ }));
      expect(screen.getByTestId('reduced-size').textContent).toMatch(/4,800 × 3,600 px/);
      await fireEvent.click(screen.getByLabelText('25%'));
      expect(screen.getByTestId('reduced-size').textContent).toMatch(/3,000 × 2,250 px/);
      await fireEvent.click(screen.getByRole('button', { name: 'Open reduced copy' }));
      expect(onopen).toHaveBeenCalledExactlyOnceWith({ kind: 'reduced', width: 3_000, height: 2_250 });
    });
  });

  describe('keyboard', () => {
    it('puts focus on the recommended choice', () => {
      open(admissionFixture());
      expect(document.activeElement?.textContent).toMatch(/Choose Region/);
    });

    it('cancels on Escape from the first screen', async () => {
      const { oncancel } = open(admissionFixture());
      await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
      expect(oncancel).toHaveBeenCalledOnce();
    });

    it('goes back, rather than cancelling, on Escape from the region screen', async () => {
      const { oncancel, cancelPreview } = open(admissionFixture());
      await fireEvent.click(screen.getByRole('button', { name: /Choose Region/ }));
      await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
      expect(oncancel).not.toHaveBeenCalled();
      expect(cancelPreview).toHaveBeenCalled();
      expect(screen.getByRole('button', { name: /Choose Region/ })).toBeTruthy();
    });
  });
});
