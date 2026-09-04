export interface RefineSelectionParameters {
  smooth: number;
  feather: number;
  contrast: number;
  shiftEdge: number;
  decontaminate: boolean;
  decontaminateStrength: number;
  decontaminateRadius: number;
}

export const REFINE_SELECTION_DEFAULTS: Readonly<RefineSelectionParameters> = Object.freeze({
  smooth: 3,
  feather: 2,
  contrast: 0,
  shiftEdge: 0,
  decontaminate: false,
  decontaminateStrength: 0.5,
  decontaminateRadius: 4
});
