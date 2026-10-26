"""Byte-preserving frozen fixtures, including files committed with CRLF."""
import hashlib
import json
from pathlib import Path

def materialize(project, corpus, manifest):
    project=Path(project).resolve()
    for path, source in corpus.items():
        data=source.encode('utf-8')
        assert hashlib.sha256(data).hexdigest()==manifest['files'][path],path
        destination=project/path
        assert destination.resolve().is_relative_to(project),path
        destination.parent.mkdir(parents=True,exist_ok=True)
        destination.write_bytes(data)

def assert_restored(project, corpus):
    for path,source in corpus.items():
        assert (Path(project)/path).read_bytes()==source.encode('utf-8'),f'source bytes changed: {path}'

def workspace(output, corpus, manifest, reuse=False):
    project=Path(output)/'workspace'
    marker=project/'.evaluation-fixture.json'
    if project.exists():
        assert reuse,'workspace already exists; choose a new output or explicit --reuse-workspace'
        assert marker.is_file(),'refusing to reuse a directory without an evaluation marker'
        assert json.loads(marker.read_text(encoding='utf-8'))['corpus_sha256']==manifest['corpus_sha256'],'different corpus snapshot'
        assert_restored(project,corpus)
    else:
        project.mkdir(parents=True)
        materialize(project,corpus,manifest)
        marker.write_text(json.dumps({'corpus_sha256':manifest['corpus_sha256']}),encoding='utf-8')
    return project
