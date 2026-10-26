"""Offline ranking experiments reuse saved semantic candidates, not model inference."""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import time
from retrieval_fixture import workspace, assert_restored
from retrieval_summary import metrics, summarize

p=argparse.ArgumentParser()
p.add_argument('--baseline',required=True)
p.add_argument('--experiment',choices=['overlap','identifier_tokens'],required=True)
p.add_argument('--binary',help='Required only for lexical token expansion; never loads a model')
p.add_argument('--dataset',default='tests/fixtures/retrieval_bkndb')
p.add_argument('--output',required=True)
a=p.parse_args()
baseline=Path(a.baseline)
out=Path(a.output)
out.mkdir(parents=True,exist_ok=True)
records=json.loads((baseline/'rankings.json').read_text(encoding='utf-8'))
original=json.loads((baseline/'summary.json').read_text(encoding='utf-8'))
root=Path(a.dataset)
corpus=json.loads((root/'corpus.json').read_text(encoding='utf-8'))
manifest=json.loads((root/'manifest.json').read_text(encoding='utf-8'))
lookup={(r['query']['id'],r['mode']):r for r in records}
term_cache={}
project=None
if a.experiment=='identifier_tokens':
    assert a.binary,'--binary required for lexical expansion'
    binary=str(Path(a.binary).resolve())
    project=workspace(out,corpus,manifest,reuse=(out/'workspace').exists())
    run=subprocess.run([binary,'search','pack_vector','--json','--project',str(project)],capture_output=True,text=True,encoding='utf-8')
    assert run.returncode==0,run.stderr

def tokens(text):
    text=re.sub(r'([a-z])([A-Z])',r'\1 \2',text).replace('_',' ')
    stop={'the','and','with','from','into','when','where','that','di','dan','yang','untuk','of','are','how','for','in','ke','dari','pada','saat','atau'}
    return [v for v in dict.fromkeys(re.findall(r'[a-zA-Z][a-zA-Z0-9]*',text)) if len(v)>2 and v.lower() not in stop][:8]

def key(hit):
    line=hit.get('line')
    if hit.get('symbol_key'):
        start=int(hit['symbol_key'].rsplit(':',2)[1])
        line=corpus[hit['path']].encode()[:start].count(b'\n')+1
    return hit.get('path'),hit.get('name'),line or 0

def lexical(term):
    if term not in term_cache:
        start=time.perf_counter()
        run=subprocess.run([binary,'search',term,'--no-sync','--no-fuzzy','--no-grep','--limit','100','--json','--project',str(project)],capture_output=True,text=True,encoding='utf-8')
        assert run.returncode==0,run.stderr
        term_cache[term]={'hits':json.loads(run.stdout)['data']['exact_symbols'],'elapsed_ms':(time.perf_counter()-start)*1000}
    return term_cache[term]['hits']

def suppress_overlap(hits):
    result=[]
    for i,hit in enumerate(hits):
        suppressed=False
        if hit.get('kind') in {'impl','class','struct'}:
            for member in hits[max(0,i-5):i+6]:
                if member.get('kind') in {'function','method'} and member.get('path')==hit.get('path'):
                    start,end=member.get('start_byte',0),member.get('end_byte',0)
                    overlap=max(0,min(end,hit.get('end_byte',0))-max(start,hit.get('start_byte',0)))
                    if end>start and overlap/(end-start)>=.8:
                        suppressed=True
                        break
        if not suppressed: result.append(hit)
    return result

updated=[]
for record in records:
    record={**record}
    start=time.perf_counter()
    if a.experiment=='overlap' and record['mode']!='lexical':
        record['ranking']=suppress_overlap(record['ranking'])
    elif a.experiment=='identifier_tokens' and record['mode']=='hybrid':
        full=lookup[record['query']['id'],'lexical']['ranking']
        counts=Counter()
        candidates={}
        for term in tokens(record['query']['query']):
            for hit in lexical(term):
                identity=key(hit);counts[identity]+=1;candidates[identity]=hit
        full_keys={key(hit) for hit in full}
        expanded=(full+[candidates[k] for k in sorted(candidates,key=lambda k:(-counts[k],k)) if k not in full_keys])[:100]
        semantic=lookup[record['query']['id'],'semantic']['ranking']
        scores={};candidates={}
        for branch in [semantic,expanded]:
            for rank,hit in enumerate(branch[:100]):
                identity=key(hit);scores[identity]=scores.get(identity,0)+1/(60+rank+1)
                candidates.setdefault(identity,hit)
        record['ranking']=[{**candidates[k],'rrf_score':scores[k]} for k in sorted(scores,key=lambda k:(-scores[k],k))]
    record['experiment_ms']=(time.perf_counter()-start)*1000
    record['metrics']=metrics(record['ranking'],record['query']['labels'])
    updated.append(record)

if project: assert_restored(project,corpus)
summary=summarize(updated,manifest,a.experiment,original['validation'],original['binary_version'],original['binary_sha256'])
summary['offline']=True
summary['native_inference_rerun']=False
summary['lexical_token_lookup_ms']=sum(v['elapsed_ms'] for v in term_cache.values())
summary['baseline_latency_is_not_experiment_latency']=True
changes=[]
for old,new in zip(records,updated):
    if old['metrics']!=new['metrics']:
        changes.append({'query':old['query']['id'],'split':old['query']['split'],'kind':old['query']['kind'],'mode':old['mode'],'before':old['metrics'],'after':new['metrics']})
summary['changed_queries']=changes
summary['production_enabled']=False
(out/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
(out/'rankings.json').write_text(json.dumps(updated,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
(out/'token-lookups.json').write_text(json.dumps(term_cache,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f'Offline {a.experiment}: {len(changes)} changed mode/query results; production unchanged')
