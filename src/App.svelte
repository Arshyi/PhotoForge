<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import ImageStage from './lib/components/ImageStage.svelte';
  import AnalysisPanel from './lib/components/AnalysisPanel.svelte';
  import ComponentsSettings from './lib/components/ComponentsSettings.svelte';
  import DiagnosticsSettings from './lib/components/DiagnosticsSettings.svelte';
  import GuidedEditPanel from './lib/components/GuidedEditPanel.svelte';
  import LocalAiPrivacy from './lib/components/LocalAiPrivacy.svelte';
  import ProfessionalWorkspace from './lib/components/ProfessionalWorkspace.svelte';
  import SelectionWorkspace from './lib/components/SelectionWorkspace.svelte';
  import RestorationPanel from './lib/components/RestorationPanel.svelte';
  import SliderControl from './lib/components/SliderControl.svelte';
  import StatusBar from './lib/components/StatusBar.svelte';
  import ToolButton from './lib/components/ToolButton.svelte';
  import WorkspaceSettings from './lib/components/WorkspaceSettings.svelte';
  import { EditHistory } from './lib/stores/history';
  import type {
    EditOperation,
    AnalysisResult,
    ExportResult,
    GuidedSettings,
    ImageQualityAnalysis,
    ImageMetadata,
    OpenImageResult,
    OperationType,
    PreviewResult
    ,ComparisonMode
    ,ExportProfile
    ,ShortcutBinding
  } from './lib/types/editor';
  import { errorMessage, formatBytes } from './lib/utils/format';
  import {
    defaultGuidedSettings,
    loadGuidedSettings,
    saveGuidedSettings
  } from './lib/utils/guided';
  import {
    baseOperation,
    cloneOperations,
    maskedOperation,
    operationLabels,
    operationSupportsMask,
    operationType,
    presets,
    replaceOperation,
    valueFor
  } from './lib/utils/operations';
  import { loadShortcuts, normalizeShortcut } from './lib/utils/workspace';
  import {
    cancelMaskOperation,
    colorRangeSelection,
    composeSelectionMasks,
    exportMaskFile,
    exportMaskPng,
    importMaskFile,
    importMaskPng,
    inspectSelectionMask,
    magicWandSelection,
    rasterizeSelection,
    refineSelection,
    transformSelection
  } from './lib/selections/commands';
  import {
    createNamedMask,
    createSelectionState,
    deleteNamedMask,
    duplicateNamedMask,
    loadNamedMask,
    moveNamedMask,
    operationModeFromModifiers,
    renameNamedMask,
    replaceNamedMask,
    SelectionHistory,
    setActiveMask,
    toggleNamedMask
  } from './lib/selections/state';
  import {
    createMaskFile,
    documentSelectionKey,
    loadSelectionSession,
    saveSelectionSession
  } from './lib/selections/serialization';
  import type {
    MaskOperation,
    MaskResult,
    NamedMask,
    SelectionGesture,
    SelectionShape,
    SelectionState,
    SelectionTool
  } from './lib/selections/types';

  const history = new EditHistory();
  const selectionHistory = new SelectionHistory();
  let operations: EditOperation[] = [];
  let metadata: ImageMetadata | null = null;
  let originalUrl: string | null = null;
  let previewUrl: string | null = null;
  let zoom = 100;
  let comparison = false;
  let comparisonPosition = 50;
  let comparisonMode: ComparisonMode = 'swipe';
  let gridOverlay = false;
  let crosshair = false;
  let processing = false;
  let previewCurrent = true;
  let processingTime = 0;
  let requestId = 0;
  let documentId = 0;
  let analysisRequestId = 0;
  let analysis: ImageQualityAnalysis | null = null;
  let analyzing = false;
  let activeOpenRequest = 0;
  let renderTimer: ReturnType<typeof setTimeout> | undefined;
  let renderInFlight = false;
  let previewQueued = false;
  let opening = false;
  let exporting = false;
  let settingsOpen = false;
  let settingsPage: 'general' | 'workspace' | 'components' | 'diagnostics' | 'privacy' = 'general';
  let componentConfigurationRevision = 0;
  let guidedSettings: GuidedSettings = { ...defaultGuidedSettings };
  let toast = '';
  let toastKind: 'error' | 'success' = 'success';
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let settingsCloseButton: HTMLButtonElement;
  let settingsDialog: HTMLDialogElement;
  let canUndo = false;
  let canRedo = false;
  let exportProfile: ExportProfile = 'lossless';
  let shortcuts: ShortcutBinding[] = [];
  let selectionState: SelectionState = createSelectionState();
  let selectionBusy = false;
  let maskRequestId = 0;
  let historyEvents: Array<'edit' | 'selection'> = [];
  let redoEvents: Array<'edit' | 'selection'> = [];
  let selectionPersistenceWarningShown = false;

  $: comparisonUsesSplitView = comparisonMode === 'split' || valueFor(operations, 'rotate', 0) % 360 !== 0;

  onMount(() => {
    guidedSettings = loadGuidedSettings();
    shortcuts = loadShortcuts();
    exportProfile = (localStorage.getItem('photoforge.lastExportProfile') as ExportProfile | null) ?? 'lossless';
    let unlisten: (() => void) | undefined;
    const persistBeforeClose = () => persistSelectionState();
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === 'drop' && event.payload.paths[0]) {
          void loadPath(event.payload.paths[0]);
        }
      })
      .then((cleanup) => (unlisten = cleanup))
      .catch(() => undefined);

    const handleKeys = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && settingsOpen) {
        event.preventDefault();
        closeSettings();
        return;
      }
      const target = event.target as HTMLElement | null;
      const textFocused = Boolean(
        target?.matches('input, textarea, select, [contenteditable="true"]')
      );
      if (!textFocused && metadata) {
        const command = event.ctrlKey || event.metaKey;
        const key = event.key.toLowerCase();
        if (command && key === 'a') {
          event.preventDefault();
          void applyMaskOperation({ type: 'select_all' });
          return;
        }
        if (command && !event.shiftKey && key === 'd') {
          event.preventDefault();
          void applyMaskOperation({ type: 'deselect' });
          return;
        }
        if (command && event.shiftKey && key === 'i') {
          event.preventDefault();
          void applyMaskOperation({ type: 'invert' });
          return;
        }
        if (!command && !event.altKey && key === 'q') {
          event.preventDefault();
          commitSelectionState({
            ...selectionState,
            overlay: { ...selectionState.overlay, visible: !selectionState.overlay.visible }
          });
          return;
        }
        if (!command && !event.altKey && key === 'm') {
          event.preventDefault();
          setSelectionTool(selectionState.tool === 'rectangle' ? 'ellipse' : 'rectangle');
          return;
        }
        if (!command && !event.altKey && key === 'l') {
          event.preventDefault();
          setSelectionTool(selectionState.tool === 'freehand' ? 'polygon' : 'freehand');
          return;
        }
        const toolShortcuts: Partial<Record<string, SelectionTool>> = {
          w: 'magic_wand',
          b: 'brush',
          e: 'eraser'
        };
        if (!command && !event.altKey && toolShortcuts[key]) {
          event.preventDefault();
          setSelectionTool(toolShortcuts[key] as SelectionTool);
          return;
        }
        if (event.key === 'Escape' && selectionBusy) {
          event.preventDefault();
          void cancelCurrentMaskOperation();
          return;
        }
      }
      const action = shortcutAction(event);
      if (action === 'Open image') {
        event.preventDefault();
        void chooseImage();
      } else if (action === 'Export image') {
        event.preventDefault();
        void exportImage();
      } else if (action === 'Undo') {
        event.preventDefault();
        undo();
      } else if (action === 'Redo') {
        event.preventDefault();
        redo();
      } else if (action === 'Compare') {
        event.preventDefault(); comparison = !comparison;
      } else if (action === 'Zoom in') {
        event.preventDefault(); zoom = Math.min(1600, zoom + (zoom >= 400 ? 100 : 25));
      } else if (action === 'Zoom out') {
        event.preventDefault(); zoom = Math.max(25, zoom - (zoom > 400 ? 100 : 25));
      } else if (action === 'Pixel inspector') {
        event.preventDefault(); crosshair = !crosshair;
      } else if (action === 'Crop' || action === 'Straighten') {
        event.preventDefault(); gridOverlay = true;
      }
    };
    window.addEventListener('keydown', handleKeys);
    window.addEventListener('beforeunload', persistBeforeClose);

    return () => {
      unlisten?.();
      window.removeEventListener('keydown', handleKeys);
      window.removeEventListener('beforeunload', persistBeforeClose);
      if (renderTimer) clearTimeout(renderTimer);
      if (toastTimer) clearTimeout(toastTimer);
      previewQueued = false;
      analysisRequestId += 1;
    };
  });

  function shortcutAction(event: KeyboardEvent): string | undefined {
    const parts: string[] = [];
    if (event.ctrlKey || event.metaKey) parts.push('ctrl');
    if (event.altKey) parts.push('alt');
    if (event.shiftKey) parts.push('shift');
    const key = event.key === ' ' ? 'space' : event.key.toLowerCase();
    if (!['control', 'meta', 'alt', 'shift'].includes(key)) parts.push(key);
    const normalized = normalizeShortcut(parts.join('+'));
    return shortcuts.find((binding) => normalizeShortcut(binding.keys) === normalized)?.action;
  }

  function notify(message: string, kind: 'error' | 'success' = 'success') {
    toast = message;
    toastKind = kind;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ''), 4200);
  }

  async function chooseImage() {
    if (opening) return;
    try {
      const path = await open({
        multiple: false,
        directory: false,
        title: 'Open a photo',
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
      });
      if (typeof path === 'string') await loadPath(path);
    } catch (error) {
      notify(errorMessage(error), 'error');
    }
  }

  async function loadPath(path: string) {
    persistSelectionState();
    const ownOpenRequest = ++requestId;
    activeOpenRequest = ownOpenRequest;
    opening = true;
    processing = true;
    previewCurrent = false;
    previewQueued = false;
    if (renderTimer) clearTimeout(renderTimer);
    try {
      const result = await invoke<OpenImageResult>('open_image', {
        path,
        requestId: ownOpenRequest
      });
      if (!result.isCurrent || activeOpenRequest !== ownOpenRequest) return;
      history.clear();
      operations = [];
      metadata = result.metadata;
      documentId = result.documentId;
      analysis = null;
      originalUrl = result.originalPreviewDataUrl;
      previewUrl = result.previewDataUrl;
      processingTime = result.processingTimeMs;
      zoom = 100;
      comparison = false;
      const selectionKey = documentSelectionKey(
        result.metadata.filename,
        result.metadata.width,
        result.metadata.height
      );
      selectionState = selectionHistory.replace(
        loadSelectionSession(selectionKey, result.metadata.width, result.metadata.height)
      );
      if (!selectionState.activeMask) selectionState = { ...selectionState, applyScope: 'global' };
      historyEvents = [];
      redoEvents = [];
      selectionPersistenceWarningShown = false;
      syncHistoryActions();
      previewCurrent = true;
      notify(`${result.metadata.filename} opened locally`);
      if (selectionState.activeMask && !selectionState.activeDiagnostics) {
        void refreshActiveMaskDiagnostics();
      }
      void requestAnalysis(result.documentId);
    } catch (error) {
      if (activeOpenRequest === ownOpenRequest) {
        previewCurrent = true;
        notify(errorMessage(error), 'error');
      }
    } finally {
      if (activeOpenRequest === ownOpenRequest) {
        opening = false;
        processing = renderInFlight;
      }
    }
  }

  async function requestAnalysis(ownDocument: number) {
    const ownRequest = ++analysisRequestId;
    analyzing = true;
    try {
      const result = await invoke<AnalysisResult>('analyze_image', {
        documentId: ownDocument,
        requestId: ownRequest
      });
      if (
        result.isCurrent &&
        result.requestId === analysisRequestId &&
        result.documentId === documentId &&
        result.analysis
      ) {
        analysis = result.analysis;
      }
    } catch (error) {
      if (ownRequest === analysisRequestId && ownDocument === documentId) {
        notify(errorMessage(error), 'error');
      }
    } finally {
      if (ownRequest === analysisRequestId) analyzing = false;
    }
  }

  function schedulePreview() {
    if (!metadata) return;
    requestId += 1;
    previewCurrent = false;
    previewQueued = true;
    if (renderTimer) clearTimeout(renderTimer);
    if (operations.length === 0) {
      previewQueued = false;
      previewUrl = originalUrl;
      processingTime = 0;
      previewCurrent = true;
      return;
    }
    renderTimer = setTimeout(() => void drainPreviewQueue(), 120);
  }

  async function drainPreviewQueue() {
    if (renderInFlight || !metadata || opening) return;
    renderInFlight = true;
    try {
      while (previewQueued && metadata && !opening) {
        previewQueued = false;
        const ownRequest = requestId;
        const ownDocument = documentId;
        const pipeline = cloneOperations(operations);
        processing = true;
        try {
          const result = await invoke<PreviewResult>('render_preview', {
            operations: pipeline,
            documentId: ownDocument,
            requestId: ownRequest
          });
          if (
            result.isCurrent &&
            result.requestId === requestId &&
            ownDocument === documentId
          ) {
            previewUrl = result.previewDataUrl;
            processingTime = result.processingTimeMs;
            previewCurrent = true;
          }
        } catch (error) {
          if (ownRequest === requestId && ownDocument === documentId) {
            previewCurrent = true;
            notify(errorMessage(error), 'error');
          }
        }
      }
    } finally {
      renderInFlight = false;
      if (!opening) processing = false;
      if (previewQueued && !opening) void drainPreviewQueue();
    }
  }

  function commit(next: EditOperation[], coalesceKey?: string) {
    const scoped = scopeChangedOperations(next);
    const before = JSON.stringify(operations);
    operations = history.commit(scoped, coalesceKey);
    if (JSON.stringify(operations) !== before) recordHistoryEvent('edit');
    syncHistoryActions();
    schedulePreview();
  }

  function commitGlobal(next: EditOperation[], coalesceKey?: string) {
    const before = JSON.stringify(operations);
    operations = history.commit(cloneOperations(next), coalesceKey);
    if (JSON.stringify(operations) !== before) recordHistoryEvent('edit');
    syncHistoryActions();
    schedulePreview();
  }

  function scopeChangedOperations(next: EditOperation[]): EditOperation[] {
    return next.map((candidate) => {
      if (candidate.type === 'masked') return structuredClone(candidate);
      const previous = operations.find((value) => operationType(value) === candidate.type);
      if (previous && JSON.stringify(previous) === JSON.stringify(candidate)) {
        return structuredClone(candidate);
      }
      const base = baseOperation(candidate);
      if (
        selectionState.applyScope === 'global' ||
        !selectionState.activeMask ||
        !operationSupportsMask(base)
      ) {
        return base;
      }
      return maskedOperation(base, selectionState.activeMask, selectionState.applyScope);
    });
  }

  function commitSelectionState(next: SelectionState, coalesceKey?: string) {
    const before = JSON.stringify(selectionState);
    selectionState = selectionHistory.commit(next, coalesceKey);
    if (JSON.stringify(selectionState) !== before) recordHistoryEvent('selection');
    persistSelectionState();
    syncHistoryActions();
  }

  function recordHistoryEvent(kind: 'edit' | 'selection') {
    historyEvents = [...historyEvents, kind];
    redoEvents = [];
  }

  function syncHistoryActions() {
    canUndo = history.canUndo || selectionHistory.canUndo;
    canRedo = history.canRedo || selectionHistory.canRedo;
  }

  function setNumeric(
    type: OperationType,
    value: number,
    defaultValue: number,
    build: (input: number) => EditOperation
  ) {
    commit(
      replaceOperation(operations, build(value), Math.abs(value - defaultValue) > 0.0001),
      type
    );
  }

  function toggle(operation: EditOperation) {
    const enabled = !operations.some((candidate) => operationType(candidate) === operationType(operation));
    commit(replaceOperation(operations, operation, enabled));
  }

  function setRestoration(
    operation: EditOperation,
    enabled: boolean,
    coalesceKey?: string
  ) {
    commit(replaceOperation(operations, operation, enabled), coalesceKey);
  }

  function rotate(delta: number) {
    const current = valueFor(operations, 'rotate', 0);
    let degrees = (current + delta) % 360;
    if (degrees < 0) degrees += 360;
    commit(replaceOperation(operations, { type: 'rotate', degrees }, degrees !== 0));
  }

  function undo() {
    while (historyEvents.length) {
      const kind = historyEvents.at(-1) as 'edit' | 'selection';
      historyEvents = historyEvents.slice(0, -1);
      if (kind === 'edit' && history.canUndo) {
        operations = history.undo();
        redoEvents = [...redoEvents, kind];
        schedulePreview();
        break;
      }
      if (kind === 'selection' && selectionHistory.canUndo) {
        selectionState = selectionHistory.undo();
        redoEvents = [...redoEvents, kind];
        persistSelectionState();
        break;
      }
    }
    syncHistoryActions();
  }

  function redo() {
    while (redoEvents.length) {
      const kind = redoEvents.at(-1) as 'edit' | 'selection';
      redoEvents = redoEvents.slice(0, -1);
      if (kind === 'edit' && history.canRedo) {
        operations = history.redo();
        historyEvents = [...historyEvents, kind];
        schedulePreview();
        break;
      }
      if (kind === 'selection' && selectionHistory.canRedo) {
        selectionState = selectionHistory.redo();
        historyEvents = [...historyEvents, kind];
        persistSelectionState();
        break;
      }
    }
    syncHistoryActions();
  }

  function undoSelectionOnly() {
    if (!selectionHistory.canUndo) return;
    selectionState = selectionHistory.undo();
    const index = historyEvents.lastIndexOf('selection');
    if (index >= 0) historyEvents = historyEvents.filter((_, candidate) => candidate !== index);
    redoEvents = [...redoEvents, 'selection'];
    persistSelectionState();
    syncHistoryActions();
  }

  function redoSelectionOnly() {
    if (!selectionHistory.canRedo) return;
    selectionState = selectionHistory.redo();
    const index = redoEvents.lastIndexOf('selection');
    if (index >= 0) redoEvents = redoEvents.filter((_, candidate) => candidate !== index);
    historyEvents = [...historyEvents, 'selection'];
    persistSelectionState();
    syncHistoryActions();
  }

  function reset() {
    if (!metadata || operations.length === 0) return;
    operations = history.reset();
    recordHistoryEvent('edit');
    syncHistoryActions();
    schedulePreview();
  }

  function applyPreset(presetOperations: EditOperation[]) {
    commit(presetOperations);
  }

  function applyGuidedPlan(planOperations: EditOperation[]) {
    commitGlobal(planOperations);
  }

  function persistSelectionState() {
    if (!selectionState.documentKey) return;
    const persisted = saveSelectionSession(selectionState);
    if (!persisted && !selectionPersistenceWarningShown) {
      selectionPersistenceWarningShown = true;
      notify('This mask set exceeds bounded session storage; export important masks as local files.', 'error');
    }
  }

  function setSelectionTool(tool: SelectionTool) {
    commitSelectionState({ ...selectionState, tool });
  }

  function updateSelectionState(next: SelectionState, coalesceKey?: string) {
    if (!next.activeMask && next.applyScope !== 'global') next = { ...next, applyScope: 'global' };
    commitSelectionState(next, coalesceKey);
  }

  async function refreshActiveMaskDiagnostics() {
    if (!selectionState.activeMask) return;
    try {
      const diagnostics = await inspectSelectionMask(selectionState.activeMask);
      if (selectionState.activeMask) {
        selectionState = { ...selectionState, activeDiagnostics: diagnostics };
        persistSelectionState();
      }
    } catch (error) {
      selectionState = selectionHistory.replace(
        setActiveMask(selectionState, null, null)
      );
      syncHistoryActions();
      notify(errorMessage(error), 'error');
    }
  }

  async function handleSelectionGesture(gesture: SelectionGesture) {
    if (!metadata || selectionBusy) return;
    const configuredMode = operationModeFromModifiers(
      selectionState.mode,
      gesture.shiftKey,
      gesture.altKey
    );
    const mode = gesture.tool === 'eraser' ? 'subtract' : configuredMode;
    const ownRequest = ++maskRequestId;
    selectionBusy = true;
    try {
      let result: MaskResult;
      if (gesture.tool === 'magic_wand') {
        result = await magicWandSelection({
          point: gesture.points[0],
          options: {
            tolerance: selectionState.settings.wandTolerance,
            connectivity: selectionState.settings.wandConnectivity,
            antiAlias: selectionState.settings.wandAntiAlias,
            contiguous: selectionState.settings.wandContiguous
          },
          mode,
          base: selectionState.activeMask,
          sampleMerged: selectionState.settings.sampleMerged,
          operations: cloneOperations(operations),
          documentId,
          requestId: ownRequest
        });
      } else if (gesture.tool === 'color_range') {
        result = await colorRangeSelection({
          samples: gesture.points,
          options: {
            tolerance: selectionState.settings.colorTolerance,
            luminanceSensitivity: selectionState.settings.luminanceSensitivity,
            hueSensitivity: selectionState.settings.hueSensitivity,
            saturationSensitivity: selectionState.settings.saturationSensitivity
          },
          mode,
          base: selectionState.activeMask,
          sampleMerged: selectionState.settings.sampleMerged,
          operations: cloneOperations(operations),
          documentId,
          requestId: ownRequest
        });
      } else {
        const shape = selectionShape(gesture);
        if (!shape) return;
        result = await rasterizeSelection({
          width: metadata.width,
          height: metadata.height,
          shape,
          mode,
          base: selectionState.activeMask,
          documentId,
          requestId: ownRequest
        });
      }
      acceptMaskResult(result, gesture.tool);
    } catch (error) {
      if (ownRequest === maskRequestId && !isMaskCancellation(error)) notify(errorMessage(error), 'error');
    } finally {
      if (ownRequest === maskRequestId) selectionBusy = false;
    }
  }

  function selectionShape(gesture: SelectionGesture): SelectionShape | null {
    const [start, end] = gesture.points;
    if ((gesture.tool === 'rectangle' || gesture.tool === 'ellipse') && start && end) {
      return { type: gesture.tool, start, end };
    }
    if (gesture.tool === 'freehand' && gesture.points.length >= 3) {
      return { type: 'freehand', points: gesture.points };
    }
    if (gesture.tool === 'polygon' && gesture.points.length >= 3) {
      return { type: 'polygon', points: gesture.points };
    }
    if ((gesture.tool === 'brush' || gesture.tool === 'eraser') && gesture.points.length) {
      return {
        type: 'brush',
        points: gesture.points,
        diameter: selectionState.settings.brushDiameter,
        hardness: selectionState.settings.brushHardness,
        opacity: selectionState.settings.brushOpacity
      };
    }
    return null;
  }

  async function applyMaskOperation(operation: MaskOperation) {
    if (!metadata || selectionBusy) return;
    if (operation.type === 'deselect') {
      commitSelectionState({
        ...setActiveMask(selectionState, null, null),
        applyScope: 'global'
      });
      return;
    }
    const ownRequest = ++maskRequestId;
    selectionBusy = true;
    try {
      let result: MaskResult | null = null;
      if (operation.type === 'select_all') {
        result = await rasterizeSelection({
            width: metadata.width,
            height: metadata.height,
            shape: {
              type: 'rectangle',
              start: { x: 0, y: 0 },
              end: { x: metadata.width, y: metadata.height }
            },
            mode: 'replace',
            base: null,
            documentId,
            requestId: ownRequest
          });
      } else if (selectionState.activeMask && operation.type === 'refine') {
        result = await refineSelection({
          mask: selectionState.activeMask,
          operation,
          edgeStrength: 0.7,
          sampleMerged: selectionState.settings.sampleMerged,
          operations: cloneOperations(operations),
          documentId,
          requestId: ownRequest
        });
      } else if (selectionState.activeMask) {
        result = await transformSelection({
              mask: selectionState.activeMask,
              operation,
              documentId,
              requestId: ownRequest
            });
      }
      if (result) acceptMaskResult(result, operation.type);
    } catch (error) {
      if (ownRequest === maskRequestId && !isMaskCancellation(error)) notify(errorMessage(error), 'error');
    } finally {
      if (ownRequest === maskRequestId) selectionBusy = false;
    }
  }

  function acceptMaskResult(result: MaskResult, source: string) {
    if (!result.isCurrent || result.requestId !== maskRequestId || result.documentId !== documentId) return;
    processingTime = result.processingTimeMs;
    commitSelectionState(
      setActiveMask(
        { ...selectionState, overlay: { ...selectionState.overlay, visible: true } },
        result.mask,
        result.diagnostics
      )
    );
    notify(`${source.replaceAll('_', ' ')} selection updated`);
  }

  async function cancelCurrentMaskOperation() {
    if (!selectionBusy) return;
    await cancelMaskOperation(maskRequestId).catch(() => false);
  }

  function isMaskCancellation(error: unknown): boolean {
    return Boolean(
      error && typeof error === 'object' && 'code' in error && error.code === 'mask_cancelled'
    );
  }

  async function handleNamedMaskAction(
    action: 'create' | 'rename' | 'duplicate' | 'delete' | 'visible' | 'locked' | 'up' | 'down' | 'load' | 'combine' | 'replace' | 'export' | 'export_png',
    id?: string,
    value?: string
  ) {
    if (action === 'create') {
      commitSelectionState(createNamedMask(selectionState, value ?? ''));
      return;
    }
    if (!id) return;
    if (action === 'rename') commitSelectionState(renameNamedMask(selectionState, id, value ?? ''));
    else if (action === 'duplicate') commitSelectionState(duplicateNamedMask(selectionState, id));
    else if (action === 'delete') commitSelectionState(deleteNamedMask(selectionState, id));
    else if (action === 'visible' || action === 'locked') commitSelectionState(toggleNamedMask(selectionState, id, action));
    else if (action === 'up' || action === 'down') commitSelectionState(moveNamedMask(selectionState, id, action === 'up' ? -1 : 1));
    else if (action === 'load') {
      commitSelectionState(loadNamedMask(selectionState, id));
      await refreshActiveMaskDiagnostics();
    } else if (action === 'combine') {
      await combineNamedMask(id);
    } else if (action === 'replace') commitSelectionState(replaceNamedMask(selectionState, id));
    else {
      const named = selectionState.namedMasks.find((mask) => mask.id === id);
      if (named) await exportNamedMask(named, action === 'export_png');
    }
  }

  async function combineNamedMask(id: string) {
    const named = selectionState.namedMasks.find((mask) => mask.id === id);
    if (!named || !selectionState.activeMask || selectionBusy) return;
    const ownRequest = ++maskRequestId;
    selectionBusy = true;
    try {
      const result = await composeSelectionMasks({
        base: selectionState.activeMask,
        incoming: named.mask,
        mode: selectionState.mode,
        documentId,
        requestId: ownRequest
      });
      acceptMaskResult(result, `${selectionState.mode} ${named.name}`);
    } catch (error) {
      if (ownRequest === maskRequestId && !isMaskCancellation(error)) notify(errorMessage(error), 'error');
    } finally {
      if (ownRequest === maskRequestId) selectionBusy = false;
    }
  }

  async function importSelectionMask(format: 'json' | 'png') {
    if (!metadata) return;
    try {
      const path = await open({
        multiple: false,
        directory: false,
        title: format === 'json' ? 'Import PhotoForge mask' : 'Import grayscale mask',
        filters: [format === 'json'
          ? { name: 'PhotoForge mask', extensions: ['json'] }
          : { name: 'Grayscale PNG mask', extensions: ['png'] }]
      });
      if (typeof path !== 'string') return;
      if (format === 'json') {
        const document = await importMaskFile(path);
        ensureMaskDimensions(document.mask);
        const duplicateId = selectionState.namedMasks.some((mask) => mask.id === document.id);
        const id = duplicateId ? `${document.id}-${Date.now().toString(36)}` : document.id;
        const named: NamedMask = {
          id,
          name: document.name,
          mask: document.mask,
          visible: true,
          locked: false,
          createdAt: document.metadata.createdAt || new Date().toISOString(),
          modifiedAt: document.metadata.modifiedAt || new Date().toISOString(),
          sourceTool: document.metadata.sourceTool as SelectionTool | undefined
        };
        commitSelectionState({
          ...setActiveMask(selectionState, document.mask, await inspectSelectionMask(document.mask)),
          namedMasks: [...selectionState.namedMasks, named]
        });
      } else {
        const mask = await importMaskPng(path);
        ensureMaskDimensions(mask);
        const diagnostics = await inspectSelectionMask(mask);
        const next = setActiveMask(selectionState, mask, diagnostics);
        commitSelectionState(createNamedMask(next, 'Imported PNG'));
      }
      notify('Mask imported locally');
    } catch (error) {
      notify(errorMessage(error), 'error');
    }
  }

  async function exportNamedMask(named: NamedMask, png: boolean) {
    try {
      const path = await save({
        title: png ? 'Export grayscale mask' : 'Export PhotoForge mask',
        defaultPath: `${named.name.replace(/[^a-z0-9_-]+/gi, '-')}${png ? '.png' : '.photoforge-mask.json'}`,
        filters: [png
          ? { name: 'Grayscale PNG mask', extensions: ['png'] }
          : { name: 'PhotoForge mask', extensions: ['json'] }]
      });
      if (!path) return;
      if (png) await exportMaskPng(path, named.mask);
      else await exportMaskFile(
        path,
        createMaskFile(named.id, named.name, named.mask, named.createdAt, named.modifiedAt, named.sourceTool)
      );
      notify(`Exported ${png ? 'grayscale PNG' : 'PhotoForge mask'}`);
    } catch (error) {
      notify(errorMessage(error), 'error');
    }
  }

  function ensureMaskDimensions(mask: { width: number; height: number }) {
    if (!metadata || mask.width !== metadata.width || mask.height !== metadata.height) {
      throw new Error(`Mask dimensions must match ${metadata?.width ?? 0} × ${metadata?.height ?? 0}.`);
    }
  }

  async function exportImage() {
    if (!metadata || exporting || opening) return;
    try {
      const stem = metadata.filename.replace(/\.[^.]+$/, '');
      const extension = ['web', 'print', 'high_jpeg'].includes(exportProfile)
        ? 'jpg'
        : exportProfile === 'maximum_compression'
          ? 'webp'
          : 'png';
      const outputPath = await save({
        title: 'Export edited photo',
        defaultPath: `${stem}-photoforge.${extension}`,
        filters: [
          { name: 'PNG image', extensions: ['png'] },
          { name: 'JPEG image', extensions: ['jpg', 'jpeg'] },
          { name: 'WebP image', extensions: ['webp'] }
        ]
      });
      if (!outputPath) return;
      exporting = true;
      localStorage.setItem('photoforge.lastExportProfile', exportProfile);
      const result = await invoke<ExportResult>('export_with_profile', {
        outputPath,
        operations,
        profile: exportProfile
      });
      processingTime = result.processingTimeMs;
      notify(`Exported ${result.width} × ${result.height} image`);
    } catch (error) {
      notify(errorMessage(error), 'error');
    } finally {
      exporting = false;
    }
  }

  function active(type: OperationType): boolean {
    return operations.some((operation) => operationType(operation) === type);
  }

  const percent = (value: number) => `${Math.round(value * 100)}%`;

  async function openSettings() {
    settingsPage = 'general';
    settingsOpen = true;
    await tick();
    settingsCloseButton?.focus();
  }

  async function closeSettings() {
    if (settingsPage === 'components') componentConfigurationRevision += 1;
    if (settingsPage === 'workspace') shortcuts = loadShortcuts();
    settingsOpen = false;
    await tick();
    document.querySelector<HTMLButtonElement>('button[aria-label="Settings"]')?.focus();
  }

  function updateProfessionalView(view: { grid?: boolean; crosshair?: boolean; comparisonMode?: ComparisonMode; zoom?: number }) {
    if (view.grid !== undefined) gridOverlay = view.grid;
    if (view.crosshair !== undefined) crosshair = view.crosshair;
    if (view.zoom !== undefined) zoom = Math.max(25, Math.min(1600, view.zoom));
    if (view.comparisonMode) { comparisonMode = view.comparisonMode; comparison = true; }
  }

  function updateGuidedSetting(key: keyof GuidedSettings, value: boolean) {
    guidedSettings = { ...guidedSettings, [key]: value };
    saveGuidedSettings(guidedSettings);
  }

  function trapSettingsFocus(event: KeyboardEvent) {
    if (event.key !== 'Tab') return;
    const focusable = Array.from(
      settingsDialog.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])'
      )
    );
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last?.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first?.focus();
    }
  }
