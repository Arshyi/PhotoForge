import type { SourceOrigin } from '../source/types';
import type { LayerWorkflowStep } from '../layers/workflow';

export interface CurvePoint { input: number; output: number }
export interface CurveSet {
  rgb: CurvePoint[];
  red: CurvePoint[];
  green: CurvePoint[];
  blue: CurvePoint[];
}
export interface HslAdjustment { hue: number; saturation: number; lightness: number }
export interface HslSettings {
  master: HslAdjustment;
  red: HslAdjustment;
  yellow: HslAdjustment;
  green: HslAdjustment;
  cyan: HslAdjustment;
  blue: HslAdjustment;
  magenta: HslAdjustment;
}
export type CropOverlay = 'none' | 'rule_of_thirds' | 'golden_ratio';
export interface PerspectiveCorners {
  topLeft: [number, number];
  topRight: [number, number];
  bottomRight: [number, number];
  bottomLeft: [number, number];
}
export interface SelectiveColorAdjustment { cyan: number; magenta: number; yellow: number; black: number }
export type WhiteBalance =
  | { mode: 'asShot'; multipliers: [number, number, number] }
  | { mode: 'auto' }
  | { mode: 'temperatureTint'; temperature: number; tint: number }
  | { mode: 'custom'; multipliers: [number, number, number] };
export interface DevelopmentParameters {
  whiteBalance: WhiteBalance;
  exposureEv: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
}
import type { MaskSnapshot } from '../selections/types';

export type BaseEditOperation =
  | { type: 'brightness'; amount: number }
  | { type: 'contrast'; amount: number }
  | { type: 'saturation'; amount: number }
  | { type: 'gamma'; value: number }
  | { type: 'grayscale' }
  | { type: 'sepia' }
  | { type: 'reflect_horizontal' }
  | { type: 'rotate'; degrees: number }
  | { type: 'gaussian_blur'; radius: number }
  | { type: 'sharpen'; strength: number }
  | { type: 'auto_white_balance'; strength: number }
  | { type: 'local_contrast'; strength: number; tile_size: number; clip_limit: number }
  | { type: 'denoise'; strength: number; preserve_edges: number; color: number }
  | { type: 'remove_defects'; strength: number; threshold: number }
  | { type: 'deconvolve'; kernel: BlurKernel; iterations: number; damping: number }
  | { type: 'deblock'; strength: number }
  | { type: 'edge_aware_sharpen'; strength: number; radius: number; threshold: number }
  | { type: 'mild_deblur'; strength: number; radius: number }
  | { type: 'document_enhance'; strength: number; grayscale: boolean }
  | { type: 'uneven_lighting_correction'; strength: number; radius: number }
  | { type: 'curves'; curves: CurveSet }
  | { type: 'levels'; input_black: number; input_white: number; gamma: number; output_black: number; output_white: number }
  | { type: 'white_point'; red: number; green: number; blue: number }
  | { type: 'black_point'; red: number; green: number; blue: number }
  | { type: 'crop'; x: number; y: number; width: number; height: number; aspect_ratio: string | null; overlay: CropOverlay }
  | { type: 'straighten'; degrees: number }
  | { type: 'perspective'; corners: PerspectiveCorners }
  | { type: 'lens_correction'; distortion: number; vignetting: number; chromatic_aberration: number }
  | { type: 'decontaminate_colors'; enabled: boolean; strength: number; radius: number }
  | { type: 'hsl'; settings: HslSettings }
  | { type: 'temperature_tint'; temperature: number; tint: number }
  | { type: 'raw_development'; parameters: DevelopmentParameters }
  | { type: 'selective_color'; target_hue: number; width: number; adjustment: SelectiveColorAdjustment };

/// The point-spread function a deconvolution inverts. Tagged exactly as the
/// Rust `BlurKernel` serialises, so a workflow written here round-trips.
export type BlurKernel =
  | { type: 'defocus'; radius: number }
  | { type: 'motion'; angleDegrees: number; distance: number }
  | { type: 'gaussian'; sigma: number };

export type EditOperation =
  | BaseEditOperation
  | {
      type: 'masked';
      operation: BaseEditOperation;
      mask: MaskSnapshot;
      invert: boolean;
      mask_id: string | null;
    };

export type OperationType = BaseEditOperation['type'];

