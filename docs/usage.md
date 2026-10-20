# Setup and usage

## Install and key

Download the x64 setup executable from [Releases](https://github.com/nikhil-swamix/tiny-voice/releases/latest) and run it. Installation is per-user. It creates a Start menu shortcut and downloads WebView2 if needed. Node.js/Rust are not needed to run the app.

The default local pipeline needs no OpenAI key. Set your own key with access to the documented models only if you enable GPT Transcribe or the OpenAI formatter. Quit any running Tiny Voice instance from its tray, open PowerShell and run:

```powershell
$voiceKey = Read-Host 'OpenAI API key' -AsSecureString
$env:OPENAI_API_KEY = [System.Net.NetworkCredential]::new('', $voiceKey).Password
[Environment]::SetEnvironmentVariable('OPENAI_API_KEY', $env:OPENAI_API_KEY, 'User')
Remove-Variable voiceKey
Start-Process -FilePath "$env:LOCALAPPDATA\Tiny Voice\tiny-voice.exe"
```

This launch reads the new key. Later Start menu launches inherit Windows environment variables: sign out/in after changing them, or use the PowerShell launch above. Never put keys in source or screenshots. A ChatGPT subscription does not supply API credentials.

The installer is unsigned, so Windows may show an unknown publisher warning. Download hashes are in `SHA256SUMS.txt`:

```powershell
Get-FileHash -Algorithm SHA256 '.\Tiny.Voice_0.3.0_x64-setup.exe'
```

## Record

1. Focus the app/field where you want notes pasted.
2. Press `Ctrl+Shift+Space`, speak, press again to stop. Each action makes a 200 ms beep.
3. The bar shows provisional Nemotron CPU captions. Stop finalizes that local transcript by default and sends it to the local SGLang/MiniCPM5 profile, which formats shallow YAML. GPT Transcribe is off by default; enable it in Settings to upload original audio for a more accurate transcript. Exact YAML is copied/pasted.

The microphone icon does the same. Permission is needed before capture. Live connection failure keeps local recording available. Limits: ten minutes and 20 MB per recording.

## Controls

| Control | Action |
| --- | --- |
| Drag bar/header | Move widget |
| Preview / expand button | Click to reveal full output; the button also collapses it. Hover does not expand |
| History button | Expand and browse saved titles/timestamps; select one to preview it |
| Dictation indicator | One faithful cleanup mode; no enrichment or selector |
| X | Hide to tray |
| Tray click / Show | Restore |
| Tray Quit | Save active audio and exit |
| Release mic | Release mic; save active recording first |
| Audio folder | Open local files |
| Footer copy icon | Copy exact final YAML; legacy entries retain their original Markdown |

The app starts hidden in the tray; the widget appears at bottom center while recording and hides ten seconds after automatic copy/paste finishes. Compact and expanded views use the same fixed 360-logical-pixel width, bounded once to smaller screens. Compact height is 176 pixels, with at most two preview lines. Click the preview or expand button for full output; Enter/Space activates the preview when keyboard focus is available. Expanded height is 520 pixels, capped to 85% of available screen height. Settings, history, and output scroll inside the panel while controls stay visible. No content-driven resizing or hover expansion occurs.

The mic stays ready up to five minutes after recording. Idle audio is never saved or uploaded. Hiding an idle widget releases its mic. Text and controls remain fully readable without hover, completion, or inactivity fading.

## Data and privacy

Files in `%LOCALAPPDATA%\com.local.tinyvoice\recordings`:

- `<id>.webm`: audio.
- `<id>.raw.txt`: raw transcript.
- `<id>.txt`: latest output or raw fallback.
- `<id>.md`: raw transcript and final output, in titled sections separated by `==========`. Older recordings may retain `.quick.md` / `.pro.md` variants.
- `speech-context.json`: archive, intent, mode and provider metadata.
- `speech-context-summary.json`: cached compact voice-workflow memory, without replacing the archive.
- `vocabulary-corrections.json`: explicit contextual spelling candidates with evidence.
- `<id>.gpt-transcript.json`: cached GPT audio recognition for formatting retries; older local-transcript caches remain untouched.
- `<id>.live.txt`: finalized CPU preview, when available.
- `formatting-prompt.txt`: optional custom formatter instructions saved from Settings; absent means the built-in default.

Live recognition runs locally on CPU. GPT Transcribe is off by default; enabling it uploads original audio. Default local SGLang formatting and history compaction send text only to the loopback service. Choosing OpenAI formatting sends recognized text and bounded context, plus screenshots if enabled, even with GPT Transcribe off. Compaction can send saved historical speech to the selected formatter. No data goes to Deepgram; no personal data is bundled. Files remain until removed.

To reset history, quit and remove transcript files, `speech-context.json`, and `speech-context-summary.json` from this folder. Removing JSON alone is insufficient: history is rebuilt from transcript files. Back up desired recordings first.

## Troubleshooting

- **Key missing:** appears only when GPT Transcribe or the OpenAI formatter is enabled. Set `OPENAI_API_KEY`, quit from tray and relaunch from that PowerShell session or after signing in again, or return to local formatting with GPT Transcribe off.
- **No mic:** allow desktop microphone access in Windows Settings; release other exclusive device users.
- **No live preview:** check cached Nemotron weights, ffmpeg on PATH and the displayed CPU error. Audio is still saved for GPT transcription; network access is needed after stop, not for local captions.
- **Raw transcript saved:** audio/raw text remain; the error identifies the formatting failure. Model metadata is unverified for this fallback.
- **Copied, paste skipped:** the captured destination is no longer focused, input is busy, or Windows blocked insertion. Focus the intended field and press `Ctrl+V`.
- **Widget hidden:** click the tray icon or start another instance.
- **Uninstall:** tray Quit, then Windows Settings → Apps → Tiny Voice → Uninstall.

## Local streaming and cloud refinement

The default profile requires the local SGLang service at `http://127.0.0.1:30000/v1`, serving `minicpm5-2b-fp8`; Tiny Voice does not start or install the server. GPT Transcribe is off by default. Set `OPENAI_API_KEY` only when enabling GPT Transcribe or the optional OpenAI formatter. Local streaming requires ffmpeg and Handy's cached Nemotron Q8_0 weights.

Local captions are English-only. Settings language applies to GPT transcription: regional English selections use the `en` hint, Hindi uses `hi`, and automatic/multilingual omits a hint. The obsolete Deepgram accuracy tiers and key field are removed. A live-model/decoder failure does not prevent saving audio or requesting GPT transcription. Cloud errors preserve audio and never silently use another model.

Formatting keeps as close as possible to the dictation: punctuation, capitalization, obvious filler/stutter cleanup, final self-corrections and minimal layout. No added advice, steps, checks, padded titles or mandatory hints. Questions remain questions. Old enrichment/word-count preferences in history cannot override this policy.

## Current widget update

The AI-block panel uses matte sRGB colors, consistent spacing, crisp SVG controls, and inward focus rings. Shadows, gradients, glow, blur, decorative pulsing, and the manual resize grip are removed. Compact preview truncation is intentional; full content remains available on explicit expansion.

## Recording screen context

Desktop images are off by default. If enabled in Settings, the entire desktop is captured at recording start and every five seconds while recording, including every monitor. Screenshots may include visible private content. They stay in memory until formatting, go only to OpenAI with that recording, and are not saved in history. Stop/cancel ends collection promptly; idle mode never captures screens. The elapsed clock updates every 500 ms. Its 2-pixel progress track fills over 60 seconds, then stays full while elapsed time continues; 01:00 does not stop recording. The clock turns amber after one minute. Starting a recording opens the compact widget at bottom center.

Screenshot compression targets 220 KB per image. Images may lose fine detail. New Markdown files contain raw and final sections; copying uses exact final YAML without Markdown fences.

## Focus-safe paste and output lifetime

The compact widget is non-activating during normal recording; explicit expansion, Settings and History allow keyboard interaction. Automatic paste never activates or restores another app. It snapshots the foreground window, process and focused native control at recording start and checks them immediately before grouped Ctrl+V input. Changed destinations, active menus, held modifiers or unavailable input skip paste while leaving final text copied. Browser inputs sharing a native focus handle cannot be distinguished; the app never moves the caret or selects a different field.

Transcript displays stay fully readable. After copy/paste finishes, the widget hides after ten seconds. History titles come from output headings when present, otherwise the first nonempty dictation line; timestamps come from saved Markdown files. Selecting a row previews that entry. Expanding/collapsing is explicit, and incoming output does not override the selected view.

Native focus inspection uses [Microsoft GetGUIThreadInfo](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getguithreadinfo).

## Expanded controls and settings

The toolbar remains available in both views. Settings automatically saves post-processing profile, loopback endpoint/model, GPT Transcribe switch, language and optional cloud controls. The formatting prompt has its own editor and Save/Restore buttons; it is stored as `formatting-prompt.txt` in the recordings folder. The strict YAML schema remains built in. Local formatting is text-only and disables screenshots. Defaults: local SGLang/MiniCPM5, GPT Transcribe off, images off. Per-recording metadata shows measured stage times; cached GPT transcription is 0 ms and local transcription timing covers only the final CPU drain, not prior live inference. Stage sum excludes capture, compaction and persistence.


The fixed-width expanded panel scrolls within screen-bounded height while the header and toolbar remain visible. Compact mode shows no more than two lines before explicit interaction. The one-minute guide does not replace the existing ten-minute/20 MB recording safety limits. Older saved mode metadata and archive variants remain readable.
