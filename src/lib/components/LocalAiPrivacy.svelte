<section class="local-ai-privacy" aria-labelledby="local-ai-privacy-heading">
  <div class="settings-intro">
    <span>Explicit local boundary</span>
    <h2 id="local-ai-privacy-heading">Local AI Privacy</h2>
    <p>Ollama is optional and untrusted. It can propose JSON, but it cannot apply edits or access application capabilities.</p>
  </div>

  <div class="privacy-boundary" role="note">
    <strong>What PhotoForge sends after Test Connection, Refresh Models, Generate Plan, or Compare Planners</strong>
    <p>The configured local Ollama endpoint receives only the user prompt and an approved deterministic analysis summary.</p>
  </div>

  <ul class="privacy-promises">
    <li>PhotoForge sends requests only to the configured local Ollama endpoint.</li>
    <li>No cloud services are contacted.</li>
    <li>No images leave the computer.</li>
    <li>Only the analysis summary and user prompt are sent for plan generation.</li>
    <li>Image pixels, thumbnails, file contents, paths, usernames, configuration, operating-system details, and environment variables are never sent.</li>
    <li>Users remain responsible for any external software they configure behind a local endpoint.</li>
  </ul>

  <div class="privacy-flow" aria-label="Local planner security flow">
    <span>Natural language</span><b>→</b><span>Untrusted JSON plan</span><b>→</b><span>Strict validation</span><b>→</b><span>Review</span><b>→</b><span>Deterministic engine</span>
  </div>

  <p class="modal-footnote">No automatic connections, model downloads, model installation, telemetry, cloud fallback, or automatic plan application occur.</p>
</section>
