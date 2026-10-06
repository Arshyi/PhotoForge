import type { AdmissionOption, AdmissionReport, SourceAdmission, Verdict } from './types';

/** A fixture shaped exactly like what `probe_image_source` returns. */
export function admissionFixture(
  overrides: {
    verdict?: Verdict;
    options?: AdmissionOption[];
    width?: number;
    height?: number;
    report?: Partial<AdmissionReport>;
  } = {}
): SourceAdmission {
  const width = overrides.width ?? 12_000;
  const height = overrides.height ?? 9_000;
  return {
    path: String.raw`C:\scans\wall.png`,
    filename: 'wall.png',
    kind: 'png',
    width,
    height,
    fileBytes: 90_000_000,
    hasIcc: false,
    bitDepth: 8,
    report: {
      verdict: overrides.verdict ?? { kind: 'regionRequired' },
      options: overrides.options ?? [
        { kind: 'openRegion', maxRegionPixels: 20_000_000, decode: { kind: 'rows' }, peakBytesAtMax: 900_000_000 },
        { kind: 'openReduced', maxScale: 0.4, decode: { kind: 'rows' } }
      ],
      fullPeakBytes: 5_000_000_000,
      budgetBytes: 2_600_000_000,
      availableBytes: 4_000_000_000,
      regionCost: { openingFixed: 1_000_000, openingPerPixel: 20, editingFixed: 40_960_000, editingPerPixel: 44 },
      outOfCore: 'Open Full Resolution (Out-of-Core) is not available: every stage holds complete buffers.',
      ...overrides.report
    }
  };
}

/** A synthetic pointer event: jsdom has no PointerEvent, and the components read `pointerId`. */
export function pointer(element: Element, type: string, x: number, y: number, button = 0) {
  const event = new MouseEvent(type, { bubbles: true, clientX: x, clientY: y, button });
  Object.defineProperty(event, 'pointerId', { value: 1 });
  element.dispatchEvent(event);
}

/** Gives an element a layout box, which jsdom does not compute. */
export function layout(element: Element, width: number, height: number) {
  (element as HTMLElement).getBoundingClientRect = () =>
    ({ left: 0, top: 0, width, height, right: width, bottom: height, x: 0, y: 0 }) as DOMRect;
  (element as HTMLElement).setPointerCapture = () => undefined;
  (element as HTMLElement).releasePointerCapture = () => undefined;
}
