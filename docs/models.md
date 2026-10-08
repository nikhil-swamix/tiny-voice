# Model configuration

## Pipeline

1. MediaRecorder starts after microphone permission, before storage setup and live negotiation finish. Early audio chunks are buffered locally.
2. WebRTC with `gpt-live-transcribe` supplies the live preview. `OPENAI_REALTIME_TRANSCRIBE_MODEL` can override this ID. Turn detection is disabled; the recording button controls capture. A failed live session leaves local recording available.
3. On stop, the complete saved WebM file goes to `gpt-4o-transcribe`. Raw text is cached and reused by upgrades.
4. Native code sends raw text and history to `/v1/responses`, requesting `gpt-6.1-sol`, `reasoning.effort: medium`, `store: false`, and a strict JSON schema for sections, subsections, intent and hints.
5. The app creates standard Markdown; DOMPurify sanitizes the display. Exact Markdown is copied and pasted when Windows can restore the captured destination.

## Output contract

Turbo is the default on first launch and when migrating older settings. A later explicit Quick selection persists. Both use medium reasoning and a 16,384-token generation allowance, including reasoning. This allowance is separate from the preferred text length.

Target: clean output of up to 200 words, including headings and hints. Simple speech should be shorter. Expand only when needed to preserve constraints or requested detail. Text is never cut off to enforce the target; this is a prompt preference, not an exact word-count guarantee.

- **Quick:** faithful notes and one 10–20-word hint for an actionable task.
- **Turbo** (internal mode `pro`): task brief, clearly titled suggested approaches/checks/contextual tips, and three distinct 10–20-word hints.
- **No assignment:** only the greeting, statement or observation; no invented plan or completion hints.
- Intent is internal metadata, not a displayed heading.
- Titles and single bullets use standard Markdown with two-space indentation; no decorative ASCII, custom brackets or duplicate markers.

The prompt preserves names, numbers, constraints, language and self-corrections. It does not answer or execute spoken tasks, browse, or claim completed work. Turbo additions are suggestions from general knowledge and may be wrong.

## Context and credentials

Formatting includes the current raw transcript, all saved prior entries, and the last ten in a separate recent-context field. Recent context is prioritized for references; unrelated tasks should not be imported. Large archives increase latency and cost and can eventually exceed context limits.

`OPENAI_API_KEY` is read by native code at launch. It is not embedded, exposed to the WebView or released. Local audio/history are retained until removed. Audio and transcript context are sent to OpenAI as described above. `store: false` requests no Responses object storage; it does not establish zero retention for every API service.

## Requested versus verified

The request lives in `src-tauri/src/processing.rs`. Each completed output shows the **provider-returned model ID and reasoning effort** in the expanded widget. Missing fields say unverified. Older history may contain earlier high-reasoning outputs; those entries are preserved.

The 0.2.0 live check made three synthetic requests: Quick, Turbo and an unassigned greeting. Response metadata returned `gpt-6.1-sol` / `medium`. Both task outputs completed within 200 words for that test input. This verifies those requests, not every future input or account's access.

Errors/refusals/incomplete responses retain saved audio and raw text. Failed Turbo reprocessing keeps previous output. No model is silently substituted.

## Official references

- [GPT-6.1 Sol and reasoning support](https://developers.openai.com/api/docs/models/gpt-6.1-sol)
- [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription)
- [GPT-4o Transcribe](https://developers.openai.com/api/docs/models/gpt-4o-transcribe)

Checked for this release on October 8, 2026.
