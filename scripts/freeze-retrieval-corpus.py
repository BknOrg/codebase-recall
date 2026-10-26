"""Freeze an explicit, local BknDb Git revision; never modifies the source repository."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

REVISION = '509f9414adc0e312354965b584cd3e165d02a08e'
p = argparse.ArgumentParser()
p.add_argument('--repo', required=True)
p.add_argument('--output', default='tests/fixtures/retrieval_bkndb')
a = p.parse_args()
def git(*args):
    return subprocess.check_output(['git', '-C', a.repo, *args])
out = Path(a.output)
out.mkdir(parents=True, exist_ok=True)
paths = git('ls-tree', '-r', '--name-only', '-z', REVISION).decode().split('\0')
sources = {}
for path in sorted(paths):
    if Path(path).suffix in {'.rs', '.py', '.go', '.kt', '.java', '.js', '.ts', '.vue', '.svelte'}:
        sources[path] = git('show', f'{REVISION}:{path}').decode('utf-8')
corpus = json.dumps(sources, ensure_ascii=False, sort_keys=True, indent=2) + '\n'
(out / 'corpus.json').write_text(corpus, encoding='utf-8', newline='\n')
manifest = {'repository': 'https://github.com/BknOrg/bkn-db', 'revision': REVISION,
            'corpus_sha256': hashlib.sha256(corpus.encode()).hexdigest(),
            'files': {k: hashlib.sha256(v.encode()).hexdigest() for k, v in sources.items()},
            'scope': 'all tracked code at pinned revision; no ignored/untracked native bindings',
            'license': 'MIT; original notices preserved in source; see LICENSE'}
(out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8', newline='\n')
(out / 'LICENSE').write_bytes(git('show', f'{REVISION}:LICENSE'))
print(f'Frozen {len(sources)} files, {len(corpus.encode())} bytes at {REVISION}')
