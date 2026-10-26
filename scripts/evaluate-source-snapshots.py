"""Before/after quality and cost measurements on isolated, byte-verified caches.

Use an unchanged pre-refactor executable for 'before'. No model downloads or
global installations. Corpus timing is existing-index sync, not cold indexing.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from retrieval_fixture import workspace, assert_restored
from retrieval_summary import metrics

p = argparse.ArgumentParser()
p.add_argument('--stage', choices=['before', 'after'], required=True)
p.add_argument('--binary', required=True)
p.add_argument('--seed-project', help='Existing frozen full-profile 4030-chunk corpus index; before only')
p.add_argument('--model-dir', required=True)
p.add_argument('--root', default='target/source-snapshot-evaluation')
a = p.parse_args()
binary = Path(a.binary).resolve()
model = Path(a.model_dir).resolve()
root = Path(a.root)
out = root / a.stage
out.mkdir(parents=True, exist_ok=False)
dataset = Path('tests/fixtures/retrieval_bkndb')
manifest = json.loads((dataset/'manifest.json').read_text(encoding='utf-8'))
raw = (dataset/'corpus.json').read_bytes()
assert hashlib.sha256(raw).hexdigest() == manifest['corpus_sha256']
corpus = json.loads(raw)
project = workspace(out, corpus, manifest)
seed = Path(a.seed_project) if a.stage == 'before' else root/'before/workspace'
assert_restored(seed, corpus)
shutil.copytree(seed/'.code-rcl', project/'.code-rcl')
queries = json.loads((dataset/'queries.json').read_text(encoding='utf-8'))['queries']
queries += json.loads((dataset/'container-queries.json').read_text(encoding='utf-8'))['queries']

def peak_bytes(child):
    if os.name == 'nt':
        class Counters(ctypes.Structure):
            _fields_ = [('cb', ctypes.c_ulong), ('faults', ctypes.c_ulong)] + [(n, ctypes.c_size_t) for n in ['peak', 'working', 'paged_peak', 'paged', 'nonpaged_peak', 'nonpaged', 'pagefile', 'pagefile_peak']]
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        fn = ctypes.WinDLL('psapi').GetProcessMemoryInfo
        fn.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_ulong]
        if fn(int(child._handle), ctypes.byref(counters), counters.cb):
            return counters.peak
    return None

operations = []

def cli(label, target, *args):
    started = time.perf_counter()
    with (out/'cli-stderr.log').open('a', encoding='utf-8') as log:
        child = subprocess.Popen([str(binary), *args, '--json', '--project', str(target)], stdout=subprocess.PIPE, stderr=log, text=True, encoding='utf-8')
        peak = None
        while True:
            sample = peak_bytes(child)
            if sample is not None:
                peak = max(peak or 0, sample)
            try:
                stdout, _ = child.communicate(timeout=.05)
                break
            except subprocess.TimeoutExpired:
                pass
    result = json.loads(stdout)
    operations.append({'scenario': label, 'elapsed_ms': (time.perf_counter()-started)*1000, 'peak_working_set_bytes': peak, 'exit_code': child.returncode, 'result': result})
    (out/'operations.json').write_text(json.dumps(operations, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    assert child.returncode == 0, result
    return result

dbfile = project/'.code-rcl/cache.bkndb'
sizes = {'before_sync': dbfile.stat().st_size}
cli('corpus_source_sync', project, 'sync', '--no-report')
sizes['after_source_sync'] = dbfile.stat().st_size
for n in range(2):
    result = cli(f'corpus_noop_embedding_sync_{n}', project, 'embeddings', 'sync', '--progress', 'off')
    assert result['data']['embedded_chunks'] == 0, result
    assert result['data']['index']['indexed_chunks'] == 4030
sizes['after_embedding_sync'] = dbfile.stat().st_size

rankings = []
with (out/'mcp-stderr.log').open('w', encoding='utf-8') as log:
    child = subprocess.Popen([str(binary), 'mcp', '--project', str(project)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, encoding='utf-8')
    counter = 0
    def request(method, params):
        global counter
        counter += 1
        child.stdin.write(json.dumps({'jsonrpc':'2.0','id':counter,'method':method,'params':params})+'\n')
        child.stdin.flush()
        while True:
            line = child.stdout.readline()
            assert line, child.poll()
            response = json.loads(line)
            if response.get('id') == counter:
                assert 'error' not in response, response
                return response['result']
    try:
        request('initialize', {'protocolVersion':'2025-06-18','capabilities':{}})
        child.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n')
        child.stdin.flush()
        with (out/'rankings.jsonl').open('w', encoding='utf-8') as stream:
            for query in queries:
                for mode in ['lexical','semantic','hybrid']:
                    value = request('tools/call', {'name':'code_rcl_search','arguments':{'query':query['query'],'retrieval':mode,'limit':100,'max_output_kb':2048,'no_sync':True}})
                    assert not value['isError'], value
                    envelope = value['structuredContent']
                    data = envelope['data']
                    hits = data.get('matches') if mode != 'lexical' else data['exact_symbols'] or data['fuzzy_suggestions']
                    record = {'query':query,'mode':mode,'metrics':metrics(hits,query['labels']),'ranking':hits,'metadata':envelope['metadata']}
                    rankings.append(record)
                    stream.write(json.dumps(record,ensure_ascii=False)+'\n')
                    stream.flush()
                print(a.stage,query['id'],flush=True)
    finally:
        child.stdin.close()
        try:
            assert child.wait(timeout=30) == 0
        finally:
            if child.poll() is None:
                child.kill(); child.wait()
assert_restored(project, corpus)

# A small cold fixture measures real inference for both schedulers without
# re-embedding the full corpus. Exact bytes include CRLF and Unicode.
for batching in ['file','length']:
    fixture = (out/f'fixture-{batching}').resolve()
    (fixture/'.code-rcl').mkdir(parents=True)
    (fixture/'.code-rcl/config.toml').write_text('[embeddings]\nmodel_dir='+json.dumps(str(model))+'\nbatching='+json.dumps(batching)+'\n', encoding='utf-8')
    sources = {'lib.rs':'// outside symbol 日本語\r\n/// doc\r\npub fn cancel_request() { let value = 123; }\r\n',
               'tool.py':'# outside symbol 日本語\ndef cancel_request():\n    return 123\n',
               'tool.go':'package app\n// 日本語\nfunc cancelRequest() int { return 123 }\n',
               'tool.kt':'// 日本語\nfun cancelRequest(): Int { return 123 }\n'}
    for path,text in sources.items(): (fixture/path).write_bytes(text.encode())
    cli(f'fixture_{batching}_cold', fixture, 'embeddings','sync','--progress','off')
    noop = cli(f'fixture_{batching}_noop', fixture, 'embeddings','sync','--progress','off')
    assert noop['data']['embedded_chunks'] == 0
    for path,text in sources.items(): assert (fixture/path).read_bytes() == text.encode()

summary = {'stage':a.stage,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'corpus_sha256':manifest['corpus_sha256'],'source_bytes_verified':True,'query_count':len(queries),'mode_runs':len(rankings),'database_sizes_bytes':sizes,'corpus_indexing':'existing index; cold indexing measured only on four-file fixture','operations':operations,'latency_comparison':'single local observations, not speed guarantees','rankings_sha256':hashlib.sha256((out/'rankings.jsonl').read_bytes()).hexdigest()}
if a.stage == 'after':
    old = [json.loads(line) for line in (root/'before/rankings.jsonl').read_text(encoding='utf-8').splitlines()]
    assert len(old) == len(rankings) == 336
    changes = []
    for previous,current in zip(old,rankings):
        assert (previous['query'],previous['mode']) == (current['query'],current['mode'])
        identity = lambda r: [(h.get('path'),h.get('name'),h.get('kind'),h.get('line'),h.get('symbol_key')) for h in r['ranking']]
        if previous['metrics'] != current['metrics'] or identity(previous) != identity(current):
            changes.append({'query':current['query']['id'],'mode':current['mode'],'before':previous['metrics'],'after':current['metrics']})
    summary['changed_rankings'] = changes
    summary['gate_passed'] = not changes
(out/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
assert summary.get('gate_passed',True), summary.get('changed_rankings')
print('Completed',a.stage,len(rankings),'mode runs',flush=True)