export interface ImageMetadata {
  filename: string;
  width: number;
  height: number;
  format: string;
  fileSize: number;
  colorSpace: string;
  bitDepth: number;
  hasAlpha: boolean;
  createdAt: string | null;
  modifiedAt: string | null;
  cameraModel: string | null;
  exifAvailable: boolean;
  /** Optional decoder-supplied camera metadata; absent for legacy raster files. */
  raw?: RawCaptureMetadata | null;
  /**
   * Present when what was opened is a region or reduced copy of a larger file.
   * `width` and `height` above are then the document's; the file's are in here.
   */
  origin?: SourceOrigin | null;
}

export interface RawCaptureMetadata {
  manufacturer: string | null;
  model: string | null;
  lens: string | null;
  focalLengthMm: number | null;
  aperture: number | null;
  shutterSpeedSeconds: number | null;
  iso: number | null;
  captureTime: string | null;
  orientation: number | null;
  exposureCompensation: number | null;
  whiteBalanceMultipliers: [number, number, number] | null;
}

export type RawFormat =
  | 'DNG' | 'CR2' | 'CR3' | 'NEF' | 'ARW' | 'RAF' | 'ORF' | 'RW2'
  | 'PEF' | 'SRW' | 'MRW' | 'ERF' | 'MEF' | 'IIQ' | '3FR' | 'MOS' | 'ARI';

export type RawSupport = 'decodable' | 'recognizedDecoderUnavailable' | 'unsupported';

/** Mirrors `raw::demosaic::Quality`. */
export type DemosaicAlgorithm = 'bilinear' | 'malvar-he-cutler';

/** Mirrors `raw::RawSourceMode`; the tag is explicit, never inferred. */
export type RawSourceMode = { mode: 'linked'; path: string } | { mode: 'embedded' };

/** Mirrors `raw::RawSourceStatus`. */
export type RawSourceStatus =
  | 'available'
  | 'missing'
  | { changed: { foundSha256: string } };

/** Mirrors `raw::RawSourceReference`. */
export interface RawSourceReference {
  filename: string;
  format: RawFormat;
  fileSize: number;
  sha256: string;
  width: number;
  height: number;
}

/** Mirrors `color::WhiteBalance`. */
export type RawWhiteBalance =
  | { mode: 'asShot'; multipliers: [number, number, number] }
  | { mode: 'auto' }
  | { mode: 'temperatureTint'; temperature: number; tint: number }
  | { mode: 'custom'; multipliers: [number, number, number] };

/** Mirrors `color::DevelopmentParameters`. */
export interface RawDevelopmentParameters {
  whiteBalance: RawWhiteBalance;
  exposureEv: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
}

/**
 * Mirrors `raw::RawLayerSource`.
 *
 * This, not the developed raster, is what defines a RAW layer: the raster is a
 * cache of one development and can be produced again from these fields.
 */
export interface RawLayerSource {
  reference: RawSourceReference;
  mode: RawSourceMode;
  parameters: RawDevelopmentParameters;
  decoder: string;
  decoderVersion: string;
  capture: RawCaptureMetadata;
}

/** Mirrors `commands::raw::RawDevelopResult`. */
export interface RawDevelopResult {
  pixelId: string;
  width: number;
  height: number;
  /** The sensor's own size, which a preview does not match. */
  sourceWidth: number;
  sourceHeight: number;
  source: RawLayerSource;
  metadata: RawCaptureMetadata;
  multipliers: [number, number, number];
  colorManaged: boolean;
  demosaic: DemosaicAlgorithm;
  cfaPattern: string;
  bitsPerSample: number;
  blackLevel: [number, number, number, number];
  whiteLevel: number;
  processingTimeMs: number;
}

export interface RawSourceStatusResult {
  status: RawSourceStatus;
  expectedSha256: string;
}

export interface RawInspection {
  filename: string;
  format: RawFormat | null;
  support: RawSupport;
  fileSize: number;
  sha256: string;
  tiffHeader: boolean;
  dimensions: [number, number] | null;
  decoder: string | null;
}

export interface RawDecoderCapabilities {
  backend: string;
  backendAvailable: boolean;
  nativeDependencies: boolean;
  license: string;
  formats: RawFormat[];
  demosaicAlgorithms: string[];
}