</script>

<svelte:head>
  <title>{metadata ? `${metadata.filename} — PhotoForge` : 'PhotoForge'}</title>
</svelte:head>

<div class="app-shell" inert={settingsOpen} aria-hidden={settingsOpen}>
  <header class="topbar">
    <div class="brand" aria-label="PhotoForge">
      <span class="brand-mark" aria-hidden="true"><b></b><i></i></span>
      <span><strong>Photo</strong>Forge</span>
      <em>LOCAL</em>
    </div>

    <nav class="primary-actions" aria-label="File actions">
      <ToolButton label="Open" icon="＋" primary disabled={opening} onclick={chooseImage} />
      <ToolButton
        label={exporting ? 'Exporting' : 'Export'}
        icon="⇩"
        disabled={!metadata || exporting || opening}
        onclick={exportImage}
      />
      <select aria-label="Export profile" bind:value={exportProfile} title="Remembered export profile">
        <option value="web">Web</option>
        <option value="print">Print</option>
        <option value="archive">Archive</option>
        <option value="lossless">Lossless</option>
        <option value="high_jpeg">High JPEG</option>
        <option value="maximum_compression">Maximum compression</option>
      </select>
    </nav>

    <div class="history-actions" aria-label="Edit history">
      <ToolButton label="Undo" icon="↶" disabled={!canUndo} title="Undo (Ctrl+Z)" onclick={undo} />
      <ToolButton label="Redo" icon="↷" disabled={!canRedo} title="Redo (Ctrl+Y)" onclick={redo} />
      <ToolButton label="Reset" icon="⌫" disabled={!metadata || !operations.length} onclick={reset} />
    </div>

    <div class="top-spacer"></div>
    <div class="privacy-chip" title="No cloud services"><span></span> Local-first</div>
    <ToolButton label="Settings" icon="⚙" onclick={openSettings} />
  </header>

  <main>
    <ImageStage
      {originalUrl}
      {previewUrl}
      filename={metadata?.filename ?? ''}
      {comparison}
      {comparisonPosition}
      splitComparison={comparisonUsesSplitView}
      {zoom}
      {comparisonMode}
      {gridOverlay}
      {crosshair}
      {processing}
      stale={!previewCurrent}
      onopen={chooseImage}
      oncomparisonchange={(value) => (comparisonPosition = value)}
      imageWidth={metadata?.width ?? 0}
      imageHeight={metadata?.height ?? 0}
      selectionTool={comparisonUsesSplitView ? 'none' : selectionState.tool}
      activeMask={selectionState.activeMask}
      overlaySettings={selectionState.overlay}
      brushDiameter={selectionState.settings.brushDiameter}
      fixedAspect={selectionState.settings.fixedAspect}
      fromCenter={selectionState.settings.fromCenter}
      onselectiongesture={handleSelectionGesture}
      onselectioncancel={() => undefined}
    />

    <aside aria-label="Editing controls">
      <div class="inspector-title">
        <div>
          <span>Adjustments</span>
          <small>Non-destructive pipeline</small>
        </div>
        <span class="count">{operations.length}</span>
      </div>

      {#if metadata}
        <div class="metadata-card">
          <div class="file-glyph">{metadata.format.slice(0, 3)}</div>
          <div>
            <strong title={metadata.filename}>{metadata.filename}</strong>
            <span>{metadata.width} × {metadata.height} · {formatBytes(metadata.fileSize)}</span>
          </div>
        </div>
      {/if}

      <div
        class="scroll-panel"
        class:disabled={!metadata || opening}
        inert={!metadata || opening}
        aria-disabled={!metadata || opening}
      >
        <SelectionWorkspace
          state={selectionState}
          disabled={!metadata || opening}
          busy={selectionBusy}
          canUndo={selectionHistory.canUndo}
          canRedo={selectionHistory.canRedo}
          onstatechange={updateSelectionState}
          onoperation={applyMaskOperation}
          onnamedaction={handleNamedMaskAction}
          onimport={importSelectionMask}
          onundo={undoSelectionOnly}
          onredo={redoSelectionOnly}
          oncancel={cancelCurrentMaskOperation}
        />

        <GuidedEditPanel
          {documentId}
          ready={Boolean(metadata && analysis)}
          disabled={opening}
          settings={guidedSettings}
          configurationRevision={componentConfigurationRevision}
          onapply={applyGuidedPlan}
          onmessage={notify}
        />

        <ProfessionalWorkspace
          {documentId}
          {metadata}
          {operations}
          oncommit={commit}
          onmessage={notify}
          onviewchange={updateProfessionalView}
        />

        <section class="tool-section">
          <h2><span>☀</span> Light</h2>
          <SliderControl
            label="Brightness"
            value={valueFor(operations, 'brightness', 0)}
            min={-0.5}
            max={0.5}
            step={0.01}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('brightness', value, 0, (amount) => ({ type: 'brightness', amount }))}
          />
          <SliderControl
            label="Contrast"
            value={valueFor(operations, 'contrast', 0)}
            min={-0.75}
            max={0.75}
            step={0.01}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('contrast', value, 0, (amount) => ({ type: 'contrast', amount }))}
          />
          <SliderControl
            label="Gamma"
            value={valueFor(operations, 'gamma', 1)}
            min={0.3}
            max={2.5}
            step={0.01}
            defaultValue={1}
            format={(value) => value.toFixed(2)}
            onchange={(value) => setNumeric('gamma', value, 1, (input) => ({ type: 'gamma', value: input }))}
          />
        </section>

        <AnalysisPanel {analysis} {analyzing} />

        <RestorationPanel {operations} onset={setRestoration} />

        <section class="tool-section">
          <h2><span>◒</span> Color</h2>
          <SliderControl
            label="Saturation"
            value={valueFor(operations, 'saturation', 0)}
            min={-1}
            max={1}
            step={0.01}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('saturation', value, 0, (amount) => ({ type: 'saturation', amount }))}
          />
          <div class="toggle-grid">
            <button class:active={active('grayscale')} type="button" on:click={() => toggle({ type: 'grayscale' })}>
              <span>◐</span> Grayscale
            </button>
            <button class:active={active('sepia')} type="button" on:click={() => toggle({ type: 'sepia' })}>
              <span>◑</span> Sepia
            </button>
          </div>
        </section>

        <section class="tool-section">
          <h2><span>✦</span> Detail</h2>
          <SliderControl
            label="Blur"
            value={valueFor(operations, 'gaussian_blur', 0)}
            min={0}
            max={12}
            step={0.1}
            defaultValue={0}
            format={(value) => value.toFixed(1)}
            onchange={(value) =>
              setNumeric('gaussian_blur', value, 0, (radius) => ({ type: 'gaussian_blur', radius }))}
          />
          <SliderControl
            label="Sharpen"
            value={valueFor(operations, 'sharpen', 0)}
            min={0}
            max={2}
            step={0.02}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('sharpen', value, 0, (strength) => ({ type: 'sharpen', strength }))}
          />
          <p class="truth-note">Sharpening improves edge contrast; it does not recover missing detail.</p>
        </section>

        <section class="tool-section">
          <h2><span>⌘</span> Transform</h2>
          <div class="transform-grid">
            <button type="button" title="Rotate left" on:click={() => rotate(-90)}>↶<small>Left</small></button>
            <button type="button" title="Rotate right" on:click={() => rotate(90)}>↷<small>Right</small></button>
            <button
              type="button"
              class:active={active('reflect_horizontal')}
              title="Reflect horizontally"
              on:click={() => toggle({ type: 'reflect_horizontal' })}
            >⇋<small>Reflect</small></button>
          </div>
        </section>

        <section class="tool-section presets-section">
          <h2><span>▦</span> Presets</h2>
          <div class="preset-list">
            {#each presets as preset}
              <button type="button" on:click={() => applyPreset(preset.operations)}>
                <span><strong>{preset.name}</strong><small>{preset.description}</small></span>
                <b>›</b>
              </button>
            {/each}
          </div>
        </section>

        {#if operations.length}
          <section class="tool-section pipeline-section" aria-labelledby="pipeline-heading">
            <h2 id="pipeline-heading"><span>≡</span> Active Pipeline</h2>
            <ol class="pipeline-list">
              {#each operations as operation, index}
                <li><span>{index + 1}</span>{operationLabels[operationType(operation)]}{operation.type === 'masked' ? ` · ${operation.invert ? 'Outside mask' : 'Inside mask'}` : ''}</li>
              {/each}
            </ol>
          </section>
        {/if}
      </div>
    </aside>
  </main>

  <div class="viewbar" aria-label="Preview controls">
    <button type="button" class:active={comparison} disabled={!metadata || opening} on:click={() => (comparison = !comparison)}>
      ◫ <span>Compare</span>
    </button>
    <i></i>
    <button type="button" disabled={!metadata || opening} aria-label="Zoom out" on:click={() => (zoom = Math.max(25, zoom - 25))}>−</button>
    <span class="zoom-value">{zoom}%</span>
    <button type="button" disabled={!metadata || opening} aria-label="Zoom in" on:click={() => (zoom = Math.min(1600, zoom + (zoom >= 400 ? 100 : 25)))}>＋</button>
    <button type="button" disabled={!metadata || opening} on:click={() => (zoom = 100)}>Fit</button>
  </div>

  <StatusBar
    dimensions={metadata ? `${metadata.width} × ${metadata.height} · ${metadata.format}` : 'No image loaded'}
    {zoom}
    operationCount={operations.length}
    {processingTime}
    isCurrent={previewCurrent}
  />
</div>

{#if toast}
  <div class="toast" class:error={toastKind === 'error'} role="status">
    <span>{toastKind === 'error' ? '!' : '✓'}</span>{toast}
    <button type="button" aria-label="Dismiss message" on:click={() => (toast = '')}>×</button>
  </div>
{/if}

{#if settingsOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    on:click={(event) => event.target === event.currentTarget && closeSettings()}
  >
    <dialog bind:this={settingsDialog} open class="modal" aria-labelledby="settings-title" on:keydown={trapSettingsFocus}>
      <div class="modal-heading">
        <div><span>Settings</span><h1 id="settings-title">Local by design</h1></div>
        <button bind:this={settingsCloseButton} type="button" aria-label="Close settings" on:click={closeSettings}>×</button>
      </div>
      <nav class="settings-tabs" aria-label="Settings pages">
        <button type="button" class:active={settingsPage === 'general'} on:click={() => (settingsPage = 'general')}>General</button>
        <button type="button" class:active={settingsPage === 'workspace'} on:click={() => (settingsPage = 'workspace')}>Workspace</button>
        <button type="button" class:active={settingsPage === 'components'} on:click={() => (settingsPage = 'components')}>Components</button>
        <button type="button" class:active={settingsPage === 'diagnostics'} on:click={() => (settingsPage = 'diagnostics')}>Diagnostics</button>
        <button type="button" class:active={settingsPage === 'privacy'} on:click={() => (settingsPage = 'privacy')}>Local AI Privacy</button>
      </nav>
      {#if settingsPage === 'general'}
        <div class="setting-row">
          <span class="setting-icon">⌂</span>
          <div><strong>On-device processing</strong><p>Images and edits never leave this computer.</p></div>
          <em>Always on</em>
        </div>
        <div class="setting-row">
          <span class="setting-icon">⌁</span>
          <div><strong>Interactive preview</strong><p>Uses a copy capped at 1600 pixels. Exports use full resolution.</p></div>
          <em>Balanced</em>
        </div>
        <div class="setting-row">
          <span class="setting-icon">◎</span>
          <div><strong>Analytics and telemetry</strong><p>PhotoForge includes no analytics, crash reporting, or remote logs.</p></div>
          <em>Off</em>
        </div>
        <div class="guided-settings" aria-labelledby="guided-settings-title">
          <h2 id="guided-settings-title">Guided Edit preferences</h2>
          <label>
            <span><strong>Show warnings</strong><small>Display conservative limitations in each proposed plan.</small></span>
            <input
              type="checkbox"
              checked={guidedSettings.showWarnings}
              on:change={(event) => updateGuidedSetting('showWarnings', event.currentTarget.checked)}
            />
          </label>
          <label>
            <span><strong>Show confidence</strong><small>Display heuristic rule-match strength.</small></span>
            <input
              type="checkbox"
              checked={guidedSettings.showConfidence}
              on:change={(event) => updateGuidedSetting('showConfidence', event.currentTarget.checked)}
            />
          </label>
          <label>
            <span><strong>Automatically open plan inspector</strong><small>Open operation editing immediately after planning.</small></span>
            <input
              type="checkbox"
              checked={guidedSettings.autoOpenPlanInspector}
              on:change={(event) => updateGuidedSetting('autoOpenPlanInspector', event.currentTarget.checked)}
            />
          </label>
          <label>
            <span><strong>Remember prompt history</strong><small>Keep up to 25 requests in local browser storage.</small></span>
            <input
              type="checkbox"
              checked={guidedSettings.rememberPromptHistory}
              on:change={(event) => updateGuidedSetting('rememberPromptHistory', event.currentTarget.checked)}
            />
          </label>
        </div>
        <p class="modal-footnote">The original file is never modified by default. Export always asks for a new location.</p>
      {:else if settingsPage === 'workspace'}
        <WorkspaceSettings />
      {:else if settingsPage === 'components'}
        <ComponentsSettings />
      {:else if settingsPage === 'diagnostics'}
        <DiagnosticsSettings />
      {:else}
        <LocalAiPrivacy />
      {/if}
    </dialog>
  </div>
{/if}
