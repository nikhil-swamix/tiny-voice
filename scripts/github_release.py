"""Publish checked local artifacts through GitHub using the existing Git credential helper."""
import argparse, json, subprocess, urllib.parse, urllib.request
from pathlib import Path

REPO = 'nikhil-swamix/tiny-voice'

def credential():
    result = subprocess.run(['git', 'credential', 'fill'], input='protocol=https\nhost=github.com\n\n', text=True, capture_output=True, check=True)
    fields = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
    return fields['password']

def api(path, method='GET', data=None, binary=False):
    url = path if path.startswith('https://uploads.github.com/') else f'https://api.github.com/repos/{REPO}{path}'
    body = data if binary else json.dumps(data).encode() if data is not None else None
    request = urllib.request.Request(url, body, {'Authorization': f'Bearer {TOKEN}', 'Accept': 'application/vnd.github+json', 'Content-Type': 'application/octet-stream' if binary else 'application/json', 'X-GitHub-Api-Version': '2022-11-28'}, method=method)
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)

parser = argparse.ArgumentParser()
parser.add_argument('action', choices=['info', 'publish', 'verify'])
parser.add_argument('--version')
parser.add_argument('--notes', type=Path)
parser.add_argument('--assets', type=Path, nargs='+')
args = parser.parse_args()
TOKEN = credential()
if args.action == 'info':
    repo = api(''); print(json.dumps({'repo': repo['full_name'], 'private': repo['private'], 'releases': [r['tag_name'] for r in api('/releases')]}))
elif args.action == 'verify':
    release = api(f'/releases/tags/v{args.version}')
    print(json.dumps({'url': release['html_url'], 'draft': release['draft'], 'assets': [{'name': a['name'], 'size': a['size'], 'url': a['browser_download_url']} for a in release['assets']]}))
else:
    assert args.version and args.notes and args.assets, 'Version, notes and assets are required'
    assert all(p.is_file() and p.stat().st_size for p in args.assets), 'All assets must exist'
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    release = api('/releases', 'POST', {'tag_name': f'v{args.version}', 'target_commitish': commit, 'name': f'Tiny Voice {args.version}', 'body': args.notes.read_text(encoding='utf-8'), 'draft': True, 'prerelease': False})
    for asset in args.assets:
        api(release['upload_url'].split('{')[0] + '?name=' + urllib.parse.quote(asset.name), 'POST', asset.read_bytes(), True)
        print(f'Uploaded {asset.name}')
    published = api(f"/releases/{release['id']}", 'PATCH', {'draft': False})
    print(published['html_url'])