export interface RawInspectionResult {
  inspection: RawInspection;
  capabilities: RawDecoderCapabilities;
}

/**
 * Mirrors `commands::editor::OpenRawImageResult`.
 *
 * The first seven fields match `OpenImageResult` exactly, so a developed RAW
 * flows through the same document pipeline as any other photograph.
 */
export interface OpenRawImageResult {
  metadata: ImageMetadata;
  originalPreviewDataUrl: string;
  previewDataUrl: string;
  processingTimeMs: number;
  documentId: number;
  isCurrent: boolean;
  backgroundPixelId: string;
  /** Absent on a stale open. */
  source: RawLayerSource | null;
  multipliers: [number, number, number];
  colorManaged: boolean;
  cfaPattern: string;
  whiteLevel: number;
}

export interface OpenImageResult {
  metadata: ImageMetadata;
  originalPreviewDataUrl: string;
  previewDataUrl: string;
  processingTimeMs: number;
  documentId: number;
  isCurrent: boolean;
  /**
   * Identifier of the pixel buffer the opened image was registered under. The
   * frontend builds the initial single-layer document around it.
   */
  backgroundPixelId: string;
}

export interface PreviewResult {
  previewDataUrl: string;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
  operationCount: number;
}

export interface ExportResult {
  outputPath: string;
  width: number;
  height: number;
  processingTimeMs: number;
}

export interface ColorCastEstimate {
  dominant: 'neutral' | 'warm' | 'cool' | 'green' | 'mixed';
  redBias: number;
  greenBias: number;
  blueBias: number;
}

export interface ImageQualityAnalysis {
  averageLuminance: number;
  luminanceSpread: number;
  estimatedColorCast: ColorCastEstimate;
  estimatedNoise: number;
  estimatedSharpness: number;
  estimatedLocalContrast: number;
  edgeDensity: number;
  whiteBackgroundRatio: number;
  likelyDocument: boolean;
}

export interface AnalysisResult {
  analysis: ImageQualityAnalysis | null;
  documentId: number;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
}

export interface EditPlan {
  summary: string;
  confidence: number;
  warnings: string[];
  operations: EditOperation[];
  /** Older operation-only planners may omit this field. */
  layerSteps?: LayerWorkflowStep[];
  operationExplanations: string[];
}

export interface PlanResult {
  plan: EditPlan | null;
  documentId: number;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
}

export interface GuidedSettings {
  showWarnings: boolean;
  showConfidence: boolean;
  autoOpenPlanInspector: boolean;
  rememberPromptHistory: boolean;
}

export type GuidedPlanner = 'rule' | 'ollama';

export interface GuidedHistoryEntry {
  prompt: string;
  provider: 'Rule' | 'Ollama';
}

export interface AppErrorPayload {
  code?: string;
  message?: string;
}

export interface Preset {
  id: string;
  name: string;
  description: string;
  operations: EditOperation[];
}

export type PlannerProvider = 'rule' | 'ollama' | 'open_ai' | 'future';
export type EngineProvider = 'deterministic' | 'onnx' | 'real_esrgan' | 'future';

export interface PlannerCapabilities {
  supportsGuidedEditing: boolean;
  supportsReasoning: boolean;
  supportsSelectionPlanning?: boolean;
  requiresModel: boolean;
  offline: boolean;
}

export interface RestorationCapabilities {
  supportsRestoration: boolean;
  supportsMaskedAdjustments?: boolean;
  supportsNeuralModels: boolean;
  requiresModel: boolean;
  offline: boolean;
  preservesAlpha: boolean;
  maxInputMegapixels: number;
}

export interface PlannerRegistration {
  id: PlannerProvider;
  name: string;
  version: string;
  provider: string;
  memoryEstimateMb: number;
  installed: boolean;
  loaded: boolean;
  active: boolean;
  unavailableReason: string | null;
  capabilities: PlannerCapabilities;
}

export interface EngineRegistration {
  id: EngineProvider;
  name: string;
  version: string;
  provider: string;
  memoryEstimateMb: number;
  installed: boolean;
  loaded: boolean;
  active: boolean;
  unavailableReason: string | null;
  capabilities: RestorationCapabilities;
}

