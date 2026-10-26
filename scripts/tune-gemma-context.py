"""Select Gemma context aggregation on tuning queries; evaluate frozen profiles."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics
from retrieval_summary import metrics


def read(path):
    return json.loads(Path(path).read_text(encoding='utf-8'))


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')


CONTAINERS = {'class', 'struct', 'interface', 'trait', 'enum', 'impl', 'type'}


def blend(a, b, weight):
    if a is None:
        return b
    if b is None:
        return a
    return a*(1-weight)+b*weight


def ranking(symbols, row, profile, protect_identity=True):
    query = row['query']['query'].strip().strip('`')
    identity = protect_identity and bool(query) and all(c.isalnum() or c == '_' for c in query)
    weights = profile['ranking']
    ranked = []
    for symbol, evidence in zip(symbols, row['evidence'], strict=True):
        body = blend(evidence[0], evidence[1], weights['query_view_weight'])
        purpose = blend(evidence[2], evidence[3], weights.get('purpose_query_view_weight', weights['query_view_weight']))
        weight = weights['container_body_weight'] if symbol['kind'] in CONTAINERS else weights['function_body_weight']
        score = blend(purpose, body, weight)
        if score is not None:
            ranked.append((symbol, score))
    ranked.sort(key=lambda pair: (-int(identity and pair[0]['name'] == query), -pair[1],
        pair[0]['path'], int(pair[0]['symbol_key'].rsplit(':', 2)[1]), pair[0]['symbol_key']))
    return ranked[:25]


def summarize(symbols, rows, profile):
    values = [(r['query']['kind'], metrics([s for s, _ in ranking(symbols, r, profile)], r['query']['labels']))
              for r in rows if not r['query'].get('absent')]
    groups = {kind: {'count': sum(k == kind for k, _ in values),
                    **{key: sum(m[key] for k, m in values if k == kind) for key in ['hit1', 'hit5']},
                    'mrr': statistics.mean(m['mrr'] for k, m in values if k == kind)}
              for kind in {k for k, _ in values}}
    return {'hit1': sum(m['hit1'] for _, m in values), 'hit5': sum(m['hit5'] for _, m in values),
            'macro_mrr': statistics.mean(g['mrr'] for g in groups.values()), 'groups': groups}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True)
    parser.add_argument('--profile', help='Frozen profile; required for evaluation')
    args = parser.parse_args()
    root = Path(args.input)
    done = read(root/'completed.json')
    assert done['source_bytes_verified'] and done['base_vectors_unchanged']
    symbols = read(root/'symbols.json')
    rows = [json.loads(line) for line in (root/'scores.jsonl').read_text(encoding='utf-8').splitlines()]
    assert len(rows) == done['queries']
    if args.profile:
        profile = read(args.profile)
    else:
        assert done['split'] == 'tuning' and all(r['query']['split'] == 'tuning' for r in rows)
        grid = []
        for function in [0, .25, .5, .75, 1]:
            for container in [0, .25, .5, .75, 1]:
                for view in [0, .5, 1]:
                    for purpose_view in [0, .5, 1]:
                        profile = {'promoted': False, 'context_tokens': done['context_tokens'], 'ranking': {
                            'function_body_weight': function, 'container_body_weight': container, 'query_view_weight': view,
                            'purpose_query_view_weight': purpose_view}}
                        grid.append({'profile': profile, **summarize(symbols, rows, profile)})
        save(root/'grid.json', grid)
        selected = min(grid, key=lambda g: (-g['macro_mrr'], -g['hit5'], -g['hit1'], -g['profile']['ranking']['function_body_weight']))
        profile = selected['profile']
        positive, negative = [], []
        for row in rows:
            top = ranking(symbols, row, profile)
            (negative if row['query'].get('absent') else positive).append(top[0][1])
        allowed = [s for s in sorted(set(positive+negative)) if sum(v < s for v in positive) <= math.floor(.05*len(positive))]
        threshold = max(allowed, key=lambda s: (sum(v < s for v in negative), -sum(v < s for v in positive), s))
        profile['weak_threshold'] = threshold
        save(root/'profile.json', profile)
        save(root/'selection.json', {'selected': selected, 'uses_held_out_data': False,
            'calibration_positive_weak': sum(s < threshold for s in positive),
            'calibration_negative_weak': sum(s < threshold for s in negative)})
    summary = summarize(symbols, rows, profile)
    save(root/'summary.json', {'profile': profile, **summary,
        'scores_sha256': hashlib.sha256((root/'scores.jsonl').read_bytes()).hexdigest(),
        'judgments': 'Fixed labels; these measurements do not establish whole-codebase recall.'})
    print(json.dumps({'profile': profile, **summary}, indent=2))


if __name__ == '__main__':
    main()
