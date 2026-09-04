<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import SliderControl from './SliderControl.svelte';
  import type {
    BatchOptions,
    BatchPreview,
    BatchStatus,
    ComparisonMode,
    CurveSet,
    DevelopmentParameters,
    EditOperation,
    ExportProfile,
    HistogramChannels,
    HistogramResult,
    HslAdjustment,
    HslSettings,
    ImageMetadata,
    PerspectiveCorners,
    PixelInspection,
    RawInspectionResult,
    Workflow,
    WorkflowDocument
  } from '../types/editor';
  import { errorMessage, formatBytes } from '../utils/format';
  import { inspectRaw } from '../utils/raw';
  import { baseOperation, operationLabels, operationType, replaceOperation } from '../utils/operations';
  import { describeLayerSteps } from '../layers/workflow';
  import type { LayerDocument } from '../layers/types';
  import {
    createWorkflow,
    cloneWorkflow,
    duplicateOperationAt,
    duplicateWorkflow,
    loadWorkflows,
    MAX_WORKFLOW_JSON_CHARACTERS,
    MAX_WORKFLOW_OPERATIONS,
    moveOperation,
    parseWorkflowDocument,
    removeOperationAt,
    removeWorkflow,
    saveWorkflows,
    searchWorkflows,
    toggleFavorite,
    upsertWorkflow,
    validateEditOperation,
    validateLayerWorkflowSteps,
    validateWorkflow,
    workflowDocument
  } from '../utils/workflows';

  export let documentId = 0;
  export let metadata: ImageMetadata | null = null;
  export let operations: EditOperation[] = [];
  export let layerDocument: LayerDocument | null = null;
  export let layerRevision = 0;
  export let oncommit: (operations: EditOperation[], coalesceKey?: string) => void | boolean | Promise<void | boolean>;
  export let onworkflow: (workflow: Workflow) => boolean | Promise<boolean> = () => false;
  export let onmessage: (message: string, kind?: 'error' | 'success') => void;
  export let onviewchange: (view: { grid?: boolean; crosshair?: boolean; comparisonMode?: ComparisonMode; zoom?: number }) => void;

  type Tab = 'tools' | 'histogram' | 'workflows' | 'batch' | 'inspect';
  type HslChannel = keyof HslSettings;
  let tab: Tab = 'tools';
  let histogram: HistogramResult | null = null;
  let histogramMode: 'before' | 'after' = 'after';
  let histogramRequest = 0;
  let histogramTimer: ReturnType<typeof setTimeout> | undefined;
  let lastHistogramKey = '';
  let sampleRequest = 0;
  let workflowRequest = 0;
  let disposed = false;
  let curves: CurveSet = identityCurves();
  let inputBlack = 0;
  let inputWhite = 255;
  let levelGamma = 1;
  let outputBlack = 0;
  let outputWhite = 255;
  let cropX = 0;
  let cropY = 0;
  let cropWidth = 100;
  let cropHeight = 100;
  let cropRatio = 'free';
  let cropOverlay: 'none' | 'rule_of_thirds' | 'golden_ratio' = 'rule_of_thirds';
  let straighten = 0;
  let perspective: PerspectiveCorners = identityPerspective();
  let distortion = 0;
  let vignetting = 0;
  let chromaticAberration = 0;
  let lensDocumentId = -1;
  let lensOperationsKey = '';
  let lensDraftKey = '';
  let lensCommitRequest = 0;
  const locallyRequestedLensKeys = new Set<string>();
  let temperature = 0;
  let tint = 0;
  let developmentMode: 'asShot' | 'auto' | 'temperatureTint' = 'asShot';
  let developmentAsShotMultipliers: [number, number, number] = [1, 1, 1];
  let developmentExposure = 0;
  let developmentContrast = 0;
  let developmentHighlights = 0;
  let developmentShadows = 0;
  let developmentWhites = 0;
  let developmentBlacks = 0;
  let developmentOperationsKey = '';
  let developmentDraftKey = '';
  const locallyRequestedDevelopmentKeys = new Set<string>();
  let hslChannel: HslChannel = 'master';
  let hslSettings: HslSettings = emptyHsl();
  let selectiveHue = 0;
  let selectiveWidth = 45;
  let selectiveCyan = 0;
  let selectiveMagenta = 0;
  let selectiveYellow = 0;
  let selectiveBlack = 0;
  let pointX = 0;
  let pointY = 0;
  let pixel: PixelInspection | null = null;
  let rawInspection: RawInspectionResult | null = null;
  let rawInspecting = false;
  let measureX1 = 0;
  let measureY1 = 0;
  let measureX2 = 0;
  let measureY2 = 0;
  let workflows: Workflow[] = [];
  let workflowName = '';
  let workflowFolder = '';
  let workflowSearch = '';
  let selectedWorkflowId = '';
  let workflowJson = '';
  let layerWorkflowJson = '';
  let inputFolder = '';
  let outputFolder = '';
  let batchWorkflowId = '';
  let filenameTemplate = '{name}-{workflow}-{index}';
  let recursive = false;
  let overwrite = false;
  let workers = 2;
  let exportProfile: ExportProfile = 'lossless';
  let batchPreview: BatchPreview | null = null;
  let batchStatus: BatchStatus | null = null;
  let batchRunning = false;
  let statusTimer: ReturnType<typeof setInterval> | undefined;

  $: histogramKey = `${documentId}:${layerRevision}:${Boolean(layerDocument)}:${JSON.stringify(operations)}`;
  $: if (histogramKey !== lastHistogramKey) {
    lastHistogramKey = histogramKey;
    histogramRequest += 1;
    sampleRequest += 1;
    histogram = null;
    pixel = null;
    if (documentId) scheduleHistogram();
  }
  $: visibleWorkflows = searchWorkflows(workflows, workflowSearch);
  $: selectedWorkflow = workflows.find((workflow) => workflow.id === selectedWorkflowId) ?? null;
  $: activeHistogram = histogram?.[histogramMode] ?? null;
  $: measurement = Math.hypot(measureX2 - measureX1, measureY2 - measureY1);
  $: synchronizeLensControls(documentId, operations);
  $: synchronizeDevelopmentControls(documentId, operations);

  onMount(() => {
    workflows = loadWorkflows();
    if (workflows[0]) selectWorkflow(workflows[0]);
    batchWorkflowId = workflows[0]?.id ?? '';
  });

  onDestroy(() => {
    disposed = true;
    if (histogramTimer) clearTimeout(histogramTimer);
    if (statusTimer) clearInterval(statusTimer);
  });

  function identityCurves(): CurveSet {
    const identity = [{ input: 0, output: 0 }, { input: 1, output: 1 }];
    return { rgb: structuredClone(identity), red: structuredClone(identity), green: structuredClone(identity), blue: structuredClone(identity) };
  }

  function identityPerspective(): PerspectiveCorners {
    return { topLeft: [0, 0], topRight: [1, 0], bottomRight: [1, 1], bottomLeft: [0, 1] };
  }

  function emptyAdjustment(): HslAdjustment { return { hue: 0, saturation: 0, lightness: 0 }; }
  function emptyHsl(): HslSettings {
    return { master: emptyAdjustment(), red: emptyAdjustment(), yellow: emptyAdjustment(), green: emptyAdjustment(), cyan: emptyAdjustment(), blue: emptyAdjustment(), magenta: emptyAdjustment() };
  }

  function scheduleHistogram() {
    if (histogramTimer) clearTimeout(histogramTimer);
    histogramTimer = setTimeout(() => void refreshHistogram(), 180);
  }

  async function refreshHistogram() {
    if (!documentId) return;
    const requestId = ++histogramRequest;
    const source = sampleSource();
    try {
      const result = await invoke<HistogramResult>('generate_histogram', {
        operations, documentId, requestId, ...(layerDocument ? { layerDocument } : {})
      });
      if (result.isCurrent && result.documentId === source.documentId && result.requestId === requestId &&
        requestId === histogramRequest && sameSampleSource(source)) histogram = result;
    } catch (error) {
      if (requestId === histogramRequest && sameSampleSource(source)) onmessage(errorMessage(error), 'error');
    }
  }

  function histogramPoints(values: number[]): string {
    const maximum = Math.max(1, ...values);
    return values.map((value, index) => `${(index / 255) * 100},${40 - (value / maximum) * 38}`).join(' ');
  }

  function setOperation(operation: EditOperation, enabled = true, key = operation.type) {
    return oncommit(replaceOperation(operations, operation, enabled), key);
  }

  function applyCurvePreset(preset: 'linear' | 'contrast' | 'matte' | 'bright') {
    curves = identityCurves();
    if (preset === 'contrast') curves.rgb = [{ input: 0, output: 0 }, { input: 0.25, output: 0.18 }, { input: 0.75, output: 0.82 }, { input: 1, output: 1 }];
    if (preset === 'matte') curves.rgb = [{ input: 0, output: 0.08 }, { input: 0.5, output: 0.52 }, { input: 1, output: 0.96 }];
    if (preset === 'bright') curves.rgb = [{ input: 0, output: 0 }, { input: 0.4, output: 0.58 }, { input: 1, output: 1 }];
    setOperation({ type: 'curves', curves }, preset !== 'linear');
  }

  function addCurvePoint(channel: keyof CurveSet) {
    const points = [...curves[channel], { input: 0.5, output: 0.5 }].sort((left, right) => left.input - right.input);
    curves = { ...curves, [channel]: points };
    setOperation({ type: 'curves', curves });
  }

  function applyLevels() {
    setOperation({ type: 'levels', input_black: inputBlack, input_white: inputWhite, gamma: levelGamma, output_black: outputBlack, output_white: outputWhite });
  }

  function applyCropRatio() {
    if (!metadata || cropRatio === 'free') return;
    const ratios: Record<string, number> = { square: 1, '16:9': 16 / 9, '4:3': 4 / 3, a4: 210 / 297, original: metadata.width / metadata.height };
    const ratio = ratios[cropRatio];
    if (!ratio) return;
    const imageRatio = metadata.width / metadata.height;
    if (ratio > imageRatio) {
      cropWidth = 100;
      cropHeight = Math.min(100, (imageRatio / ratio) * 100);
    } else {
      cropHeight = 100;
      cropWidth = Math.min(100, (ratio / imageRatio) * 100);
    }
    cropX = (100 - cropWidth) / 2;
    cropY = (100 - cropHeight) / 2;
  }

  function applyCrop() {
    setOperation({ type: 'crop', x: cropX / 100, y: cropY / 100, width: cropWidth / 100, height: cropHeight / 100, aspect_ratio: cropRatio, overlay: cropOverlay });
  }

  function applyPerspective() { setOperation({ type: 'perspective', corners: perspective }); }
  function lensValues(source: EditOperation[]): { distortion: number; vignetting: number; chromaticAberration: number } | null {
    const candidate = source.find((operation) => operationType(operation) === 'lens_correction');
    if (!candidate) return null;
    const operation = baseOperation(candidate);
    if (operation.type !== 'lens_correction') return null;
    return {
      distortion: operation.distortion,
      vignetting: operation.vignetting,
      chromaticAberration: operation.chromatic_aberration
    };
  }

  function lensKey(values: { distortion: number; vignetting: number; chromaticAberration: number } | null): string {
    return values
      ? `${values.distortion}:${values.vignetting}:${values.chromaticAberration}`
      : 'none';
  }

  function synchronizeLensControls(nextDocumentId: number, source: EditOperation[]) {
    const values = lensValues(source);
    const nextKey = lensKey(values);
    const documentChanged = nextDocumentId !== lensDocumentId;
    if (!documentChanged && nextKey === lensOperationsKey) return;
    lensDocumentId = nextDocumentId;
    lensOperationsKey = nextKey;
    if (documentChanged) lensCommitRequest += 1;

    // Geometry commits are asynchronous. Ignore an older local echo while a newer
    // slider value is still queued, otherwise a continuing drag jumps backwards.
    if (!documentChanged && locallyRequestedLensKeys.has(nextKey) && nextKey !== lensDraftKey) {
      locallyRequestedLensKeys.delete(nextKey);
      return;
    }

    distortion = values?.distortion ?? 0;
    vignetting = values?.vignetting ?? 0;
    chromaticAberration = values?.chromaticAberration ?? 0;
    lensDraftKey = nextKey;
    locallyRequestedLensKeys.clear();
  }

  function restoreLensControlsFromOperations() {
    const values = lensValues(operations);
    const key = lensKey(values);
    distortion = values?.distortion ?? 0;
    vignetting = values?.vignetting ?? 0;
    chromaticAberration = values?.chromaticAberration ?? 0;
    lensOperationsKey = key;
    lensDraftKey = key;
    locallyRequestedLensKeys.clear();
  }

  async function applyLens() {
    const values = { distortion, vignetting, chromaticAberration };
    const enabled = Math.abs(distortion) + Math.abs(vignetting) + Math.abs(chromaticAberration) > 0.0001;
    const requestedKey = lensKey(enabled ? values : null);
    const ownRequest = ++lensCommitRequest;
    lensDraftKey = requestedKey;
    locallyRequestedLensKeys.add(requestedKey);
    if (locallyRequestedLensKeys.size > 32) {
      const oldest = locallyRequestedLensKeys.values().next().value;
      if (oldest !== undefined) locallyRequestedLensKeys.delete(oldest);
    }
    try {
      const accepted = await setOperation(
        { type: 'lens_correction', distortion, vignetting, chromatic_aberration: chromaticAberration },
        enabled
      );
      if (accepted === false && ownRequest === lensCommitRequest) {
        restoreLensControlsFromOperations();
      }
    } catch (error) {
      if (ownRequest !== lensCommitRequest) return;
      restoreLensControlsFromOperations();
      onmessage(errorMessage(error), 'error');
    }
  }
  function applyTemperatureTint() { setOperation({ type: 'temperature_tint', temperature, tint }, Math.abs(temperature) + Math.abs(tint) > 0.0001); }
  function defaultDevelopmentParameters(): DevelopmentParameters {
    return {
      whiteBalance: { mode: 'asShot', multipliers: [1, 1, 1] },
      exposureEv: 0,
      contrast: 0,
      highlights: 0,
      shadows: 0,
      whites: 0,
      blacks: 0
    };
  }

  function developmentValues(source: EditOperation[]): DevelopmentParameters | null {
    const candidate = source.find((operation) => operationType(operation) === 'raw_development');
    if (!candidate) return null;
    const operation = baseOperation(candidate);
    return operation.type === 'raw_development' ? operation.parameters : null;
  }

  function developmentParameters(): DevelopmentParameters {
    return {
      whiteBalance: developmentMode === 'auto'
        ? { mode: 'auto' }
        : developmentMode === 'temperatureTint'
          ? { mode: 'temperatureTint', temperature, tint }
          : { mode: 'asShot', multipliers: [...developmentAsShotMultipliers] as [number, number, number] },
      exposureEv: developmentExposure,
      contrast: developmentContrast,
      highlights: developmentHighlights,
      shadows: developmentShadows,
      whites: developmentWhites,
      blacks: developmentBlacks
    };
  }

  function developmentKey(parameters: DevelopmentParameters): string { return JSON.stringify(parameters); }

  function synchronizeDevelopmentControls(nextDocumentId: number, source: EditOperation[]) {
    const next = developmentValues(source) ?? defaultDevelopmentParameters();
    const nextKey = developmentKey(next);
    if (nextKey === developmentOperationsKey) return;
    developmentOperationsKey = nextKey;
    if (nextKey !== developmentDraftKey && locallyRequestedDevelopmentKeys.has(nextKey)) {
      locallyRequestedDevelopmentKeys.delete(nextKey);
      return;
    }
    developmentMode = next.whiteBalance.mode === 'auto' || next.whiteBalance.mode === 'temperatureTint'
      ? next.whiteBalance.mode : 'asShot';
    if (next.whiteBalance.mode === 'asShot' || next.whiteBalance.mode === 'custom') {
      developmentAsShotMultipliers = [...next.whiteBalance.multipliers] as [number, number, number];
    }
    if (next.whiteBalance.mode === 'temperatureTint') {
      temperature = next.whiteBalance.temperature;
      tint = next.whiteBalance.tint;
    }
    developmentExposure = next.exposureEv;
    developmentContrast = next.contrast;
    developmentHighlights = next.highlights;
    developmentShadows = next.shadows;
    developmentWhites = next.whites;
    developmentBlacks = next.blacks;
    developmentDraftKey = nextKey;
    if (nextDocumentId !== documentId) locallyRequestedDevelopmentKeys.clear();
  }

  function restoreDevelopmentControlsFromOperations() {
    developmentOperationsKey = '';
    locallyRequestedDevelopmentKeys.clear();
    synchronizeDevelopmentControls(documentId, operations);
  }

  async function applyDevelopment() {
    const parameters = developmentParameters();
    const neutral = defaultDevelopmentParameters();
    const enabled = developmentKey(parameters) !== developmentKey(neutral);
    const requestedKey = enabled ? developmentKey(parameters) : developmentKey(neutral);
    developmentDraftKey = requestedKey;
    locallyRequestedDevelopmentKeys.add(requestedKey);
    if (locallyRequestedDevelopmentKeys.size > 32) {
      const oldest = locallyRequestedDevelopmentKeys.values().next().value;
      if (oldest !== undefined) locallyRequestedDevelopmentKeys.delete(oldest);
    }
    try {
      const accepted = await setOperation({ type: 'raw_development', parameters }, enabled, 'raw_development');
      if (accepted === false) restoreDevelopmentControlsFromOperations();
    } catch (error) {
      restoreDevelopmentControlsFromOperations();
      onmessage(errorMessage(error), 'error');
    }
  }

  async function exportDevelopedPng16() {
    const path = await save({
      defaultPath: `${(metadata?.filename ?? 'developed').replace(/\.[^.]+$/, '')}-16bit.png`,
      filters: [{ name: '16-bit PNG', extensions: ['png'] }]
    });
    if (!path) return;
    try {
      const result = await invoke<{ outputPath: string; width: number; height: number }>('export_developed_png16', {
        outputPath: path,
        operations
      });
      onmessage(`Exported ${result.width} × ${result.height} true 16-bit PNG`);
    } catch (error) {
      onmessage(errorMessage(error), 'error');
    }
  }
  function applyHsl() { setOperation({ type: 'hsl', settings: hslSettings }, JSON.stringify(hslSettings) !== JSON.stringify(emptyHsl())); }
  function setHslValue(field: keyof HslAdjustment, value: number) {
    hslSettings = { ...hslSettings, [hslChannel]: { ...hslSettings[hslChannel], [field]: value } };
    applyHsl();
  }
  function applySelective() { setOperation({ type: 'selective_color', target_hue: selectiveHue, width: selectiveWidth, adjustment: { cyan: selectiveCyan, magenta: selectiveMagenta, yellow: selectiveYellow, black: selectiveBlack } }, Math.abs(selectiveCyan) + Math.abs(selectiveMagenta) + Math.abs(selectiveYellow) + Math.abs(selectiveBlack) > 0.0001); }

  async function inspect() {
    const source = sampleSource();
    const requestId = ++sampleRequest;
    try {
      const result = await invoke<PixelInspection>('inspect_image_pixel', {
        x: pointX, y: pointY, operations, documentId, ...(layerDocument ? { layerDocument } : {})
      });
      if (requestId === sampleRequest && sameSampleSource(source)) pixel = result;
    } catch (error) {
      if (requestId === sampleRequest && sameSampleSource(source)) onmessage(errorMessage(error), 'error');
    }
  }

  async function inspectRawFile() {
    const chosen = await open({
      multiple: false,
      directory: false,
      filters: [{ name: 'Camera RAW', extensions: ['dng', 'cr2', 'cr3', 'nef', 'arw', 'raf', 'orf', 'rw2', 'pef', 'srw'] }]
    });
    if (typeof chosen !== 'string' || rawInspecting) return;
    rawInspecting = true;
    try {
      rawInspection = await inspectRaw(chosen);
      onmessage(
        rawInspection.inspection.support === 'recognizedDecoderUnavailable'
          ? 'RAW format recognised; decoder support is not bundled in this build.'
          : 'The selected file is not a recognised RAW source.',
        rawInspection.inspection.support === 'recognizedDecoderUnavailable' ? 'success' : 'error'
      );
    } catch (error) {
      onmessage(errorMessage(error), 'error');
    } finally {
      rawInspecting = false;
    }
  }

  async function pickPoint(white: boolean) {
    const source = sampleSource();
    const requestId = ++sampleRequest;
    const x = pointX; const y = pointY;
    try {
      const operation = await invoke<EditOperation>('create_point_operation', {
        x, y, white, operations, documentId, ...(layerDocument ? { layerDocument } : {})
      });
      if (requestId !== sampleRequest || !sameSampleSource(source)) return;
      const accepted = await setOperation(operation);
      // The accepted edit itself changes the source/revision; only suppress
      // completion after a document switch, cancellation, or rejected commit.
      if (!disposed && source.documentId === documentId && accepted !== false) {
        onmessage(`${white ? 'White' : 'Black'} point sampled at ${x}, ${y}`);
      }
    } catch (error) {
      if (requestId === sampleRequest && sameSampleSource(source)) onmessage(errorMessage(error), 'error');
    }
  }

  function sampleSource() {
    return { documentId, layerRevision, layerDocument, operationsKey: JSON.stringify(operations) };
  }

  function sameSampleSource(source: ReturnType<typeof sampleSource>) {
    return !disposed && source.documentId === documentId && source.layerRevision === layerRevision &&
      source.layerDocument === layerDocument && source.operationsKey === JSON.stringify(operations);
  }

  function persistWorkflows(next: Workflow[]) {
    saveWorkflows(next);
    workflows = next;
  }

  function recordWorkflow() {
    if (!workflowName.trim() || operations.length === 0) {
      onmessage('Enter a workflow name and add at least one edit.', 'error'); return;
    }
    const value = createWorkflow(workflowName, operations, workflowFolder);
    const errors = validateWorkflow(value);
    if (errors.length) {
      onmessage(errors.join(' '), 'error'); return;
    }
    persistWorkflows(upsertWorkflow(workflows, value));
    selectWorkflow(value); batchWorkflowId = value.id; workflowName = '';
    onmessage('Workflow saved locally');
  }

  function updateSelected(update: (workflow: Workflow) => Workflow) {
    if (!selectedWorkflow) return false;
    const next = update(structuredClone(selectedWorkflow));
    next.updatedAt = new Date().toISOString();
    const errors = validateWorkflow(next);
    if (errors.length) { onmessage(errors.join(' '), 'error'); return false; }
    persistWorkflows(upsertWorkflow(workflows, next));
    return true;
  }

  function selectWorkflow(workflow: Workflow) {
    selectedWorkflowId = workflow.id;
    workflowJson = JSON.stringify(workflow.operations, null, 2);
    layerWorkflowJson = JSON.stringify(workflow.layerSteps, null, 2);
  }

  async function applyWorkflow(workflow: Workflow) {
    const source = sampleSource();
    const requestId = ++workflowRequest;
    try {
      const errors = validateWorkflow(workflow);
      if (errors.length) throw new Error(errors.join(' '));
      const accepted = await onworkflow(cloneWorkflow(workflow));
      if (accepted === false && requestId === workflowRequest && sameSampleSource(source)) {
        onmessage(`${workflow.name} was not applied.`, 'error');
      }
    } catch (error) {
      if (requestId === workflowRequest && sameSampleSource(source)) onmessage(errorMessage(error), 'error');
    }
  }

  function applyWorkflowJson() {
    try {
      if (workflowJson.length > MAX_WORKFLOW_JSON_CHARACTERS) throw new Error('Operation JSON exceeds the bounded limit.');
      const parsed: unknown = JSON.parse(workflowJson);
      if (!Array.isArray(parsed)) throw new Error('Operation JSON must be an array.');
      if (parsed.length > MAX_WORKFLOW_OPERATIONS) throw new Error('Operation JSON has too many operations.');
      parsed.forEach((operation, index) => {
        const error = validateEditOperation(operation);
        if (error) throw new Error(`Operation ${index + 1}: ${error}`);
      });
      if (updateSelected((workflow) => ({ ...workflow, operations: structuredClone(parsed) as EditOperation[] }))) {
        onmessage('Workflow definition updated locally');
      }
    } catch (error) { onmessage(errorMessage(error), 'error'); }
  }

  function applyLayerWorkflowJson() {
    try {
      if (layerWorkflowJson.length > MAX_WORKFLOW_JSON_CHARACTERS) throw new Error('Layer step JSON exceeds the bounded limit.');
      const parsed: unknown = JSON.parse(layerWorkflowJson);
      const errors = validateLayerWorkflowSteps(parsed);
      if (errors.length) throw new Error(errors.join(' '));
      if (updateSelected((workflow) => ({ ...workflow, layerSteps: structuredClone(parsed) as Workflow['layerSteps'] }))) {
        onmessage('Workflow layer steps updated locally');
      }
    } catch (error) { onmessage(errorMessage(error), 'error'); }
  }

  async function importWorkflowFile() {
    const path = await open({ multiple: false, directory: false, filters: [{ name: 'PhotoForge workflow', extensions: ['json'] }] });
    if (typeof path !== 'string') return;
    try {
      const imported = await invoke<WorkflowDocument>('import_workflow', { path });
      const document = parseWorkflowDocument(JSON.stringify(imported));
      persistWorkflows(upsertWorkflow(workflows, document.workflow));
      selectWorkflow(document.workflow);
      onmessage('Workflow imported and validated');
    } catch (error) { onmessage(errorMessage(error), 'error'); }
  }

  async function exportSelectedWorkflow() {
    if (!selectedWorkflow) return;
    const path = await save({ defaultPath: `${selectedWorkflow.name.replace(/[^a-z0-9]+/gi, '-')}.json`, filters: [{ name: 'PhotoForge workflow', extensions: ['json'] }] });
    if (!path) return;
    try {
      await invoke('export_workflow', { path, document: workflowDocument(selectedWorkflow) });
      onmessage('Versioned workflow JSON exported');
    } catch (error) { onmessage(errorMessage(error), 'error'); }
  }

  async function chooseBatchFolder(kind: 'input' | 'output') {
    const path = await open({ directory: true, multiple: false, title: `Choose batch ${kind} folder` });
    if (typeof path === 'string') { if (kind === 'input') inputFolder = path; else outputFolder = path; }
  }

  function batchOptions(dryRun: boolean): BatchOptions {
    return { inputFolder, outputFolder, filenameTemplate, recursive, overwrite, workers, exportProfile, dryRun };
  }

  async function previewBatch() {
    const workflow = workflows.find((candidate) => candidate.id === batchWorkflowId);
    if (!workflow) { onmessage('Choose a saved workflow first.', 'error'); return; }
    try { batchPreview = await invoke<BatchPreview>('preview_batch_workflow', { options: batchOptions(true), workflow }); }
    catch (error) { onmessage(errorMessage(error), 'error'); }
  }

  async function startBatch() {
    const workflow = workflows.find((candidate) => candidate.id === batchWorkflowId);
    if (!workflow) { onmessage('Choose a saved workflow first.', 'error'); return; }
    batchRunning = true;
    const batchId = Date.now();
    statusTimer = setInterval(() => void pollBatch(), 250);
    try {
      batchStatus = await invoke<BatchStatus>('start_batch_workflow', { batchId, options: batchOptions(false), workflow });
      onmessage(`Batch ${batchStatus.state}: ${batchStatus.completed} completed, ${batchStatus.failed} failed`);
    } catch (error) { onmessage(errorMessage(error), 'error'); }
    finally { batchRunning = false; if (statusTimer) clearInterval(statusTimer); statusTimer = undefined; }
  }

  async function pollBatch() {
    try { batchStatus = await invoke<BatchStatus>('get_batch_status'); } catch { /* final request reports errors */ }
  }

  async function cancelBatch() {
    try { batchStatus = await invoke<BatchStatus>('cancel_batch'); } catch (error) { onmessage(errorMessage(error), 'error'); }
  }
