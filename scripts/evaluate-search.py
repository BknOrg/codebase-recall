"""Frozen-corpus baseline, tuning selection and release gates for automatic search.

Native model runs use the ignored Rust search_pipeline_evaluation test. This
script never downloads a model, changes query labels or mutates the seed index.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

from retrieval_fixture import assert_restored, workspace
from retrieval_summary import metrics


def read_json(path):
    return json.loads(Path(path).read_text(encoding='utf-8'))


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def records(root):
    root = Path(root)
    completed = read_json(root / 'completed.json')
    assert completed['source_bytes_verified'], 'source postcheck did not pass'
    if (root / 'validation.json').exists():
        validation = read_json(root / 'validation.json')
        assert validation.get('valid_for_release', False) and not validation.get('invalid_for_release', False), 'run invalidated; retain it for diagnosis only'
    rows = [json.loads(line) for line in (root / 'rankings.jsonl').read_text(encoding='utf-8').splitlines()]
    for row in rows:
        row.setdefault('metrics', metrics(row['ranking'], row['query']['labels']))
        if 'trace' in row:
            gold = {(label['path'], label['name']) for label in row['query']['labels'] if label['grade'] == 3}
            row['candidate_hit'] = any((hit['path'], hit['name']) in gold for hit in row['trace']['candidates'])
    return rows


def p95(values):
    values = sorted(values)
    return values[max(0, math.ceil(.95 * len(values)) - 1)]


def scored_hit(row):
    gold = {(label['path'], label['name']) for label in row['query']['labels'] if label['grade'] == 3}
    return any((hit['path'], hit['name']) in gold for hit in row['trace']['scored'])


def group(rows):
    return {'count': len(rows), **{k: statistics.mean(r['metrics'][k] for r in rows)
                                 for k in ['hit1', 'hit5', 'mrr', 'ndcg5']},
            'candidate_hit_rate': statistics.mean(r['candidate_hit'] for r in rows)
                                  if all('candidate_hit' in r for r in rows) else None,
            'scored_candidate_hit_rate': statistics.mean(scored_hit(r) for r in rows)
                                         if all('trace' in r for r in rows) else None,
            'median_ms': statistics.median(r['elapsed_ms'] for r in rows),
            'p95_ms': p95([r['elapsed_ms'] for r in rows])}


def baseline(args):
    binary = Path(args.binary).resolve(strict=True)
    fingerprint = hashlib.sha256(binary.read_bytes()).hexdigest()
    seed = Path(args.seed_project).resolve(strict=True)
    out = Path(args.output).resolve()
    assert not out.is_relative_to(seed), 'output must be outside the seed'
    out.mkdir(parents=True, exist_ok=False)
    dataset = Path('tests/fixtures/retrieval_bkndb')
    raw = (dataset / 'corpus.json').read_bytes()
    corpus, manifest = json.loads(raw), read_json(dataset / 'manifest.json')
    assert hashlib.sha256(raw).hexdigest() == manifest['corpus_sha256']
    assert_restored(seed, corpus)
    project = workspace(out, corpus, manifest).resolve()
    (project / '.code-rcl').mkdir()
    for name in ['cache.bkndb', 'config.toml']:
        shutil.copy2(seed / '.code-rcl' / name, project / '.code-rcl' / name)
    queries = []
    if args.query_file:
        queries = read_json(args.query_file)['queries']
    else:
        for name in ['queries.json', 'container-queries.json', 'negative-queries.json']:
            queries += read_json(dataset / name)['queries']
    sync = subprocess.run([str(binary), 'sync', '--no-report', '--json', '--project', str(project)],
                          capture_output=True, text=True, encoding='utf-8', timeout=180)
    save(out / 'sync.json', {'exit_code': sync.returncode, 'stdout': sync.stdout, 'stderr': sync.stderr})
    assert sync.returncode == 0, sync.stderr
    counter = 0
    with (out / 'stderr.log').open('w', encoding='utf-8') as log:
        child = subprocess.Popen([str(binary), 'mcp', '--project', str(project)], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=log, text=True, encoding='utf-8')

        def request(method, params):
            nonlocal counter
            counter += 1
            child.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': counter, 'method': method, 'params': params}) + '\n')
            child.stdin.flush()
            while True:
                line = child.stdout.readline()
                assert line, f'MCP exited: {child.poll()}'
                value = json.loads(line)
                if value.get('id') == counter:
                    assert 'error' not in value, value
                    return value['result']

        try:
            request('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}})
            child.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
            child.stdin.flush()
            with (out / 'rankings.jsonl').open('x', encoding='utf-8') as stream:
                for q in queries:
                    for mode in ['lexical', 'semantic', 'hybrid']:
                        started = time.perf_counter()
                        result = request('tools/call', {'name': 'code_rcl_search', 'arguments': {
                            'query': q['query'], 'retrieval': mode, 'limit': 25,
                            'no_sync': True, 'max_output_kb': 2048}})
                        elapsed = (time.perf_counter() - started) * 1000
                        assert not result['isError'], result
                        envelope = result['structuredContent']
                        assert envelope['metadata']['embeddings']['complete'], envelope['metadata']
                        data = envelope['data']
                        hits = data['matches'] if mode != 'lexical' else data['exact_symbols'] or data['fuzzy_suggestions']
                        record = {'query': q, 'mode': mode, 'elapsed_ms': elapsed, 'ranking': hits,
                                  'metrics': metrics(hits, q['labels']), 'metadata': envelope['metadata']}
                        stream.write(json.dumps(record, ensure_ascii=False) + '\n')
                        stream.flush()
                    print(q['id'], flush=True)
        finally:
            child.stdin.close()
            try:
                assert child.wait(timeout=30) == 0
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait()
    assert_restored(project, corpus)
    assert_restored(seed, corpus)
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == fingerprint
    save(out / 'completed.json', {'binary_sha256': fingerprint, 'queries': len(queries),
                                'source_bytes_verified': True, 'rank_limit': 25,
                                'corpus_sha256': manifest['corpus_sha256']})


def report(args):
    root = Path(args.evaluation)
    done = read_json(root / 'completed.json')
    rows = records(root)
    assert done['source_bytes_verified']
    assert done['rank_limit'] == 25, 'release gates require the default output limit'
    old = {(r['query']['id'], r['mode']): r for r in records(args.baseline)}
    groups, changes = {}, []
    for mode in ['semantic', 'hybrid']:
        for kind in ['all', 'behavior', 'container', 'identifier', 'mixed']:
            selected = [r for r in rows if r['mode'] == mode and not r['query'].get('absent')
                        and (kind == 'all' or r['query']['kind'] == kind)]
            if selected:
                groups[f'{mode}/{kind}'] = {'before': group([old[r['query']['id'], mode] for r in selected]),
                                          'after': group(selected)}
    for r in rows:
        before = old[r['query']['id'], r['mode']]
        if any(abs(before['metrics'][key] - r['metrics'][key]) > 1e-9
               for key in ['hit1', 'hit5', 'mrr', 'ndcg5']):
            changes.append({'id': r['query']['id'], 'kind': r['query']['kind'], 'language': r['query']['query_language'],
                            'mode': r['mode'], 'before': before['metrics'], 'after': r['metrics']})
    semantic = [r for r in rows if r['mode'] == 'semantic' and not r['query'].get('absent')]
    behavior = [r for r in semantic if r['query']['kind'] == 'behavior']
    containers = [r for r in semantic if r['query']['kind'] == 'container']
    total = lambda rs, key: sum(r['metrics'][key] for r in rs)
    warm_latency = {mode: {'count': len([r for r in rows if r['mode'] == mode]),
                           'median_ms': statistics.median(r['elapsed_ms'] for r in rows if r['mode'] == mode),
                           'p95_ms': p95([r['elapsed_ms'] for r in rows if r['mode'] == mode])}
                    for mode in ['semantic', 'hybrid']}
    gates = {'semantic_hit1': total(semantic, 'hit1') >= 26, 'semantic_hit5': total(semantic, 'hit5') >= 31,
             'behavior_hit1': total(behavior, 'hit1') >= 9, 'behavior_hit5': total(behavior, 'hit5') >= 11,
             'container_hit1': total(containers, 'hit1') >= 7, 'container_hit5': total(containers, 'hit5') >= 8,
             'warm_p95': all(value['p95_ms'] <= 5000 for value in warm_latency.values())}
    assert len(semantic) == 32 and len(behavior) == 12 and len(containers) == 8
    negative = [r for r in rows if r['query'].get('absent')]
    weak = {'negative_count': len(negative), 'negative_flagged_weak': sum(r['metadata']['retrieval']['quality'] == 'weak' for r in negative),
            'positive_count': len(rows) - len(negative), 'positive_flagged_weak': sum(r['metadata']['retrieval']['quality'] == 'weak' for r in rows if not r['query'].get('absent'))}
    language_groups = {}
    for mode in ['semantic', 'hybrid']:
        for language in sorted({r['query']['query_language'] for r in rows}):
            selected = [r for r in rows if r['mode'] == mode and not r['query'].get('absent')
                        and r['query']['query_language'] == language]
            if selected:
                language_groups[f'{mode}/{language}'] = {
                    'before': group([old[r['query']['id'], mode] for r in selected]), 'after': group(selected)}
    source_language_groups = {}
    suffixes = {'.rs': 'Rust', '.py': 'Python', '.go': 'Go', '.kt': 'Kotlin'}
    for mode in ['semantic', 'hybrid']:
        for suffix, language in suffixes.items():
            selected = [r for r in rows if r['mode'] == mode and not r['query'].get('absent')
                        and any(l['grade'] == 3 and l['path'].endswith(suffix) for l in r['query']['labels'])]
            if selected:
                source_language_groups[f'{mode}/{language}'] = {
                    'before': group([old[r['query']['id'], mode] for r in selected]), 'after': group(selected)}
    result = {'completed': done, 'gates': gates, 'quality_latency_gates_passed': all(gates.values()),
              'query_language_groups': language_groups,
              'source_language_groups': source_language_groups,
              'warm_latency_all_queries': warm_latency,
              'groups': groups, 'changed_queries': changes, 'weak_marker': weak,
              'promotion_allowed': False,
              'promotion_note': 'Also requires historical-suite and contract/CLI/MCP validation; never promote from this report alone.',
              'rankings_sha256': hashlib.sha256((root / 'rankings.jsonl').read_bytes()).hexdigest(),
              'judgments': 'seed judgments; unjudged code is not proven irrelevant; no corpus-wide recall claim'}
    save(root / 'report.json', result)
    print(json.dumps({'gates': gates, 'weak_marker': weak}, indent=2))


def native_run(args):
    harness = Path(args.harness).resolve(strict=True)
    output = Path(args.output).resolve()
    assert not output.exists(), 'use a new output directory'
    output.parent.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ)
    environment.update({'CODE_RCL_SEARCH_OUTPUT': str(output),
                        'CODE_RCL_SEARCH_SEED': str(Path(args.seed_project).resolve(strict=True)),
                        'CODE_RCL_SEARCH_CONTEXT_VECTORS': str(Path(args.context_vectors).resolve(strict=True)),
                        'CODE_RCL_SEARCH_SPLIT': args.split})
    if args.profile:
        environment['CODE_RCL_SEARCH_PROFILE'] = str(Path(args.profile).resolve(strict=True))
    started, peak = time.perf_counter(), None
    with output.with_suffix('.log').open('x', encoding='utf-8') as log:
        child = subprocess.Popen([str(harness), '--exact',
                                  'commands::search::retrieval::evaluation::search_pipeline_evaluation',
                                  '--ignored', '--nocapture'], env=environment, stdout=log, stderr=log)
        try:
            while True:
                if os.name == 'nt':
                    import ctypes

                    class Counters(ctypes.Structure):
                        _fields_ = [('cb', ctypes.c_ulong), ('faults', ctypes.c_ulong)] + [
                            (n, ctypes.c_size_t) for n in ['peak', 'working', 'paged_peak', 'paged',
                                                         'nonpaged_peak', 'nonpaged', 'pagefile', 'pagefile_peak']]
                    counters = Counters()
                    counters.cb = ctypes.sizeof(counters)
                    function = ctypes.WinDLL('psapi').GetProcessMemoryInfo
                    function.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_ulong]
                    if function(int(child._handle), ctypes.byref(counters), counters.cb):
                        peak = max(peak or 0, counters.peak)
                try:
                    code = child.wait(timeout=.25)
                    break
                except subprocess.TimeoutExpired:
                    pass
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
    save(output.with_suffix('.process.json'), {'exit_code': code, 'peak_working_set_bytes': peak,
                                             'elapsed_ms': (time.perf_counter() - started) * 1000,
                                             'test_executable_sha256': hashlib.sha256(harness.read_bytes()).hexdigest()})
    assert code == 0, f'native evaluation failed; inspect {output.with_suffix(".log")}'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    p = sub.add_parser('baseline')
    p.add_argument('--query-file')
    for name in ['binary', 'seed-project', 'output']:
        p.add_argument('--' + name, required=True)
    p = sub.add_parser('report'); p.add_argument('--evaluation', required=True); p.add_argument('--baseline', required=True)
    p = sub.add_parser('run')
    for name in ['harness', 'output', 'seed-project', 'context-vectors']:
        p.add_argument('--' + name, required=True)
    p.add_argument('--split', choices=['tuning', 'evaluation', 'fresh'], default='evaluation')
    p.add_argument('--profile')
    args = parser.parse_args()
    {'baseline': baseline, 'report': report, 'run': native_run}[args.action](args)


if __name__ == '__main__':
    main()
