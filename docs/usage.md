# Setup and usage

## Install and key

Download the x64 setup executable from [Releases](https://github.com/nikhil-swamix/tiny-voice/releases/latest) and run it. Installation is per-user. It creates a Start menu shortcut and downloads WebView2 if needed. Node.js/Rust are not needed to run the app.

You need your own OpenAI API key with access to the documented models. Quit any running Tiny Voice instance from its tray, open PowerShell and run:

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
Get-FileHash -Algorithm SHA256 '.\Tiny.Voice_0.2.0_x64-setup.exe'
```

## Record

1. Focus the app/field where you want notes pasted.
2. Press `Ctrl+Shift+Space`, speak, press again to stop. Each action makes a 200 ms beep.
3. The preview streams during capture; stop transcribes the complete file, formats it in Turbo by default, copies and pastes.

The microphone icon does the same. Permission is needed before capture. Live connection failure keeps local recording available. Limits: ten minutes and 20 MB per recording.

## Controls

| Control | Action |
| --- | --- |
| Drag bar/header | Move widget |
| Click empty minibar space | Expand |
| Quick / Turbo in expanded view | Select future output mode |
| Turbo bolt | Reprocess latest transcript and deliver again |
| X | Hide to tray |
| Tray click / Show | Restore |
| Tray Quit | Save active audio and exit |
| Release mic | Release mic; save active recording first |
| Audio folder | Open local files |

The attached preview grows/shrinks with text, up to a screen-relative height limit, then scrolls. Logical pixels respect Windows scaling. Expanded content reflows and scrolls.

The mic stays ready up to five minutes at launch/after recording. Idle audio is never saved or uploaded. Windows manages sharing; mute/ended events release the stream and later recording reacquires it. Hiding an idle widget releases its mic. Five minutes of inactivity fades the widget to 50%; focus loss/completion also dims the collapsed view. Expanding restores visibility.

## Data and privacy

Files in `%LOCALAPPDATA%\com.local.tinyvoice\recordings`:

- `<id>.webm`: audio.
- `<id>.raw.txt`: raw transcript.
- `<id>.txt`: latest output or raw fallback.
- `<id>.quick.md` / `<id>.pro.md`: processed variants.
- `speech-context.json`: archive, intent, mode and provider metadata.

Current audio is sent to OpenAI for live/file transcription. Formatting sends saved transcript history, emphasizing the last ten. This can include private speech. No personal data is bundled or published. Files remain until removed; uninstall does not promise to erase recordings.

To reset history, quit and remove both transcript files and `speech-context.json` from this folder. Removing JSON alone is insufficient: history is rebuilt from transcript files. Back up desired recordings first.

## Troubleshooting

- **Key missing:** set `OPENAI_API_KEY`, quit from tray and relaunch from that PowerShell session or after signing in again.
- **No mic:** allow desktop microphone access in Windows Settings; release other exclusive device users.
- **No live preview:** saved audio can still receive final transcription. Check internet and realtime model access.
- **Raw transcript saved:** audio/raw text remain; the error identifies the formatting failure. Model metadata is unverified for this fallback.
- **Copied, paste target unavailable:** focus your destination and press `Ctrl+V`. Windows may block pasting into an elevated application.
- **Widget hidden:** click the tray icon or start another instance.
- **Uninstall:** tray Quit, then Windows Settings → Apps → Tiny Voice → Uninstall.