</script>

<section class="professional-workspace" aria-labelledby="professional-title">
  <header>
    <div><span>PRO</span><h2 id="professional-title">Professional workspace</h2></div>
    <small>0.6 · deterministic</small>
  </header>
  <div class="professional-tabs" aria-label="Professional editing panels" role="tablist">
    {#each [['tools', 'Tools'], ['histogram', 'Scopes'], ['workflows', 'Flows'], ['batch', 'Batch'], ['inspect', 'Inspect']] as item}
      <button type="button" role="tab" aria-selected={tab === item[0]} class:active={tab === item[0]} on:click={() => (tab = item[0] as Tab)}>{item[1]}</button>
    {/each}
  </div>

  {#if tab === 'tools'}
    <div class="professional-panel" aria-label="Professional editing tools">
      <details open>
        <summary>Curves <small>RGB · R · G · B</small></summary>
        <div class="preset-row">
          {#each ['linear', 'contrast', 'matte', 'bright'] as preset}
            <button type="button" on:click={() => applyCurvePreset(preset as 'linear' | 'contrast' | 'matte' | 'bright')}>{preset}</button>
          {/each}
        </div>
        <div class="curve-grid">
          {#each Object.keys(curves) as channel}
            <button type="button" on:click={() => addCurvePoint(channel as keyof CurveSet)}>{channel.toUpperCase()} · {curves[channel as keyof CurveSet].length} points ＋</button>
          {/each}
        </div>
      </details>

      <details open>
        <summary>Levels <small>Input · gamma · output</small></summary>
        <div class="five-fields">
          <label>Black<input aria-label="Input black" type="number" min="0" max="254" bind:value={inputBlack} on:change={applyLevels} /></label>
          <label>White<input aria-label="Input white" type="number" min="1" max="255" bind:value={inputWhite} on:change={applyLevels} /></label>
          <label>Gamma<input aria-label="Levels gamma" type="number" min="0.1" max="10" step="0.05" bind:value={levelGamma} on:change={applyLevels} /></label>
          <label>Out B<input aria-label="Output black" type="number" min="0" max="255" bind:value={outputBlack} on:change={applyLevels} /></label>
          <label>Out W<input aria-label="Output white" type="number" min="0" max="255" bind:value={outputWhite} on:change={applyLevels} /></label>
        </div>
      </details>

      <details>
        <summary>White & black point <small>Pixel sampler</small></summary>
        <div class="coordinate-fields"><label>X<input type="number" min="0" max={metadata?.width ?? 0} bind:value={pointX} /></label><label>Y<input type="number" min="0" max={metadata?.height ?? 0} bind:value={pointY} /></label></div>
        <div class="button-row"><button type="button" on:click={() => pickPoint(true)}>Pick white</button><button type="button" on:click={() => pickPoint(false)}>Pick black</button></div>
      </details>

      <details>
        <summary>Crop <small>Free · presets · overlays</small></summary>
        <label class="field">Aspect ratio<select bind:value={cropRatio} on:change={applyCropRatio}><option value="free">Free</option><option value="square">Square</option><option value="16:9">16:9</option><option value="4:3">4:3</option><option value="a4">A4</option><option value="original">Original ratio</option></select></label>
        <label class="field">Overlay<select bind:value={cropOverlay} on:change={() => onviewchange({ grid: cropOverlay !== 'none' })}><option value="none">None</option><option value="rule_of_thirds">Rule of thirds</option><option value="golden_ratio">Golden ratio</option></select></label>
        <div class="four-fields"><label>X %<input type="number" min="0" max="99" bind:value={cropX} /></label><label>Y %<input type="number" min="0" max="99" bind:value={cropY} /></label><label>W %<input type="number" min="1" max="100" bind:value={cropWidth} /></label><label>H %<input type="number" min="1" max="100" bind:value={cropHeight} /></label></div>
        <button class="primary" type="button" on:click={applyCrop}>Apply crop</button>
      </details>

      <details>
        <summary>Straighten <small>±45° · 90° snap</small></summary>
        <SliderControl label="Rotation" value={straighten} min={-45} max={45} step={0.1} defaultValue={0} format={(value) => `${value.toFixed(1)}°`} onchange={(value) => { straighten = value; setOperation({ type: 'straighten', degrees: value }, Math.abs(value) > 0.001); }} />
        <div class="button-row"><button type="button" on:click={() => onviewchange({ grid: true })}>Grid overlay</button><button type="button" on:click={() => setOperation({ type: 'rotate', degrees: 90 })}>Snap 90°</button></div>
      </details>

      <details>
        <summary>Perspective <small>Four-corner correction</small></summary>
        <div class="perspective-fields">
          {#each [['TL', 'topLeft'], ['TR', 'topRight'], ['BR', 'bottomRight'], ['BL', 'bottomLeft']] as corner}
            <label>{corner[0]} X<input type="number" min="0" max="1" step="0.01" bind:value={perspective[corner[1] as keyof PerspectiveCorners][0]} /></label>
            <label>{corner[0]} Y<input type="number" min="0" max="1" step="0.01" bind:value={perspective[corner[1] as keyof PerspectiveCorners][1]} /></label>
          {/each}
        </div>
        <div class="button-row"><button type="button" on:click={() => (perspective = identityPerspective())}>Reset</button><button class="primary" type="button" on:click={applyPerspective}>Apply perspective</button></div>
      </details>

      <details>
        <summary>Lens correction <small>Distortion · vignette · CA</small></summary>
        <SliderControl label="Barrel / pincushion" value={distortion} min={-0.16} max={1} step={0.01} defaultValue={0} format={(value) => value.toFixed(2)} onchange={(value) => { distortion = value; applyLens(); }} />
        <SliderControl label="Vignetting" value={vignetting} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => value.toFixed(2)} onchange={(value) => { vignetting = value; applyLens(); }} />
        <SliderControl label="Chromatic aberration" value={chromaticAberration} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => value.toFixed(2)} onchange={(value) => { chromaticAberration = value; applyLens(); }} />
      </details>

      <details>
        <summary>HSL <small>Master + six colors</small></summary>
        <label class="field">Channel<select bind:value={hslChannel}>{#each Object.keys(hslSettings) as channel}<option value={channel}>{channel}</option>{/each}</select></label>
        <SliderControl label="Hue" value={hslSettings[hslChannel].hue} min={-180} max={180} step={1} defaultValue={0} format={(value) => `${value}°`} onchange={(value) => setHslValue('hue', value)} />
        <SliderControl label="Saturation" value={hslSettings[hslChannel].saturation} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => setHslValue('saturation', value)} />
        <SliderControl label="Lightness" value={hslSettings[hslChannel].lightness} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => setHslValue('lightness', value)} />
      </details>

      <details>
        <summary>Temperature & tint <small>Manual color balance</small></summary>
        <SliderControl label="Temperature" value={temperature} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => value.toFixed(2)} onchange={(value) => { temperature = value; applyTemperatureTint(); }} />
        <SliderControl label="Tint" value={tint} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => value.toFixed(2)} onchange={(value) => { tint = value; applyTemperatureTint(); }} />
      </details>

      <details open>
        <summary>RAW development <small>Linear-light · non-destructive</small></summary>
        <p class="privacy-note">Works on the high-precision boundary for raster previews today; camera RAW import remains decoder-gated.</p>
        <label class="field">White balance mode<select bind:value={developmentMode} on:change={applyDevelopment}><option value="asShot">As Shot</option><option value="auto">Auto (gray world)</option><option value="temperatureTint">Temperature / tint</option></select></label>
        {#if developmentMode === 'temperatureTint'}
          <SliderControl label="Temperature" value={temperature} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => value.toFixed(2)} onchange={(value) => { temperature = value; applyDevelopment(); }} />
          <SliderControl label="Tint" value={tint} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => value.toFixed(2)} onchange={(value) => { tint = value; applyDevelopment(); }} />
        {/if}
        <SliderControl label="Exposure" value={developmentExposure} min={-8} max={8} step={0.01} defaultValue={0} format={(value) => `${value.toFixed(2)} EV`} onchange={(value) => { developmentExposure = value; applyDevelopment(); }} />
        <SliderControl label="Contrast" value={developmentContrast} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => { developmentContrast = value; applyDevelopment(); }} />
        <SliderControl label="Highlights" value={developmentHighlights} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => { developmentHighlights = value; applyDevelopment(); }} />
        <SliderControl label="Shadows" value={developmentShadows} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => { developmentShadows = value; applyDevelopment(); }} />
        <SliderControl label="Whites" value={developmentWhites} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => { developmentWhites = value; applyDevelopment(); }} />
        <SliderControl label="Blacks" value={developmentBlacks} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => { developmentBlacks = value; applyDevelopment(); }} />
        <button type="button" on:click={exportDevelopedPng16}>Export true 16-bit PNG</button>
      </details>

      <details>
        <summary>Selective color <small>Deterministic hue range</small></summary>
        <SliderControl label="Target hue" value={selectiveHue} min={0} max={360} step={1} defaultValue={0} format={(value) => `${value}°`} onchange={(value) => { selectiveHue = value; applySelective(); }} />
        <SliderControl label="Range" value={selectiveWidth} min={1} max={180} step={1} defaultValue={45} format={(value) => `${value}°`} onchange={(value) => { selectiveWidth = value; applySelective(); }} />
        {#each [['Cyan', 'cyan'], ['Magenta', 'magenta'], ['Yellow', 'yellow'], ['Black', 'black']] as control}
          <SliderControl label={control[0]} value={control[1] === 'cyan' ? selectiveCyan : control[1] === 'magenta' ? selectiveMagenta : control[1] === 'yellow' ? selectiveYellow : selectiveBlack} min={-1} max={1} step={0.01} defaultValue={0} format={(value) => `${Math.round(value * 100)}%`} onchange={(value) => { if (control[1] === 'cyan') selectiveCyan = value; else if (control[1] === 'magenta') selectiveMagenta = value; else if (control[1] === 'yellow') selectiveYellow = value; else selectiveBlack = value; applySelective(); }} />
        {/each}
      </details>
    </div>

  {:else if tab === 'histogram'}
    <div class="professional-panel scopes-panel">
      <div class="segmented"><button class:active={histogramMode === 'before'} on:click={() => (histogramMode = 'before')}>Before</button><button class:active={histogramMode === 'after'} on:click={() => (histogramMode = 'after')}>After</button><button on:click={refreshHistogram}>Refresh</button></div>
      {#if activeHistogram}
        <svg class="histogram" viewBox="0 0 100 40" role="img" aria-label={`${histogramMode} RGB and luminance histogram`}>
          <polyline class="red" points={histogramPoints(activeHistogram.red)} /><polyline class="green" points={histogramPoints(activeHistogram.green)} /><polyline class="blue" points={histogramPoints(activeHistogram.blue)} /><polyline class="luma" points={histogramPoints(activeHistogram.luminance)} />
        </svg>
        <div class="scope-stats"><span>Pixels <b>{activeHistogram.pixelCount}</b></span><span>Shadow clipping <b>{activeHistogram.shadowClipping}</b></span><span>Highlight clipping <b>{activeHistogram.highlightClipping}</b></span><span>Updated <b>{histogram?.processingTimeMs.toFixed(1)} ms</b></span></div>
      {:else}<p class="empty-copy">Open an image to generate live before/after histograms.</p>{/if}
      <h3>Comparison modes</h3>
      <div class="comparison-grid">{#each ['swipe', 'split', 'blink', 'difference'] as mode}<button type="button" on:click={() => onviewchange({ comparisonMode: mode as ComparisonMode })}>{mode}</button>{/each}</div>
    </div>

  {:else if tab === 'workflows'}
    <div class="professional-panel workflow-panel">
      <div class="record-card"><strong>Record current pipeline</strong><input aria-label="Workflow name" placeholder="Workflow name" bind:value={workflowName} /><input aria-label="Workflow folder" placeholder="Folder (optional)" bind:value={workflowFolder} /><button class="primary" type="button" disabled={!operations.length} on:click={recordWorkflow}>Save workflow · {operations.length} edits</button></div>
      <p class="empty-copy">Recording captures document operations only. Add layer steps through imported version 2 JSON or the layer-step editor below.</p>
      <div class="workflow-toolbar"><input aria-label="Search workflows" placeholder="Search workflows" bind:value={workflowSearch} /><button on:click={importWorkflowFile}>Import JSON</button><button disabled={!selectedWorkflow} on:click={exportSelectedWorkflow}>Export JSON</button></div>
      <div class="workflow-list">
        {#each visibleWorkflows as workflow}
          <article class:selected={workflow.id === selectedWorkflowId}>
            <button class="workflow-main" type="button" on:click={() => selectWorkflow(workflow)}><span><strong>{workflow.name}</strong><small>{workflow.folder || 'Unfiled'} · {workflow.operations.length} operations · {workflow.layerSteps.length} layer steps</small></span></button>
            <div><button aria-label={`Favorite ${workflow.name}`} on:click={() => persistWorkflows(toggleFavorite(workflows, workflow.id))}>{workflow.favorite ? '★' : '☆'}</button><button on:click={() => applyWorkflow(workflow)}>Replay</button><button on:click={() => persistWorkflows(duplicateWorkflow(workflows, workflow.id))}>Duplicate</button><button on:click={() => persistWorkflows(removeWorkflow(workflows, workflow.id))}>Delete</button></div>
          </article>
        {:else}<p class="empty-copy">No saved workflows match this search.</p>{/each}
      </div>
      {#if selectedWorkflow}
        <div class="workflow-editor"><h3>Workflow editor</h3><label>Name<input value={selectedWorkflow.name} on:change={(event) => updateSelected((workflow) => ({ ...workflow, name: event.currentTarget.value }))} /></label><label>Folder<input value={selectedWorkflow.folder} on:change={(event) => updateSelected((workflow) => ({ ...workflow, folder: event.currentTarget.value }))} /></label>
          <ol>{#each selectedWorkflow.operations as operation, index}<li><span>{index + 1}. {operationLabels[operationType(operation)]}{operation.type === 'masked' ? ' · Masked' : ''}</span><div><button aria-label="Move up" on:click={() => updateSelected((workflow) => ({ ...workflow, operations: moveOperation(workflow.operations, index, -1) }))}>↑</button><button aria-label="Move down" on:click={() => updateSelected((workflow) => ({ ...workflow, operations: moveOperation(workflow.operations, index, 1) }))}>↓</button><button aria-label="Duplicate operation" on:click={() => updateSelected((workflow) => ({ ...workflow, operations: duplicateOperationAt(workflow.operations, index) }))}>⧉</button><button aria-label="Delete operation" on:click={() => updateSelected((workflow) => ({ ...workflow, operations: removeOperationAt(workflow.operations, index) }))}>×</button></div></li>{/each}</ol>
          <label>Typed operation JSON<textarea rows="8" bind:value={workflowJson} placeholder="Select workflow to edit operation parameters"></textarea></label><div class="button-row"><button on:click={applyWorkflowJson}>Validate & update</button><button class="primary" on:click={() => applyWorkflow(selectedWorkflow)}>Preview workflow</button></div>
          <h3>Layer steps</h3>
          <ol aria-label="Workflow layer steps">{#each describeLayerSteps(selectedWorkflow.layerSteps) as label}<li>{label}</li>{/each}</ol>
          <label>Typed layer-step JSON<textarea rows="8" bind:value={layerWorkflowJson} placeholder="Select workflow to edit layer steps"></textarea></label>
          <button type="button" on:click={applyLayerWorkflowJson}>Validate & update layer steps</button>
        </div>
      {/if}
    </div>

  {:else if tab === 'batch'}
    <div class="professional-panel batch-panel">
      <p class="privacy-note">Batch processing stays offline and uses bounded workers. Originals are never overwritten.</p>
      <label class="path-field">Input folder<span><input readonly value={inputFolder} placeholder="Choose input folder" /><button on:click={() => chooseBatchFolder('input')}>Browse</button></span></label>
      <label class="path-field">Output folder<span><input readonly value={outputFolder} placeholder="Choose output folder" /><button on:click={() => chooseBatchFolder('output')}>Browse</button></span></label>
      <label class="field">Workflow<select bind:value={batchWorkflowId}><option value="">Choose workflow</option>{#each workflows as workflow}<option value={workflow.id}>{workflow.name}</option>{/each}</select></label>
      <label class="field">Filename template<input bind:value={filenameTemplate} /></label>
      <label class="field">Export profile<select bind:value={exportProfile}>{#each ['web', 'print', 'archive', 'lossless', 'high_jpeg', 'maximum_compression'] as profile}<option value={profile}>{profile.replaceAll('_', ' ')}</option>{/each}</select></label>
      <SliderControl label="Bounded workers" value={workers} min={1} max={8} step={1} defaultValue={2} format={(value) => `${value}`} onchange={(value) => (workers = value)} />
      <div class="check-grid"><label><input type="checkbox" bind:checked={recursive} /> Recursive</label><label><input type="checkbox" bind:checked={overwrite} /> Allow overwrite</label></div>
      <div class="button-row"><button disabled={batchRunning} on:click={previewBatch}>Batch preview</button><button class="primary" disabled={batchRunning} on:click={startBatch}>Start batch</button><button disabled={!batchRunning} on:click={cancelBatch}>Cancel</button></div>
      {#if batchPreview}<div class="batch-summary"><strong>{batchPreview.discovered} images</strong><span>~{(batchPreview.estimatedTimeMs / 1000).toFixed(1)} sec · {batchPreview.skippedExisting} protected</span><ol>{#each batchPreview.sampleOutputs as output}<li>{output}</li>{/each}</ol></div>{/if}
      {#if batchStatus}<div class="batch-progress" aria-live="polite"><progress max={Math.max(1, batchStatus.discovered)} value={batchStatus.completed + batchStatus.failed + batchStatus.skipped}></progress><strong>{batchStatus.state}</strong><span>{batchStatus.completed} complete · {batchStatus.skipped} skipped · {batchStatus.failed} failed</span>{#if batchStatus.currentFile}<small>{batchStatus.currentFile}</small>{/if}{#if batchStatus.logPath}<small>Log: {batchStatus.logPath}</small>{/if}{#each batchStatus.failures as failure}<p>{failure.inputPath}: {failure.error}</p>{/each}</div>{/if}
    </div>

  {:else}
    <div class="professional-panel inspect-panel">
      {#if metadata}<div class="metadata-grid"><span>Dimensions<b>{metadata.width} × {metadata.height}</b></span><span>Color space<b>{metadata.colorSpace}</b></span><span>Bit depth<b>{metadata.bitDepth}-bit</b></span><span>Alpha<b>{metadata.hasAlpha ? 'Yes' : 'No'}</b></span><span>Format<b>{metadata.format}</b></span><span>Size<b>{formatBytes(metadata.fileSize)}</b></span><span>Camera<b>{metadata.cameraModel ?? 'Not present'}</b></span><span>EXIF<b>{metadata.exifAvailable ? 'Available' : 'Not present'}</b></span><span>Created<b>{metadata.createdAt ?? 'Unavailable'}</b></span><span>Modified<b>{metadata.modifiedAt ?? 'Unavailable'}</b></span></div>{/if}
      <div class="raw-card"><div><strong>RAW source inspection</strong><small>Metadata-only; the decoder is not bundled yet.</small></div><button type="button" disabled={rawInspecting} on:click={inspectRawFile}>{rawInspecting ? 'Inspecting…' : 'Choose RAW file'}</button></div>
      {#if rawInspection}<div class="raw-result"><span>Format <b>{rawInspection.inspection.format ?? 'Unknown'}</b></span><span>Status <b>{rawInspection.inspection.support === 'recognizedDecoderUnavailable' ? 'Decoder unavailable' : 'Unsupported'}</b></span><span>SHA-256 <b>{rawInspection.inspection.sha256.slice(0, 16)}…</b></span><span>Size <b>{formatBytes(rawInspection.inspection.fileSize)}</b></span></div>{/if}
      <h3>Pixel inspector</h3><div class="coordinate-fields"><label>X<input type="number" min="0" max={metadata?.width ?? 0} bind:value={pointX} /></label><label>Y<input type="number" min="0" max={metadata?.height ?? 0} bind:value={pointY} /></label></div><div class="button-row"><button on:click={inspect}>Inspect pixel</button><button on:click={() => onviewchange({ crosshair: true })}>Crosshair</button><button on:click={() => onviewchange({ grid: true })}>Pixel grid</button><button on:click={() => onviewchange({ zoom: 1600 })}>Zoom 1600%</button></div>
      {#if pixel}<div class="pixel-result"><span style={`background:rgba(${pixel.red},${pixel.green},${pixel.blue},${pixel.alpha / 255})`}></span><div><b>RGB {pixel.red}, {pixel.green}, {pixel.blue} · A {pixel.alpha}</b><small>HSV {pixel.hue.toFixed(1)}°, {(pixel.saturation * 100).toFixed(1)}%, {(pixel.value * 100).toFixed(1)}% · ({pixel.x}, {pixel.y})</small></div></div>{/if}
      <h3>Measurement tool</h3><div class="four-fields"><label>X₁<input type="number" bind:value={measureX1} /></label><label>Y₁<input type="number" bind:value={measureY1} /></label><label>X₂<input type="number" bind:value={measureX2} /></label><label>Y₂<input type="number" bind:value={measureY2} /></label></div><output>{measurement.toFixed(2)} pixels</output>
    </div>
  {/if}
</section>

<style>
  .professional-workspace { display: grid; border-bottom: 1px solid var(--line); background: rgba(19, 21, 18, 0.55); }
  header { display: flex; align-items: center; justify-content: space-between; padding: 12px; }
  header div { display: flex; align-items: center; gap: 7px; }
  header span { padding: 4px 5px; border-radius: 4px; color: #162014; background: var(--accent); font: 800 0.48rem/1 var(--font-mono); }
  header h2 { margin: 0; color: var(--ink); font-size: 0.68rem; }
  header small { color: var(--ink-faint); font: 0.49rem/1 var(--font-mono); }
  .professional-tabs { display: grid; grid-template-columns: repeat(5, 1fr); padding: 0 8px 8px; gap: 3px; }
  button, input, select, textarea { font: inherit; }
  button { border: 1px solid var(--line); border-radius: 6px; color: var(--ink-soft); background: var(--surface-raised); cursor: pointer; }
  button:hover:not(:disabled), button.active { color: var(--accent-bright); border-color: #617051; }
  button:focus-visible, input:focus-visible, select:focus-visible, textarea:focus-visible, summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  button:disabled { opacity: .4; cursor: not-allowed; }
  button.primary, .primary { color: #172014; border-color: var(--accent); background: var(--accent); font-weight: 800; }
  .professional-tabs button { padding: 7px 2px; font-size: .53rem; font-weight: 750; }
  .professional-panel { display: grid; gap: 8px; padding: 0 8px 12px; }
  details { display: grid; gap: 8px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface-soft); }
  summary { padding: 9px; color: var(--ink); font-size: .61rem; font-weight: 750; cursor: pointer; }
  summary small { float: right; color: var(--ink-faint); font-size: .49rem; font-weight: 500; }
  details > :not(summary) { margin-inline: 8px; }
  details > :last-child { margin-bottom: 8px; }
  input, select, textarea { min-width: 0; padding: 7px; border: 1px solid var(--line-strong); border-radius: 5px; color: var(--ink); background: var(--surface); font-size: .56rem; }
  label { color: var(--ink-soft); font-size: .54rem; }
  .field, .path-field, .workflow-editor label { display: grid; gap: 5px; }
  .field input, .field select, .workflow-editor input, .workflow-editor textarea { width: 100%; }
  .preset-row, .button-row, .segmented { display: flex; flex-wrap: wrap; gap: 5px; }
  .preset-row button, .button-row button, .segmented button { padding: 7px; font-size: .53rem; flex: 1; }
  .curve-grid, .comparison-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 5px; }
  .curve-grid button, .comparison-grid button { padding: 8px 4px; text-transform: capitalize; font-size: .52rem; }
  .five-fields { display: grid; grid-template-columns: repeat(5, 1fr); gap: 4px; }
  .four-fields, .perspective-fields { display: grid; grid-template-columns: repeat(4, 1fr); gap: 4px; }
  .coordinate-fields { display: grid; grid-template-columns: 1fr 1fr; gap: 5px; }
  .five-fields label, .four-fields label, .perspective-fields label, .coordinate-fields label { display: grid; gap: 3px; text-align: center; }
  .histogram { width: 100%; height: 120px; border: 1px solid var(--line); border-radius: 7px; background: #11130f; }
  .histogram polyline { fill: none; stroke-width: .35; opacity: .75; vector-effect: non-scaling-stroke; }
  .histogram .red { stroke: #f07070; } .histogram .green { stroke: #85d67d; } .histogram .blue { stroke: #7ca8ff; } .histogram .luma { stroke: white; opacity: .5; }
  .scope-stats, .metadata-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 5px; }
  .scope-stats span, .metadata-grid span { display: grid; gap: 3px; padding: 7px; border: 1px solid var(--line); border-radius: 6px; color: var(--ink-faint); font-size: .49rem; overflow-wrap: anywhere; }
  .scope-stats b, .metadata-grid b { color: var(--ink); font-size: .55rem; }
  .raw-card, .raw-result { display: grid; gap: 6px; padding: 8px; border: 1px solid var(--line); border-radius: 7px; background: var(--surface-soft); }
  .raw-card { grid-template-columns: 1fr auto; align-items: center; }
  .raw-card div, .raw-result span { display: grid; gap: 2px; }
  .raw-card strong, .raw-result b { color: var(--ink); font-size: .55rem; }
  .raw-card small, .raw-result span { color: var(--ink-faint); font-size: .49rem; }
  .raw-card button { padding: 6px; font-size: .51rem; }
  .raw-result { grid-template-columns: 1fr 1fr; }
  h3 { margin: 5px 0 0; color: var(--accent); font: 700 .55rem/1 var(--font-mono); text-transform: uppercase; }
  .record-card, .workflow-editor, .batch-summary, .batch-progress { display: grid; gap: 7px; padding: 9px; border: 1px solid var(--line); border-radius: 7px; background: var(--surface-soft); }
  .workflow-toolbar { display: grid; grid-template-columns: 1fr auto auto; gap: 4px; }
  .workflow-list { display: grid; gap: 5px; }
  .workflow-list article { display: grid; gap: 4px; padding: 6px; border: 1px solid var(--line); border-radius: 7px; }
  .workflow-list article.selected { border-color: #637451; }
  .workflow-main { border: 0; background: transparent; text-align: left; }
  .workflow-main span { display: grid; gap: 3px; }
  .workflow-main strong { color: var(--ink); font-size: .59rem; } .workflow-main small { color: var(--ink-faint); font-size: .49rem; }
  .workflow-list article > div { display: flex; gap: 3px; }
  .workflow-list article > div button { flex: 1; padding: 5px 2px; font-size: .47rem; }
  .workflow-editor ol { display: grid; gap: 4px; margin: 0; padding: 0; list-style: none; }
  .workflow-editor li { display: flex; align-items: center; justify-content: space-between; gap: 5px; color: var(--ink-soft); font-size: .51rem; }
  .workflow-editor li div { display: flex; gap: 2px; } .workflow-editor li button { width: 22px; height: 22px; }
  .path-field span { display: grid; grid-template-columns: 1fr auto; gap: 4px; }
  .check-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 5px; } .check-grid label { display: flex; align-items: center; gap: 5px; }
  .privacy-note, .empty-copy { margin: 0; color: var(--ink-faint); font-size: .55rem; line-height: 1.45; }
  .batch-summary ol { max-height: 100px; margin: 0; padding-left: 16px; overflow: auto; color: var(--ink-faint); font: .47rem/1.4 var(--font-mono); }
  .batch-progress progress { width: 100%; accent-color: var(--accent); }
  .batch-progress span, .batch-progress small { color: var(--ink-faint); font-size: .5rem; overflow-wrap: anywhere; }
  .batch-progress p { margin: 0; color: #e19a91; font-size: .49rem; }
  .pixel-result { display: flex; gap: 8px; align-items: center; padding: 8px; border: 1px solid var(--line); border-radius: 7px; }
  .pixel-result > span { width: 34px; height: 34px; border: 1px solid white; border-radius: 5px; }
  .pixel-result div { display: grid; gap: 3px; } .pixel-result b { color: var(--ink); font-size: .55rem; } .pixel-result small { color: var(--ink-faint); font-size: .49rem; }
  output { padding: 8px; border-radius: 6px; color: var(--accent); background: var(--surface-soft); text-align: center; font: 700 .62rem/1 var(--font-mono); }
</style>
