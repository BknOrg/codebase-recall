"""Compare two local CLI builds on independently labelled fixture edges.

Only labelled positive/negative pairs enter precision/recall. Unlabelled edges
are deliberately not treated as ground truth. Does not need embedding models.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def measure(binary, fixture, work, flags=(), reuse=False):
    if not reuse:
        shutil.copytree(fixture, work, ignore=shutil.ignore_patterns('.code-rcl', '.git', 'target'))
    started = time.perf_counter()
    result = subprocess.run([
        str(binary), 'graph', '--project', str(work), '--format', 'json', '-o', '-',
        '--min-confidence', '0', '--max-nodes', '0',
    ] + list(flags), capture_output=True, text=True, encoding='utf-8', check=True)
    elapsed_ms = (time.perf_counter() - started) * 1000
    envelope = json.loads(result.stdout)
    graph = envelope['data']
    nodes = {n['id']: n for n in graph['nodes']}
    calls = {}
    for edge in graph['edges']:
        if edge['kind'] != 'calls':
            continue
        a, b = nodes[edge['source']], nodes[edge['target']]
        key = (a.get('path'), a['label'], b.get('path'), b['label'])
        calls[key] = edge
        if b.get("lines"):
            calls[key + (b["lines"][0],)] = edge
    labels = json.loads((fixture / 'expected-edges.json').read_text(encoding='utf-8'))
    positive = [tuple(e) for e in labels.get('must_have', [])]
    negative = [tuple(e) for e in labels.get('must_not_have', [])]
    positive += [tuple(e) for e in labels.get('must_have_target_lines', [])]
    negative += [tuple(e) for e in labels.get('must_not_target_lines', [])]
    hits = sum(e in calls for e in positive)
    wrong = [calls[e] for e in negative if e in calls]
    return {
        'status': envelope.get('status'),
        'wall_ms': round(elapsed_ms, 3),
        'coverage': envelope.get('metadata', {}).get('precision'),
        'timings_ms': envelope.get('metadata', {}).get('timings_ms'),
        'resolution_mode': envelope.get('metadata', {}).get('resolution_mode'),
        'positive_hits': hits, 'positive_labels': len(positive),
        'negative_labels': len(negative), 'forbidden_edges': len(wrong),
        'forbidden_strong_edges': sum(e['confidence'] > .350001 for e in wrong),
        'labelled_precision': hits / (hits + len(wrong)) if hits + len(wrong) else None,
        'labelled_recall': hits / len(positive) if positive else None,
        'ambiguous_edges': sum(bool(e.get('resolution', {}).get('ambiguous')) for e in graph['edges']),
        'unresolved_references': graph.get('resolution_summary', {}).get('unresolved'),
        'languages': sorted({n['language'] for n in graph['nodes'] if n.get('language')}),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('before', type=Path)
    parser.add_argument('after', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    output = {'scope': 'independently labelled fixture pairs, not exhaustive project precision', 'fixtures': {}}
    # All generated state lives in the explicitly chosen output directory.
    with tempfile.TemporaryDirectory(prefix='resolution-', dir=args.output.parent.resolve()) as tmp:
        root = Path(tmp).resolve()
        assert root.is_relative_to(args.output.parent.resolve())
        for labels in sorted((repo / 'tests/fixtures').glob('*/expected-edges.json')):
            fixture = labels.parent
            output['fixtures'][fixture.name] = {
                name: measure(binary.resolve(), fixture, root / (fixture.name + '-' + name))
                for name, binary in [('before', args.before), ('after', args.after)]
            }
    args.output.write_text(json.dumps(output, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(output, indent=2))


if __name__ == '__main__':
    main()
