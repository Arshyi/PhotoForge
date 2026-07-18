<script lang="ts">
  import type { ImageQualityAnalysis } from '../types/editor';
  import { analysisObservations } from '../utils/analysis';

  export let analysis: ImageQualityAnalysis | null;
  export let analyzing = false;

  $: observations = analysis ? analysisObservations(analysis) : [];
</script>

<section class="tool-section analysis-section" aria-labelledby="analysis-heading" aria-busy={analyzing}>
  <h2 id="analysis-heading"><span>◎</span> Image Analysis</h2>
  {#if analyzing}
    <p class="analysis-state"><i></i> Inspecting local pixels…</p>
  {:else if analysis}
    <ul>
      {#each observations as observation}
        <li>{observation}</li>
      {/each}
    </ul>
    <details>
      <summary>Heuristic measurements</summary>
      <dl>
        <div><dt>Average light</dt><dd>{Math.round(analysis.averageLuminance * 100)}%</dd></div>
        <div><dt>Tonal spread</dt><dd>{Math.round(analysis.luminanceSpread * 100)}%</dd></div>
        <div><dt>Noise estimate</dt><dd>{Math.round(analysis.estimatedNoise * 100)}%</dd></div>
        <div><dt>Sharpness estimate</dt><dd>{Math.round(analysis.estimatedSharpness * 100)}%</dd></div>
      </dl>
    </details>
    <p class="disclaimer">Observations are deterministic heuristics, not diagnoses. Nothing is applied automatically.</p>
  {:else}
    <p class="analysis-state">Analysis becomes available after opening an image.</p>
  {/if}
</section>

<style>
  .analysis-section { gap: 11px; }
  ul { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
  li { position: relative; padding-left: 13px; color: var(--ink-soft); font-size: 0.63rem; line-height: 1.4; }
  li::before { content: '·'; position: absolute; left: 1px; color: var(--accent); font-weight: 900; }
  .analysis-state { margin: 0; color: var(--ink-faint); font-size: 0.63rem; }
  .analysis-state i { display: inline-block; width: 7px; height: 7px; margin-right: 6px; border-radius: 50%; background: var(--accent); animation: pulse 900ms ease-in-out infinite alternate; }
  details { border-top: 1px solid var(--line); padding-top: 8px; }
  summary { color: var(--ink-faint); font-size: 0.58rem; cursor: pointer; }
  dl { display: grid; gap: 4px; margin: 9px 0 0; }
  dl div { display: flex; justify-content: space-between; gap: 8px; }
  dt, dd { margin: 0; color: var(--ink-faint); font: 0.56rem/1.3 var(--font-mono); }
  dd { color: var(--ink-soft); }
  .disclaimer { margin: 0; color: var(--ink-faint); font-size: 0.55rem; line-height: 1.45; }
  @keyframes pulse { to { opacity: 0.3; } }
</style>
