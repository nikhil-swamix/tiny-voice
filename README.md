# Tiny Voice

A floating Windows voice widget. Speak, get clear task notes, and paste them into the app you were using.

![Tiny Voice: joined minibar and task preview](screenshots/product-preview.png)

## Install

1. Download **Tiny.Voice_0.2.0_x64-setup.exe** from [GitHub Releases](https://github.com/nikhil-swamix/tiny-voice/releases/latest).
2. Run the per-user installer. It creates a Start menu shortcut and downloads WebView2 if missing.
3. Set your OpenAI API key and launch Tiny Voice; follow [setup and usage](docs/usage.md).

Windows 10/11 x64, microphone access, internet and an OpenAI API account with model access are needed. The installer is unsigned. No credentials or personal recordings are included. Release downloads include SHA-256 checksums.

## Use

- **Ctrl+Shift+Space** starts/stops recording, with a 200 ms beep for each action.
- **Turbo is the default.** It produces a concise task brief, suggested steps/checks and three task-specific hints. **Quick** preserves the spoken task with one hint.
- Both modes prefer at most **200 words**, expanding when necessary to preserve the request.
- Finished Markdown is automatically copied and pasted into the app active when recording began. The minibar **Turbo** button reprocesses the latest transcript without recording again.
- The minibar and preview have the same width, a zero-gap printer-style join, and square preview top corners. Text grows/shrinks the window; long notes scroll within a screen-relative limit.
- Drag the bar to move it. Click its empty area to expand. Focus loss or transcript completion collapses it. Five minutes of inactivity fades it to 50% opacity.
- X hides to tray; the tray restores the widget, toggles recording or quits.
- The mic stays ready for up to five minutes. **Release mic** releases it immediately. Idle audio is never recorded or uploaded.

The engine formats speech into tasks without answering or executing them. Speech with no assignment stays a greeting, statement or observation. The expanded view shows engine status and provider-returned model metadata.

## Models and data

| Stage | Requested model | Configuration |
| --- | --- | --- |
| Live preview | `gpt-live-transcribe` | WebRTC transcription |
| Complete recording | `gpt-4o-transcribe` | File transcription; cached raw text |
| Quick / Turbo | `gpt-6.1-sol` | Responses API; **medium** reasoning; strict structured output |

Turbo is an app output mode, separate from provider execution modes. Read [model documentation](docs/models.md) for requests, verification, context and limits.

Recordings are under `%LOCALAPPDATA%\com.local.tinyvoice\recordings`. Current audio is sent to OpenAI for transcription. Formatting sends the raw transcript, saved transcript archive and latest ten entries as context. Keys stay in the native process. See [data and privacy](docs/usage.md#data-and-privacy).

## Build

Install Node.js, Rust MSVC, Microsoft C++ Build Tools and WebView2, then:

```powershell
git clone https://github.com/nikhil-swamix/tiny-voice.git
cd tiny-voice
npm ci
npm test
cargo test --manifest-path src-tauri/Cargo.toml
npm run desktop:release
npm run installer
```

Production executable: `src-tauri/target/release/tiny-voice.exe`. NSIS installer: `src-tauri/target/release/bundle/nsis`. `npm start` runs desktop development; `npm run desktop` builds an embedded debug app.

The WebView handles capture, streaming, the meter, Markdown and layout. Tauri handles API credentials/calls, local files, hotkeys, clipboard, tray and window controls. Markdown uses `marked` and DOMPurify. Production builds use LTO and stripped binaries.

## Checks and release

- `npm test`: mic lifetime, recording races, automatic delivery, fading and rendering.
- `cargo test --manifest-path src-tauri/Cargo.toml`: request settings, context and output structure.
- `cargo test --manifest-path src-tauri/Cargo.toml live_ -- --ignored --nocapture`: three paid synthetic API requests; requires `OPENAI_API_KEY`.
- With Vite at port 1420, `node tests/output-smoke.mjs` checks layout, shrinking/growing, safe Markdown, defaults and delivery with synthetic media.
- `node tests/native-smoke.mjs` checks an executable launched with temporary WebView2 debug port 9237. Normal launches do not enable it.

Release order: interface/processing, checks, installer/installation test, docs, source push, draft asset uploads, public release last. [0.2.0 release notes](docs/releases/0.2.0.md).

The product image uses the tested interface with a synthetic transcript. Regenerate with `python scripts/create_preview.py` after the renderer smoke check.
