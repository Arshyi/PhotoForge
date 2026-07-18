# Phase 3 results

PhotoForge 0.3.0 adds guided local editing through a deterministic `RuleBasedPlanner`. It converts supported ordinary-language requests into typed, validated plans made only from existing local image operations. The planner neither edits pixels nor contacts a model or service. Users review the summary, heuristic confidence, warnings, operation explanations, order, and strengths before applying a plan.

## Delivered scope

- Public `EditPlanner` trait and the sole `RuleBasedPlanner` implementation
- Analysis-aware phrase rules for brightness, color casts, weak or harsh contrast, noise, JPEG damage, blur, sharpening, uneven lighting, documents, receipts, scans, handwriting, and old photos
- Typed plan summary, bounded heuristic confidence, warnings, operations, and one human-readable explanation per operation
- Validation for unknown operations, non-finite or out-of-range values, empty/oversized plans, duplicates, conflicts, unsafe ordering, and explanation mismatches
- Guided Edit request panel, ten suggestions, optional local history capped at 25 entries, and four local preferences
- Review inspector with removal, reordering, adjustable strengths, Apply, Cancel, Edit Plan, Enter generation, Ctrl+Enter apply, and Escape cancel behavior
- Stale document/request protection and reuse of cached image analysis
- No LLM, Ollama, neural model, OCR, cloud API, telemetry, or new runtime dependency

## Automated verification

The final authoritative `E:\PhotoForge` tree passed:

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| `cargo test --workspace` | 112 passed, 0 failed |
| `npm run check` | 0 errors, 0 warnings |
| `npm run test` | 56 passed, 0 failed |
| `npm run build` | Pass; 136 modules transformed |
| `npm run tauri build` | Pass; portable EXE, NSIS, and MSI produced |

The Rust planner/validation module contains 61 focused tests. Frontend coverage includes prompt constants and local history/settings helpers plus rendered inspector behavior, stale responses, operation deletion/reordering/strength changes, validation before apply, and keyboard shortcuts.

## Packaged workflow results

The packaged application opened a real dark-photo fixture and the specification request “Make this darker but bring out the writing” produced a two-operation typed plan in 0.03 ms. The inspector displayed a summary, `High · 79%` heuristic confidence with the explicit “not AI certainty” label, two analysis-aware warnings, Document Enhance and Local Contrast, and an explanation and strength control for each operation.

The final packaged Rust command boundary was then exercised with real local fixtures. Every case opened, analyzed, and returned a current validated plan:

| Case | Request | Result | Planner time |
| --- | --- | --- | ---: |
| Dark photo | Make this photo brighter | Brightness +0.14 | 0.0117 ms |
| Bright photo | This is too bright | Brightness −0.14 | 0.0118 ms |
| Receipt | Clean up this receipt and fix uneven lighting | Uneven Lighting, Document Enhance | 0.0133 ms |
| Handwritten notes | Make this darker but bring out the writing | Document Enhance, Brightness, Local Contrast | 0.0125 ms |
| Screenshot | Make handwriting easier to read | Document Enhance | 0.0114 ms |
| Old scan | Improve this old scan | White Balance, Document Enhance, Local Contrast, Denoise, Sharpen | 0.0161 ms |
| Damaged JPEG | Reduce JPEG artifacts | JPEG Cleanup, Denoise | 0.0143 ms |
| Portrait | Sharpen slightly | Edge-Aware Sharpen | 0.0116 ms |
| Transparent PNG | Make colors more natural | Auto White Balance | 0.0121 ms |
| 12 MP JPEG | Improve without changing colors | Local Contrast | 0.0112 ms |
| 24 MP JPEG | Reduce noise | Denoise | 0.0105 ms |

All ten visible suggested prompts also returned non-empty valid plans. Unknown intent is rejected instead of guessed. A corrupt WebP returned the typed `corrupt_image` error and left the prior document session usable. The frontend stale-response checks and packaged rapid-request check retained only the newest relevant plan.

The 24 MP fixture exported a reviewed Brightness + Local Contrast pipeline at full 6000×4000 resolution:

| Format | Time | Output size |
| --- | ---: | ---: |
| PNG | 2,191 ms | 1,620,390 bytes |
| JPEG | 2,877 ms | 918,646 bytes |
| WebP | 2,197 ms | 1,060,876 bytes |

The NSIS installer returned exit code 0, registered PhotoForge 0.3.0, launched a responsive installed window in approximately 755 ms, and removed both its install directory and uninstall registration successfully. The portable build produced its first responsive window in approximately 671 ms. No external TCP connection was present during the packaged offline check.

## Planner performance

One hundred sequential packaged planner calls on cached analysis measured 0.0093 ms minimum, 0.0117 ms median, 0.0145 ms p95, and 0.0206 ms maximum. All samples were below the 50 ms target. These timings exclude image decode and initial analysis by design.

## Release artifacts

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `PhotoForge-portable.exe` | 10,548,224 bytes | `2D2EBE87117E53A46599EE0A57C6FE52F36399DC0DA2F62CE194299FE93C78E1` |
| `PhotoForge_0.3.0_x64-setup.exe` | 2,407,396 bytes | `C8AD5B40E61CA28379FA71D581473A32E93FC9F9973D6DE69E2634760D16CBC4` |
| `PhotoForge_0.3.0_x64_en-US.msi` | 3,547,136 bytes | `00755D5145B44BA66E5AE2DA4B3B639EBD3407904431A01CBC59ED5B5775BAC9` |

`release/SHA256SUMS.txt` contains the same hashes. No npm or Cargo package was added in Phase 3.

## Honest limitations

- The planner recognizes a documented bounded vocabulary; it is not an open-ended language model or chatbot.
- Heuristic confidence reports rule-match strength, not factual certainty or image-quality correctness.
- Plans can improve captured pixels only. They cannot reconstruct missing factual detail.
- OCR, perspective correction, auto-crop, batch workflows, neural restoration, super-resolution, and generated content remain outside 0.3.0.