export interface ComponentConfiguration {
  activePlanner: PlannerProvider;
  activeEngine: EngineProvider;
  plannerEndpoint: string;
  initializationTimeoutMs: number;
  ollamaTimeoutMs: number;
  ollamaMaxResponseBytes: number;
  ollamaSelectedModel: string | null;
  ollamaMaxOperations: number;
  modelDirectories: string[];
  pluginDirectory: string;
}

export interface OllamaModel {
  name: string;
  sizeBytes: number;
  modifiedAt: string;
  capabilities: string[];
}

export interface OllamaModelDiscoveryResult {
  models: OllamaModel[];
  message: string;
  responseTimeMs: number;
}

export interface OllamaConnectionResult {
  connected: boolean;
  message: string;
  version: string;
  responseTimeMs: number;
}

export interface PlanValidationReport {
  valid: boolean;
  originalResponse: string;
  validatedResponse: string | null;
  rejectedFields: string[];
  errors: string[];
  validationTimeMs: number;
}

export interface OllamaPlanResult {
  plan: EditPlan | null;
  documentId: number;
  requestId: number;
  model: string;
  generationTimeMs: number;
  validationTimeMs: number;
  totalTimeMs: number;
  isCurrent: boolean;
  error: string | null;
  validationReport: PlanValidationReport;
}

export interface PlannerComparisonEntry {
  provider: 'Rule' | 'Ollama';
  plan: EditPlan | null;
  executionTimeMs: number;
  error: string | null;
}

export interface PlannerComparisonResult {
  rule: PlannerComparisonEntry;
  ollama: PlannerComparisonEntry;
  validationReport: PlanValidationReport | null;
  totalTimeMs: number;
}

export interface OllamaDiagnostics {
  connected: boolean;
  lastError: string | null;
  lastResponseTimeMs: number | null;
  connectionLatencyMs: number | null;
  generationLatencyMs: number | null;
  validationLatencyMs: number | null;
  rulePlannerLatencyMs: number | null;
  comparisonLatencyMs: number | null;
  modelSelected: string | null;
  plannerVersion: string;
  validationFailures: number;
  rejectedPlans: number;
  successfulPlans: number;
  cancelledPlans: number;
  localClientMemoryEstimateMb: number;
  memoryNote: string;
}

export interface ComponentSnapshot {
  applicationVersion: string;
  planners: PlannerRegistration[];
  engines: EngineRegistration[];
  configuration: ComponentConfiguration;
}

export interface ComponentDiagnostics {
  applicationVersion: string;
  registeredPlanners: string[];
  registeredEngines: string[];
  loadedComponents: string[];
  unavailableComponents: string[];
  initializationFailures: string[];
  pluginValidationErrors: string[];
  configurationPath: string;
}

export interface ComponentPerformanceMetrics {
  samples: number;
  registryLookupAverageNs: number;
  plannerDispatchAverageNs: number;
  componentFactoryAverageNs: number;
  note: string;
}

export interface ComponentActionResult {
  success: boolean;
  message: string;
}

export interface ModelMetadata {
  name: string;
  path: string;
  format: string;
  fileSizeBytes: number;
  memoryEstimateMb: number;
  supportedTasks: string[];
  expectedInput: string;
  expectedInputSize: number[] | null;
  expectedOutput: string;
  compatible: boolean;
  unavailableReason: string;
}

export interface ModelDiscoveryResult {
  searchedDirectories: string[];
  models: ModelMetadata[];
  message: string;
  processingTimeMs: number;
}

export interface PluginManifest {
  schemaVersion: number;
  name: string;
  version: string;
  type: 'planner' | 'restoration_engine';
  provider: string;
  entry: string;
  memoryEstimateMb: number;
  capabilities: string[];
}

export interface PluginManifestRecord {
  manifestPath: string;
  valid: boolean;
  manifest: PluginManifest | null;
  error: string | null;
  executionAllowed: boolean;
}

export interface PluginScanResult {
  directory: string;
  records: PluginManifestRecord[];
  message: string;
}

export interface HistogramChannels {
  red: number[];
  green: number[];
  blue: number[];
  luminance: number[];
  shadowClipping: number;
  highlightClipping: number;
  pixelCount: number;
}

export interface HistogramResult {
  before: HistogramChannels;
  after: HistogramChannels;
  documentId: number;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
}

