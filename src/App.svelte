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
  import RefineSelectionDialog, {
    REFINE_SELECTION_DEFAULTS,
    type RefineSelectionParameters
  } from './lib/components/RefineSelectionDialog.svelte';
  import SelectionWorkspace from './lib/components/SelectionWorkspace.svelte';
  import RestorationPanel from './lib/components/RestorationPanel.svelte';
  import SliderControl from './lib/components/SliderControl.svelte';
  import StatusBar from './lib/components/StatusBar.svelte';
  import ToolButton from './lib/components/ToolButton.svelte';
  import WorkspaceSettings from './lib/components/WorkspaceSettings.svelte';
  import { EditHistory } from './lib/stores/history';
  import {
    retainedHistorySuffix,
    selectionPanelHistoryAvailability,
    type HistoryEvent
  } from './lib/stores/historyTimeline';
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
    getMaskProgress,
    importMaskFile,
    importMaskPng,
    inspectSelectionMask,
    magicWandSelection,
    rasterizeSelection,
    remapSelectionMasks,
    refineSelection,
    transformSelection,
    validateMaskSnapshot
  } from './lib/selections/commands';
  import { MaskProgressTracker, type MaskProgressView } from './lib/selections/progress';
  import {
    extractGeometryOperations,
    geometryFingerprint,
    geometryOperationsToEditOperations
  } from './lib/selections/geometry';
  import {
    applyGeometryRemap,
    planGeometryRemap,
    validateGeometryRemapResult
  } from './lib/selections/geometryTransactions';
  import {
    createNamedMask,
    createSelectionState,
    deleteNamedMask,
    duplicateNamedMask,
    loadNamedMask,
    MAX_NAMED_MASKS,
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
    legacyDocumentSelectionKey,
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
  import {
    createGeometryCommitToken,
    createWorkspaceMutationGuard,
    isGeometryCommitTokenCurrent,
    isWorkspaceMutationGuardCurrent,
    selectionCanvasRectangle,
    workspaceMutationBlocked,
    type GeometryCommitToken,
    type WorkspaceMutationGuard
  } from './lib/selections/workflowGuards';

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
  const maskProgressTracker = new MaskProgressTracker();
  let maskProgress: MaskProgressView | null = null;
  let maskProgressTimer: ReturnType<typeof setTimeout> | undefined;
  let historyEvents: HistoryEvent[] = [];
  let redoEvents: HistoryEvent[] = [];
  let selectionPersistenceWarningShown = false;
  let refineOriginalMask: SelectionState['activeMask'] = null;
  let refinePreviewMask: SelectionState['activeMask'] = null;
  let refinePreviewDiagnostics: SelectionState['activeDiagnostics'] = null;
  let refineParameters: RefineSelectionParameters = { ...REFINE_SELECTION_DEFAULTS };
  let refineBusy = false;
  let refineError = '';
  let refineTimer: ReturnType<typeof setTimeout> | undefined;
  let refineSourceGuard: WorkspaceMutationGuard | null = null;
  let geometryTransactionRunning = false;
  let geometryCommitGeneration = 0;
  type PendingGeometryCommit = {
    operations: EditOperation[];
    coalesceKey?: string;
    sourceWidth: number;
    sourceHeight: number;
    token: GeometryCommitToken;
  };
  let pendingGeometryCommit: PendingGeometryCommit | null = null;
  let geometryCommitTimer: ReturnType<typeof setTimeout> | undefined;

  $: comparisonUsesSplitView = comparison && (comparisonMode === 'split' || valueFor(operations, 'rotate', 0) % 360 !== 0);
  $: selectionCanvasWidth = selectionState.canvasWidth || metadata?.width || 0;
  $: selectionCanvasHeight = selectionState.canvasHeight || metadata?.height || 0;
  $: selectionPanelHistory = selectionPanelHistoryAvailability(historyEvents, redoEvents);

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
      if (refineOriginalMask) return;
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
          c: 'color_range',
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
      if (maskProgressTimer) clearTimeout(maskProgressTimer);
      if (refineTimer) clearTimeout(refineTimer);
      invalidateGeometryCommits();
      maskProgressTracker.reset();
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
    closeRefineState();
    invalidateGeometryCommits();
    const previousMaskRequest = maskRequestId;
    if (selectionBusy) void cancelMaskOperation(previousMaskRequest).catch(() => false);
    maskRequestId += 1;
    selectionBusy = false;
    stopMaskProgress();
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
        path,
        result.metadata.width,
        result.metadata.height
      );
      const legacySelectionKey = legacyDocumentSelectionKey(
        result.metadata.filename,
        result.metadata.width,
        result.metadata.height
      );
      const restoredSelection = loadSelectionSession(
        selectionKey,
        result.metadata.width,
        result.metadata.height,
        localStorage,
        legacySelectionKey
      );
      operations = history.replace(
        geometryOperationsToEditOperations(restoredSelection.geometryOperations)
      );
      selectionState = selectionHistory.replace(restoredSelection);
      if (!selectionState.activeMask) selectionState = { ...selectionState, applyScope: 'global' };
      historyEvents = [];
      redoEvents = [];
      selectionPersistenceWarningShown = false;
      syncHistoryActions();
      previewCurrent = true;
      notify(`${result.metadata.filename} opened locally`);
      if (operations.length) schedulePreview();
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
    commitPrepared(scoped, coalesceKey);
  }

  function commitGlobal(next: EditOperation[], coalesceKey?: string) {
    commitPrepared(cloneOperations(next), coalesceKey);
  }

  function commitPrepared(next: EditOperation[], coalesceKey?: string) {
    if (!allowWorkspaceMutation()) return;
    let geometryChanged = false;
    try {
      geometryChanged = geometryFingerprint(extractGeometryOperations(operations)) !==
        geometryFingerprint(extractGeometryOperations(next));
    } catch (error) {
      notify(errorMessage(error), 'error');
      return;
    }
    if (geometryChanged) {
      queueGeometryCommit(next, coalesceKey);
      return;
    }
    selectionHistory.endCoalescing();
    const before = JSON.stringify(operations);
    operations = history.commit(next, coalesceKey);
    if (JSON.stringify(operations) !== before) {
      recordHistoryMutation('edit', history.lastCommitCreatedEntry);
    }
    syncHistoryActions();
    schedulePreview();
  }

  function queueGeometryCommit(next: EditOperation[], coalesceKey?: string) {
    if (!metadata) return;
    pendingGeometryCommit = {
      operations: cloneOperations(next),
      ...(coalesceKey ? { coalesceKey } : {}),
      sourceWidth: metadata.width,
      sourceHeight: metadata.height,
      token: createGeometryCommitToken(
        documentId,
        activeOpenRequest,
        geometryCommitGeneration,
        selectionState.documentKey
      )
    };
    if (geometryTransactionRunning) return;
    if (geometryCommitTimer) clearTimeout(geometryCommitTimer);
    geometryCommitTimer = setTimeout(() => {
      geometryCommitTimer = undefined;
      void drainGeometryCommit();
    }, coalesceKey === 'straighten' ? 140 : 0);
  }

  async function drainGeometryCommit() {
    if (geometryTransactionRunning || !pendingGeometryCommit || !metadata) return;
    const pending = pendingGeometryCommit;
    pendingGeometryCommit = null;
    if (!geometryCommitIsCurrent(pending.token)) return;
    geometryTransactionRunning = true;
    selectionBusy = true;
    const oldOperations = cloneOperations(operations);
    const selectionBefore = structuredClone(selectionState);
    const ownDocument = documentId;
    const ownRequest = ++maskRequestId;
    let progressStarted = false;
    let progressDelay: ReturnType<typeof setTimeout> | undefined;
    try {
      const plan = planGeometryRemap(
        pending.sourceWidth,
        pending.sourceHeight,
        oldOperations,
        pending.operations,
        selectionBefore
      );
      for (const embedded of plan.newEmbeddedMasks) {
        const validated = await validateMaskSnapshot(embedded.mask);
        if (
          validated.checksum !== embedded.mask.checksum ||
          validated.width !== embedded.width ||
          validated.height !== embedded.height
        ) {
          throw new Error(`Masked operation ${embedded.operationIndex + 1} failed geometry validation.`);
        }
      }

      progressDelay = setTimeout(() => {
        progressDelay = undefined;
        if (ownRequest !== maskRequestId || !geometryCommitIsCurrent(pending.token)) return;
        startMaskProgress(ownRequest, 'Validate and remap mask geometry');
        progressStarted = true;
      }, 180);
      const result = await remapSelectionMasks({
        oldGeometry: plan.oldGeometry,
        newGeometry: plan.newGeometry,
        items: plan.items,
        documentId: ownDocument,
        requestId: ownRequest
      });
      if (progressDelay) {
        clearTimeout(progressDelay);
        progressDelay = undefined;
      }
      const remapped = validateGeometryRemapResult(plan, result, ownDocument, ownRequest);
      if (result.documentId !== documentId || result.requestId !== maskRequestId ||
        !geometryCommitIsCurrent(pending.token)) {
        throw new Error('The mask geometry result became stale or incomplete before it could be committed.');
      }
      processingTime = result.processingTimeMs;
      if (
        ownDocument !== documentId ||
        !geometryCommitIsCurrent(pending.token) ||
        JSON.stringify(operations) !== JSON.stringify(oldOperations) ||
        JSON.stringify(selectionState) !== JSON.stringify(selectionBefore)
      ) {
        throw new Error('The document changed while masks were being remapped; no changes were committed.');
      }
      const applied = applyGeometryRemap(plan, pending.operations, selectionBefore, remapped);
      if (historyEvents.at(-1) !== 'geometry') {
        history.endCoalescing();
        selectionHistory.endCoalescing();
      }
      operations = history.commit(applied.operations, pending.coalesceKey);
      selectionState = selectionHistory.commit(applied.selection, pending.coalesceKey);
      const editPushed = history.lastCommitCreatedEntry;
      const selectionPushed = selectionHistory.lastCommitCreatedEntry;
      let historyReset = false;
      if (editPushed !== selectionPushed) {
        resetHistoryAtCurrentState();
        historyReset = true;
      } else {
        recordHistoryMutation('geometry', editPushed);
      }
      persistSelectionState();
      syncHistoryActions();
      schedulePreview();
      notify(
        historyReset
          ? 'Geometry applied, but history was reset to preserve atomic undo safety.'
          : 'Image geometry and masks updated together',
        historyReset ? 'error' : 'success'
      );
    } catch (error) {
      if (geometryCommitIsCurrent(pending.token) && !isMaskCancellation(error)) {
        notify(errorMessage(error), 'error');
      }
    } finally {
      if (progressDelay) clearTimeout(progressDelay);
      if (progressStarted && ownRequest === maskRequestId && geometryCommitIsCurrent(pending.token)) {
        finishMaskProgress(ownRequest);
      }
      if (ownRequest === maskRequestId) selectionBusy = false;
      geometryTransactionRunning = false;
      const queuedAfterTransaction = pendingGeometryCommit as PendingGeometryCommit | null;
      if (queuedAfterTransaction && geometryCommitIsCurrent(queuedAfterTransaction.token)) {
        if (geometryCommitTimer) clearTimeout(geometryCommitTimer);
        geometryCommitTimer = setTimeout(() => {
          geometryCommitTimer = undefined;
          void drainGeometryCommit();
        }, 0);
      } else if (queuedAfterTransaction) {
        pendingGeometryCommit = null;
      }
    }
  }

  function invalidateGeometryCommits() {
    geometryCommitGeneration += 1;
    pendingGeometryCommit = null;
    if (geometryCommitTimer) clearTimeout(geometryCommitTimer);
    geometryCommitTimer = undefined;
  }

  function geometryCommitIsCurrent(token: GeometryCommitToken): boolean {
    return isGeometryCommitTokenCurrent(
      token,
      documentId,
      activeOpenRequest,
      geometryCommitGeneration,
      selectionState.documentKey
    );
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

  function commitSelectionState(
    next: SelectionState,
    coalesceKey?: string,
    allowBusyResult = false
  ) {
    if (!allowBusyResult && !allowWorkspaceMutation()) return;
    history.endCoalescing();
    const before = JSON.stringify(selectionState);
    selectionState = selectionHistory.commit(next, coalesceKey);
    if (JSON.stringify(selectionState) !== before) {
      recordHistoryMutation('selection', selectionHistory.lastCommitCreatedEntry);
    }
    persistSelectionState();
    syncHistoryActions();
  }

  function recordHistoryMutation(kind: HistoryEvent, createdEntry: boolean) {
    if (createdEntry) historyEvents = [...historyEvents, kind];
    redoEvents = [];
    history.clearRedo();
    selectionHistory.clearRedo();
    reconcileHistoryRetention();
  }

  function reconcileHistoryRetention() {
    const retained = retainedHistorySuffix(
      historyEvents,
      history.undoDepth,
      selectionHistory.undoDepth
    );
    historyEvents = retained.events;
    history.retainUndoDepth(retained.editDepth);
    selectionHistory.retainUndoDepth(retained.selectionDepth);
  }

  function resetHistoryAtCurrentState() {
    operations = history.replace(operations);
    selectionState = selectionHistory.replace(selectionState);
    historyEvents = [];
    redoEvents = [];
  }

  function allowWorkspaceMutation(): boolean {
    if (!workspaceMutationBlocked(selectionBusy, geometryTransactionRunning, refineOriginalMask !== null)) {
      return true;
    }
    notify('Wait for the current selection or geometry operation to finish.', 'error');
    return false;
  }

  function syncHistoryActions() {
    canUndo = historyEvents.length > 0;
    canRedo = redoEvents.length > 0;
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
    if (!allowWorkspaceMutation()) return;
    const kind = historyEvents.at(-1);
    if (!kind) return;
    if ((kind === 'geometry' && (!history.canUndo || !selectionHistory.canUndo)) ||
      (kind === 'edit' && !history.canUndo) ||
      (kind === 'selection' && !selectionHistory.canUndo)) {
      resetHistoryAtCurrentState();
      syncHistoryActions();
      notify('History was reset because its paired snapshots were unavailable.', 'error');
      return;
    }
    historyEvents = historyEvents.slice(0, -1);
    history.endCoalescing();
    selectionHistory.endCoalescing();
    if (kind === 'geometry') {
      operations = history.undo();
      selectionState = selectionHistory.undo();
      persistSelectionState();
      schedulePreview();
    } else if (kind === 'edit') {
      operations = history.undo();
      schedulePreview();
    } else {
      selectionState = selectionHistory.undo();
      persistSelectionState();
    }
    redoEvents = [...redoEvents, kind];
    syncHistoryActions();
  }

  function redo() {
    if (!allowWorkspaceMutation()) return;
    const kind = redoEvents.at(-1);
    if (!kind) return;
    if ((kind === 'geometry' && (!history.canRedo || !selectionHistory.canRedo)) ||
      (kind === 'edit' && !history.canRedo) ||
      (kind === 'selection' && !selectionHistory.canRedo)) {
      resetHistoryAtCurrentState();
      syncHistoryActions();
      notify('Redo history was reset because its paired snapshots were unavailable.', 'error');
      return;
    }
    redoEvents = redoEvents.slice(0, -1);
    history.endCoalescing();
    selectionHistory.endCoalescing();
    if (kind === 'geometry') {
      operations = history.redo();
      selectionState = selectionHistory.redo();
      persistSelectionState();
      schedulePreview();
    } else if (kind === 'edit') {
      operations = history.redo();
      schedulePreview();
    } else {
      selectionState = selectionHistory.redo();
      persistSelectionState();
    }
    historyEvents = [...historyEvents, kind];
    reconcileHistoryRetention();
    syncHistoryActions();
  }

  function undoSelectionOnly() {
    if (!selectionPanelHistory.canUndo) return;
    undo();
  }

  function redoSelectionOnly() {
    if (!selectionPanelHistory.canRedo) return;
    redo();
  }

  function reset() {
    if (!allowWorkspaceMutation()) return;
    if (!metadata || operations.length === 0) return;
    commitGlobal([]);
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
    const inspectedChecksum = selectionState.activeMask.checksum;
    try {
      const diagnostics = await inspectSelectionMask(selectionState.activeMask);
      if (selectionState.activeMask?.checksum === inspectedChecksum) {
        selectionState = { ...selectionState, activeDiagnostics: diagnostics };
        persistSelectionState();
      }
    } catch (error) {
      if (selectionState.activeMask?.checksum !== inspectedChecksum) return;
      selectionState = selectionHistory.replace(
        setActiveMask(selectionState, null, null)
      );
      syncHistoryActions();
      notify(errorMessage(error), 'error');
    }
  }

  async function handleSelectionGesture(gesture: SelectionGesture) {
    if (!metadata || selectionBusy) return;
    const mutationGuard = createWorkspaceMutationGuard(documentId, operations, selectionState);
    const configuredMode = operationModeFromModifiers(
      selectionState.mode,
      gesture.shiftKey,
      gesture.altKey
    );
    const mode = gesture.tool === 'eraser' ? 'subtract' : configuredMode;
    const ownRequest = ++maskRequestId;
    selectionBusy = true;
    startMaskProgress(ownRequest, selectionToolLabel(gesture.tool));
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
          width: selectionCanvasWidth,
          height: selectionCanvasHeight,
          shape,
          mode,
          base: selectionState.activeMask,
          documentId,
          requestId: ownRequest
        });
      }
      acceptMaskResult(result, gesture.tool, mutationGuard);
    } catch (error) {
      if (ownRequest === maskRequestId && !isMaskCancellation(error)) notify(errorMessage(error), 'error');
    } finally {
      if (ownRequest === maskRequestId) {
        selectionBusy = false;
        finishMaskProgress(ownRequest);
      }
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
      if (gesture.resolvedBrushSamples?.length) {
        return {
          type: 'resolved_brush',
          samples: gesture.resolvedBrushSamples,
          hardness: selectionState.settings.brushHardness
        };
      }
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

  function openRefineSelection() {
    if (!selectionState.activeMask || selectionBusy) return;
    refineSourceGuard = createWorkspaceMutationGuard(documentId, operations, selectionState);
    refineOriginalMask = structuredClone(selectionState.activeMask);
    refinePreviewMask = null;
    refinePreviewDiagnostics = null;
    refineParameters = { ...REFINE_SELECTION_DEFAULTS };
    refineError = '';
    scheduleRefinePreview();
  }

  function updateRefineParameters(parameters: RefineSelectionParameters) {
    if (!refineOriginalMask) return;
    refineParameters = { ...parameters };
    refinePreviewMask = null;
    refinePreviewDiagnostics = null;
    refineError = '';
    scheduleRefinePreview();
  }

  function scheduleRefinePreview() {
    if (refineTimer) clearTimeout(refineTimer);
    refineTimer = setTimeout(() => {
      refineTimer = undefined;
      void renderRefinePreview();
    }, 120);
  }

  async function renderRefinePreview() {
    if (!refineOriginalMask || !refineSourceGuard || !metadata || refineBusy || selectionBusy) return;
    const original = structuredClone(refineOriginalMask);
    const sourceGuard = refineSourceGuard;
    const originalChecksum = original.checksum;
    const ownDocument = documentId;
    const parameters = { ...refineParameters };
    const ownRequest = ++maskRequestId;
    selectionBusy = true;
    refineBusy = true;
    refineError = '';
    startMaskProgress(ownRequest, 'Refine selection preview');
    try {
      const result = await refineSelection({
        mask: original,
        operation: {
          type: 'refine',
          smooth: parameters.smooth,
          feather: parameters.feather,
          contrast: parameters.contrast,
          shift_edge: parameters.shiftEdge
        },
        edgeStrength: 0.7,
        sampleMerged: selectionState.settings.sampleMerged,
        operations: cloneOperations(operations),
        documentId: ownDocument,
        requestId: ownRequest
      });
      if (
        result.isCurrent &&
        result.requestId === maskRequestId &&
        result.documentId === documentId &&
        refineOriginalMask?.checksum === originalChecksum &&
        isWorkspaceMutationGuardCurrent(sourceGuard, documentId, operations, selectionState)
      ) {
        refinePreviewMask = result.mask;
        refinePreviewDiagnostics = result.diagnostics;
      }
    } catch (error) {
      if (
        ownRequest === maskRequestId &&
        refineOriginalMask?.checksum === originalChecksum &&
        !isMaskCancellation(error)
      ) {
        refineError = errorMessage(error);
      }
    } finally {
      if (ownRequest === maskRequestId) {
        selectionBusy = false;
        refineBusy = false;
        finishMaskProgress(ownRequest);
      }
    }
  }

  function applyRefineSelection() {
    if (!refineOriginalMask || !refinePreviewMask || !refinePreviewDiagnostics ||
      !refineSourceGuard || refineBusy) return;
    if (!isWorkspaceMutationGuardCurrent(refineSourceGuard, documentId, operations, selectionState)) {
      closeRefineState();
      notify('The selection or edit pipeline changed; the stale refinement preview was discarded.', 'error');
      return;
    }
    const mask = structuredClone(refinePreviewMask);
    const diagnostics = structuredClone(refinePreviewDiagnostics);
    closeRefineState();
    commitSelectionState(
      setActiveMask(
        { ...selectionState, overlay: { ...selectionState.overlay, visible: true } },
        mask,
        diagnostics
      )
    );
    notify('Refine selection applied');
  }

  function cancelRefineSelection() {
    if (refineTimer) clearTimeout(refineTimer);
    refineTimer = undefined;
    if (refineBusy) {
      maskProgress = maskProgressTracker.markCancelling();
      void cancelMaskOperation(maskRequestId).catch(() => false);
    }
    closeRefineState();
  }

  function closeRefineState() {
    if (refineTimer) clearTimeout(refineTimer);
    refineTimer = undefined;
    refineOriginalMask = null;
    refinePreviewMask = null;
    refinePreviewDiagnostics = null;
    refineParameters = { ...REFINE_SELECTION_DEFAULTS };
    refineError = '';
    refineSourceGuard = null;
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
    const mutationGuard = createWorkspaceMutationGuard(documentId, operations, selectionState);
    const ownRequest = ++maskRequestId;
    selectionBusy = true;
    startMaskProgress(ownRequest, maskOperationLabel(operation));
    try {
      let result: MaskResult | null = null;
      if (operation.type === 'select_all') {
        result = await rasterizeSelection({
            width: selectionCanvasWidth,
            height: selectionCanvasHeight,
            shape: selectionCanvasRectangle(selectionCanvasWidth, selectionCanvasHeight),
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
      if (result) acceptMaskResult(result, operation.type, mutationGuard);
    } catch (error) {
      if (ownRequest === maskRequestId && !isMaskCancellation(error)) notify(errorMessage(error), 'error');
    } finally {
      if (ownRequest === maskRequestId) {
        selectionBusy = false;
        finishMaskProgress(ownRequest);
      }
    }
  }

  function acceptMaskResult(
    result: MaskResult,
    source: string,
    mutationGuard: WorkspaceMutationGuard
  ) {
    if (!result.isCurrent || result.requestId !== maskRequestId || result.documentId !== documentId) return;
    if (!isWorkspaceMutationGuardCurrent(mutationGuard, documentId, operations, selectionState)) {
      notify('The selection or edit pipeline changed; the stale mask result was discarded.', 'error');
      return;
    }
    processingTime = result.processingTimeMs;
    commitSelectionState(
      setActiveMask(
        { ...selectionState, overlay: { ...selectionState.overlay, visible: true } },
        result.mask,
        result.diagnostics
      ),
      undefined,
      true
    );
    notify(`${source.replaceAll('_', ' ')} selection updated`);
  }

  async function cancelCurrentMaskOperation() {
    if (!selectionBusy) return;
    maskProgress = maskProgressTracker.markCancelling();
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
    if (selectionBusy) return;
    if (action === 'create') {
      if (selectionState.namedMasks.length >= MAX_NAMED_MASKS) {
        notify(`Named masks are limited to ${MAX_NAMED_MASKS} per document.`, 'error');
        return;
      }
      commitSelectionState(createNamedMask(selectionState, value ?? ''));
      return;
    }
    if (!id) return;
    if (action === 'rename') commitSelectionState(renameNamedMask(selectionState, id, value ?? ''));
    else if (action === 'duplicate') {
      if (selectionState.namedMasks.length >= MAX_NAMED_MASKS) {
        notify(`Named masks are limited to ${MAX_NAMED_MASKS} per document.`, 'error');
      } else commitSelectionState(duplicateNamedMask(selectionState, id));
    }
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
    const mutationGuard = createWorkspaceMutationGuard(documentId, operations, selectionState);
    const sourceMode = selectionState.mode;
    const ownRequest = ++maskRequestId;
    selectionBusy = true;
    startMaskProgress(ownRequest, `Combine ${named.name}`);
    try {
      const result = await composeSelectionMasks({
        base: selectionState.activeMask,
        incoming: named.mask,
        mode: selectionState.mode,
        documentId,
        requestId: ownRequest
      });
      acceptMaskResult(result, `${sourceMode} ${named.name}`, mutationGuard);
    } catch (error) {
      if (ownRequest === maskRequestId && !isMaskCancellation(error)) notify(errorMessage(error), 'error');
    } finally {
      if (ownRequest === maskRequestId) {
        selectionBusy = false;
        finishMaskProgress(ownRequest);
      }
    }
  }

  function startMaskProgress(ownRequest: number, label: string) {
    if (maskProgressTimer) clearTimeout(maskProgressTimer);
    maskProgressTracker.begin(documentId, ownRequest, label);
    maskProgress = maskProgressTracker.view();
    scheduleMaskProgressPoll(documentId, ownRequest);
  }

  function scheduleMaskProgressPoll(ownDocument: number, ownRequest: number) {
    maskProgressTimer = setTimeout(async () => {
      maskProgressTimer = undefined;
      if (documentId !== ownDocument || maskRequestId !== ownRequest || !selectionBusy) return;
      try {
        const value = await getMaskProgress(ownDocument, ownRequest);
        if (value && documentId === ownDocument && maskRequestId === ownRequest) {
          maskProgress = maskProgressTracker.ingest(value);
        } else {
          maskProgress = maskProgressTracker.view();
        }
      } catch {
        maskProgress = maskProgressTracker.view();
      }
      if (documentId === ownDocument && maskRequestId === ownRequest && selectionBusy) {
        scheduleMaskProgressPoll(ownDocument, ownRequest);
      }
    }, 100);
  }

  function finishMaskProgress(ownRequest: number) {
    if (maskProgressTimer) clearTimeout(maskProgressTimer);
    maskProgressTimer = undefined;
    maskProgressTracker.finish(documentId, ownRequest);
    maskProgress = null;
  }

  function stopMaskProgress() {
    if (maskProgressTimer) clearTimeout(maskProgressTimer);
    maskProgressTimer = undefined;
    maskProgressTracker.reset();
    maskProgress = null;
  }

  function selectionToolLabel(tool: SelectionTool): string {
    return ({
      rectangle: 'Rectangle selection',
      ellipse: 'Ellipse selection',
      freehand: 'Freehand selection',
      polygon: 'Polygon selection',
      brush: 'Selection brush',
      eraser: 'Selection eraser',
      magic_wand: 'Magic wand',
      color_range: 'Color range',
      none: 'Selection'
    })[tool];
  }

  function maskOperationLabel(operation: MaskOperation): string {
    return ({
      select_all: 'Select all',
      deselect: 'Deselect',
      invert: 'Invert selection',
      feather: 'Feather selection',
      expand: 'Expand selection',
      contract: 'Contract selection',
      smooth: 'Smooth selection',
      fill_holes: 'Fill mask holes',
      remove_small_islands: 'Clean small islands',
      border: 'Create selection border',
      refine: 'Refine selection'
    })[operation.type];
  }

  async function importSelectionMask(format: 'json' | 'png') {
    if (!metadata || selectionBusy) return;
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
      if (selectionState.namedMasks.length >= MAX_NAMED_MASKS) {
        throw new Error(`Named masks are limited to ${MAX_NAMED_MASKS} per document.`);
      }
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
    if (!metadata || mask.width !== selectionCanvasWidth || mask.height !== selectionCanvasHeight) {
      throw new Error(`Mask dimensions must match ${selectionCanvasWidth} × ${selectionCanvasHeight}.`);
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
      <ToolButton label="Undo" icon="↶" disabled={!canUndo || selectionBusy || geometryTransactionRunning || Boolean(refineOriginalMask)} title="Undo (Ctrl+Z)" onclick={undo} />
      <ToolButton label="Redo" icon="↷" disabled={!canRedo || selectionBusy || geometryTransactionRunning || Boolean(refineOriginalMask)} title="Redo (Ctrl+Y)" onclick={redo} />
      <ToolButton label="Reset" icon="⌫" disabled={!metadata || !operations.length || selectionBusy || geometryTransactionRunning || Boolean(refineOriginalMask)} onclick={reset} />
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
      imageWidth={selectionCanvasWidth}
      imageHeight={selectionCanvasHeight}
      selectionTool={comparisonUsesSplitView ? 'none' : selectionState.tool}
      activeMask={selectionState.activeMask}
      visibleMasks={selectionState.namedMasks.filter((mask) => mask.visible).map((mask) => mask.mask)}
      overlaySettings={selectionState.overlay}
      brushDiameter={selectionState.settings.brushDiameter}
      brushOpacity={selectionState.settings.brushOpacity}
      pressureEnabled={selectionState.settings.pressureEnabled}
      pressureAffectsSize={selectionState.settings.pressureAffectsSize}
      pressureAffectsOpacity={selectionState.settings.pressureAffectsOpacity}
      pressureMinSizeFactor={selectionState.settings.pressureMinSizeFactor}
      pressureMinOpacityFactor={selectionState.settings.pressureMinOpacityFactor}
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
          progress={maskProgress}
          canUndo={selectionPanelHistory.canUndo}
          canRedo={selectionPanelHistory.canRedo}
          onstatechange={updateSelectionState}
          onoperation={applyMaskOperation}
          onrefine={openRefineSelection}
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
    dimensions={metadata ? `${selectionCanvasWidth} × ${selectionCanvasHeight} · ${metadata.format}` : 'No image loaded'}
    {zoom}
    operationCount={operations.length}
    {processingTime}
    isCurrent={previewCurrent}
  />
</div>

{#if refineOriginalMask}
  <RefineSelectionDialog
    originalMask={refineOriginalMask}
    previewMask={refinePreviewMask}
    originalImageUrl={previewUrl ?? originalUrl ?? ''}
    busy={refineBusy}
    error={refineError}
    parameters={refineParameters}
    onparameterschange={updateRefineParameters}
    onapply={applyRefineSelection}
    oncancel={cancelRefineSelection}
  />
{/if}

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
