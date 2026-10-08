"""Assemble release assets from an explicit public-file allowlist."""
import hashlib, json, shutil, zipfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
version = json.loads((root / 'package.json').read_text())['version']
out = root / 'release'
out.mkdir(exist_ok=True)
installer = out / f'Tiny.Voice_{version}_x64-setup.exe'
shutil.copy2(root / f'src-tauri/target/release/bundle/nsis/Tiny Voice_{version}_x64-setup.exe', installer)
portable = out / f'Tiny.Voice_{version}_x64-portable.zip'
files = [root / 'README.md', *sorted((root / 'docs').rglob('*.md')), *sorted((root / 'screenshots').glob('*.png'))]
with zipfile.ZipFile(portable, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
    archive.write(root / 'src-tauri/target/release/tiny-voice.exe', 'tiny-voice.exe')
    for file in files:
        archive.write(file, file.relative_to(root))
(out / 'SHA256SUMS.txt').write_text(''.join(f'{hashlib.sha256(file.read_bytes()).hexdigest()}  {file.name}\n' for file in [installer, portable]), encoding='ascii')
print('\n'.join(f'{file.name}: {file.stat().st_size:,} bytes' for file in [installer, portable, out / 'SHA256SUMS.txt']))