export interface PixelInspection {
  x: number;
  y: number;
  red: number;
  green: number;
  blue: number;
  alpha: number;
  hue: number;
  saturation: number;
  value: number;
}

export interface Workflow {
  id: string;
  name: string;
  description: string;
  folder: string;
  favorite: boolean;
  operations: EditOperation[];
  layerSteps: LayerWorkflowStep[];
  createdAt: string;
  updatedAt: string;
}

export interface WorkflowDocument { schemaVersion: 1 | 2; workflow: Workflow }
export type ExportProfile = 'web' | 'print' | 'archive' | 'lossless' | 'high_jpeg' | 'maximum_compression';
export interface ColorExportOptions {
  colorSpace: 'srgb' | 'display_p3' | 'adobe_rgb';
  bitDepth: 8 | 16;
  dither: boolean;
}

export interface RenderCacheStats {
  hits: number;
  misses: number;
  evictions: number;
  refusals: number;
  entries: number;
  bytes: number;
  capacityBytes: number;
  diskEntries: number;
  diskBytes: number;
  diskCapacityBytes: number;
}

export interface RenderGpuInfo {
  backend: string;
  adapter: string;
  deviceType: string;
  driver: string;
  maxBufferBytes: number;
  healthy: boolean;
}

export interface RenderGpuStats {
  dispatches: number;
  megapixels: number;
  declined: number;
  failures: number;
}

export interface RenderDiagnostics {
  tileSize: number;
  maxThreads: number;
  cache: RenderCacheStats;
  gpuCompiled: boolean;
  gpuAvailable: boolean;
  gpu: RenderGpuInfo | null;
  gpuStats: RenderGpuStats | null;
  gpuNote: string;
}

export type RenderBackendMode = 'auto' | 'cpu' | 'gpu';
export interface RenderBackendPolicy {
  mode: RenderBackendMode;
  active: boolean;
  effectiveBackend: 'cpu' | 'gpu';
  adapterStatus: 'not_probed' | 'available' | 'unavailable' | 'unhealthy';
  hardDisabled: boolean;
  fallbackReason?: string | null;
}
export interface BatchOptions {
  inputFolder: string;
  outputFolder: string;
  filenameTemplate: string;
  recursive: boolean;
  overwrite: boolean;
  workers: number;
  exportProfile: ExportProfile;
  color?: ColorExportOptions;
  dryRun: boolean;
}
export type BatchState = 'idle' | 'discovering' | 'running' | 'cancelling' | 'completed' | 'cancelled' | 'failed';
export interface BatchFailureRecord { inputPath: string; error: string }
export interface BatchStatus {
  batchId: number;
  state: BatchState;
  discovered: number;
  completed: number;
  skipped: number;
  failed: number;
  currentFile: string | null;
  estimatedRemainingMs: number | null;
  elapsedMs: number;
  failures: BatchFailureRecord[];
  logPath: string | null;
}
export interface BatchPreview { discovered: number; sampleOutputs: string[]; estimatedTimeMs: number; skippedExisting: number }
export interface WorkspaceLayout {
  schemaVersion: 1;
  name: string;
  leftPanelWidth: number;
  rightPanelWidth: number;
  collapsedSections: string[];
  activePanel: string;
  highContrast: boolean;
  uiScale: number;
}
export interface ShortcutBinding { action: string; keys: string }
export type ComparisonMode = 'swipe' | 'split' | 'blink' | 'difference';

/// What the Model Manager shows. Mirrors `inference::InferenceStatus`.
///
/// `runtimeCompiled` and an empty `installed` are different states with
/// different fixes, which is why both are reported rather than collapsed into
/// one "unavailable".
export interface InferenceModel {
  id: string;
  name: string;
  version: string;
  architecture: string;
  capability: string;
  format: string;
  colorSpace: string;
  normalization: string;
  inputChannels: number;
  outputChannels: number;
  scale: number;
  tileSize: number;
  tileOverlap: number;
  fileBytes: number;
  sha256: string;
  license: string;
  source: string;
}

export interface InferenceCapability {
  capability: string;
  available: boolean;
  models: string[];
  reason: string;
}

export interface InferenceStatus {
  runtimeCompiled: boolean;
  runtime: string;
  modelDirectory: string;
  installed: InferenceModel[];
  capabilities: InferenceCapability[];
  classicalOnly: boolean;
}
