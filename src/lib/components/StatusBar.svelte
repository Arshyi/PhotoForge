<script lang="ts">
  export let dimensions = 'No image';
  export let zoom = 100;
  export let operationCount = 0;
  export let processingTime = 0;
  export let isCurrent = true;
</script>

<footer>
  <span>{dimensions}</span>
  <i></i>
  <span>{zoom}%</span>
  <i></i>
  <span>{operationCount} {operationCount === 1 ? 'operation' : 'operations'}</span>
  <span class="spacer"></span>
  <span>{processingTime.toFixed(0)} ms</span>
  <i></i>
  <span class:current={isCurrent} class="state"><b></b>{isCurrent ? 'Preview current' : 'Updating preview'}</span>
</footer>

<style>
  footer {
    height: 28px;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 14px;
    border-top: 1px solid var(--line);
    color: var(--ink-faint);
    background: var(--surface);
    font: 600 0.63rem/1 var(--font-mono);
    letter-spacing: 0.01em;
  }

  i {
    width: 2px;
    height: 2px;
    border-radius: 50%;
    background: var(--ink-faint);
  }

  .spacer { flex: 1; }
  .state { display: flex; align-items: center; gap: 6px; }
  .state b { width: 6px; height: 6px; border-radius: 50%; background: #d7aa65; }
  .state.current b { background: var(--accent); }

  @media (max-width: 620px) {
    footer > :nth-child(3), footer > :nth-child(4), footer > :nth-child(5) { display: none; }
  }
</style>
