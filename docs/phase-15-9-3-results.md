# Phase 15.9.3 — a window that does not close can now be explained

[15.9.1](phase-15-9-1-results.md) recorded a window that did not close on `WM_CLOSE`, seen in the
packaged window with a document that was failing to render. It could not be reproduced on demand
([15.9.2](phase-15-9-2-results.md): 3 failures in about 15 attempts with the render commands made to
fail, none with a healthy document, none in later batches), and with the page alone as the record
there was no way to tell what had happened. This adds the record.

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-9-3-close-diagnostic`, from `main` (`8f7e32a`).

## What it is

A local log of what happens when the window is asked to close
(`%LOCALAPPDATA%\PhotoForge\diagnostics\close.log`, `src-tauri/src/close_log.rs`). Each request
writes up to three lines:

```
1791881489 close requested
1791881489 the page decided: close
1791881489 the window was destroyed
```

* **`close requested`** is written by the backend when the operating system's request arrives
  (`WindowEvent::CloseRequested`), whether or not a page is there to answer it.
* **`the page decided: …`** is what the interface decided, one of `close`, `prompt-busy`,
  `prompt-dirty` or `failed` (`note_close_decision`, called from the close-requested handler with
  the label from `closeDecisionLabel`). Anything else is written as `unrecognised`.
* **`the window was destroyed`** is written when the window goes away.
* **`the window is still open 3 s after the request`** is written by a watchdog if it did not.

So the cases are told apart by which lines are there: no decision line means the page never
answered; a `close` decision with no `destroyed` line, and the still-open line, means the page
answered and the window stayed (the destroy call failed, which is how the one failure that was
seen first looked); a `prompt-…` decision is a window held open on purpose.

## What it does not do

* **It does not change what happens.** The decision is the same function as before, the call is
  fire and forget and its failure is ignored, and nothing waits on the log. It does not close a
  window that stayed open. A fallback that did, from the backend after a few seconds, would risk
  discarding unsaved work when the page simply has not answered yet, so it was left as a decision
  for a person.
* **It records no document, path or name.** Timestamps (seconds since the epoch) and fixed phrases
  only; the file is cut back to nothing when it passes 32 KiB; nothing is sent anywhere.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets | clean, all four (all features, no GPU, no inference, no plugins) |
| `cargo test --all-targets --no-fail-fast` | **1,993** / **1,986** / **1,985** / **1,954** passed, 0 failed (60 binaries), same four feature sets (15.9.2 ended at 1,989 / 1,982 / 1,981 / 1,950: four new unit tests) |
| `npx tsc --noEmit`, `npm run check`, `npx vitest run`, `npm run build` | 0 errors, 0 warnings (523 files); **1,389** tests in 93 files passed (one new); build succeeds. One unhandled error, the `CommandPalette` test that throws on purpose, unchanged |
| Packaged window | with a RAW region open and the render commands made to fail, 8 of 8 closes wrote the three lines above within the same second and the window exited at once |

**Unit tests:** a line is stamped and appended and the directory is created; a file past the bound
is cut back rather than grown; a file that cannot be written (a directory where it should be) is not
an error; the decisions the log accepts are a fixed set. **Interface:** the label for every state
of `decideClose` is one of three fixed phrases and says what was decided.

## What was not verified

* **The failure it is for.** It did not recur in these 8 runs, so the log has not yet explained
  anything. If the window does not close again, `close.log` (in the user's local application data,
  or the scratch copy used for validation) is the first thing to read.
* **That the watchdog line appears.** It is exercised only in its absence (the window closed every
  time); the code path that writes it is the same function as the other lines.
* **The `failed` decision.** It is written when working out the real decision throws (which then
  closes the window, as before). That branch was added after the packaged run and is covered by the
  type check and the unit test of the label set, not by a window that hit it.
