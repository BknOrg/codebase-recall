"""Recover summaries from saved rankings; preserve the original validation outcome."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import statistics

def metrics(hits,labels):
    unique={}
    for hit in hits:
        unique.setdefault((hit.get('path'),hit.get('name')),hit)
    hits=list(unique.values())
    grades={(v['path'],v['name']):v['grade'] for v in labels}
    relevance=lambda hit:grades.get((hit.get('path'),hit.get('name')),0)
    ranks=[i+1 for i,h in enumerate(hits) if relevance(h)==3]
    dcg=sum((2**relevance(h)-1)/math.log2(i+2) for i,h in enumerate(hits[:5]))
    ideal=sum((2**g-1)/math.log2(i+2) for i,g in enumerate(sorted(grades.values(),reverse=True)[:5]))
    return {'hit1':int(bool(ranks and min(ranks)==1)),'hit5':int(bool(ranks and min(ranks)<=5)),'mrr':1/min(ranks) if ranks else 0,'ndcg5':dcg/ideal if ideal else 0}

def summarize(records,manifest,experiment,validation,version,sha):
    summary={'manifest':manifest,'experiment':experiment,'validation':validation,'binary_version':version,'binary_sha256':sha,'query_freshness':'warm no_sync queries against frozen isolated fixture; source-sync cost recorded separately','groups':{}}
    for split in ['tuning','evaluation']:
        for mode in ['lexical','semantic','hybrid']:
            for dimension,values in [('all',['all']),('query_language',['en','id']),('kind',['identifier','behavior','mixed']),('intent',['implementation','configuration','wrapper','test'])]:
                for label in values:
                    selected=[r for r in records if r['query']['split']==split and r['mode']==mode and (dimension=='all' or r['query'][dimension]==label)]
                    if not selected:
                        continue
                    times=sorted(r['elapsed_ms'] for r in selected)
                    summary['groups'][f'{split}/{mode}/{dimension}/{label}']={'count':len(selected),**{k:statistics.mean(metrics(r['ranking'],r['query']['labels'])[k] for r in selected) for k in ['hit1','hit5','mrr','ndcg5']},'median_ms':statistics.median(times),'p95_ms':times[max(0,math.ceil(.95*len(times))-1)]}
    summary['completed_query_count']=len({r['query']['id'] for r in records})
    summary['completed_mode_runs']=len(records)
    return summary

if __name__=='__main__':
    p=argparse.ArgumentParser()
    p.add_argument('--results',required=True)
    p.add_argument('--dataset',default='tests/fixtures/retrieval_bkndb')
    p.add_argument('--binary-version',default='codebase-recall 0.10.0')
    p.add_argument('--validation-note',required=True,help='Original postcheck outcome; never imply a failed check passed')
    a=p.parse_args()
    root=Path(a.results)
    records=json.loads((root/'rankings.json').read_text(encoding='utf-8'))
    manifest=json.loads((Path(a.dataset)/'manifest.json').read_text(encoding='utf-8'))
    sha=(root/'binary.sha256').read_text().strip()
    result=summarize(records,manifest,'baseline',a.validation_note,a.binary_version,sha)
    (root/'summary.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f"Recovered {result['completed_mode_runs']} mode runs / {result['completed_query_count']} queries; validation note retained")
