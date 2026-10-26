import hashlib
import gzip
import math
import json
from pathlib import Path
import tempfile
import unittest
import importlib.util
from retrieval_fixture import materialize, assert_restored, workspace
from retrieval_summary import metrics, summarize

class RetrievalHelpers(unittest.TestCase):
    def test_indexing_vector_gate_rejects_changed_inputs_and_nonfinite_values(self):
        spec=importlib.util.spec_from_file_location('indexing_benchmark',Path(__file__).parent/'benchmark-indexing.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        import struct
        with tempfile.TemporaryDirectory() as temp:
            left,right=Path(temp)/'left',Path(temp)/'right'
            for directory in [left,right]:
                directory.mkdir()
                (directory/'inputs.json').write_text('[{"input_hash":"same"}]',encoding='utf-8')
                (directory/'vectors.f32').write_bytes(struct.pack('<768f',1.,*([0.]*767)))
            self.assertTrue(module.vector_comparison(left,right)['cosine_passed'])
            (right/'inputs.json').write_text('[{"input_hash":"different"}]',encoding='utf-8')
            with self.assertRaises(AssertionError):module.vector_comparison(left,right)
            (right/'inputs.json').write_bytes((left/'inputs.json').read_bytes())
            (right/'vectors.f32').write_bytes(struct.pack('<768f',float('nan'),*([0.]*767)))
            with self.assertRaises(AssertionError):module.vector_comparison(left,right)
    def test_full_corpus_bytes_survive_edit_restore_and_crlf(self):
        root=Path(__file__).resolve().parent.parent/'tests/fixtures/retrieval_bkndb'
        corpus=json.loads((root/'corpus.json').read_text(encoding='utf-8'))
        manifest=json.loads((root/'manifest.json').read_text(encoding='utf-8'))
        with tempfile.TemporaryDirectory() as temp:
            materialize(temp,corpus,manifest)
            assert_restored(temp,corpus)
            crlf=[p for p,v in corpus.items() if '\r\n' in v]
            self.assertTrue(crlf,'fixture must exercise the historical postcheck bug')
            path=Path(temp)/crlf[0]
            self.assertNotEqual(path.read_text(encoding='utf-8'),corpus[crlf[0]])
            path.write_bytes(b'changed')
            with self.assertRaises(AssertionError): assert_restored(temp,corpus)
            path.write_bytes(corpus[crlf[0]].encode('utf-8'))
            assert_restored(temp,corpus)
    def test_saved_rankings_match_manifest_and_query_ids(self):
        root=Path(__file__).resolve().parent.parent/'tests/fixtures/retrieval_bkndb'
        baseline=json.loads((root/'baseline.json').read_text(encoding='utf-8'))
        payload=(root/baseline['ranking_artifact']['file']).read_bytes()
        self.assertEqual(hashlib.sha256(payload).hexdigest(),baseline['ranking_artifact']['sha256'])
        rankings=json.loads(gzip.decompress(payload))
        queries=json.loads((root/'queries.json').read_text(encoding='utf-8'))['queries']
        ids={q['id'] for q in queries}
        self.assertEqual(len(rankings),240)
        self.assertEqual({r['query_id'] for r in rankings},ids)
        for record in rankings:
            self.assertLessEqual(len(record['ranking']),100)
            for hit in record['ranking']:
                for field in ['cosine_score','rrf_score']:
                    if hit.get(field) is not None:self.assertTrue(math.isfinite(hit[field]))
    def test_workspace_reuse_is_explicit_and_hash_guarded(self):
        corpus={'lib.rs':'fn main() {}\r\n'}
        manifest={'files':{'lib.rs':hashlib.sha256(corpus['lib.rs'].encode()).hexdigest()},'corpus_sha256':'known'}
        with tempfile.TemporaryDirectory() as temp:
            project=workspace(temp,corpus,manifest)
            with self.assertRaises(AssertionError): workspace(temp,corpus,manifest)
            self.assertEqual(workspace(temp,corpus,manifest,True),project)
            (project/'lib.rs').write_bytes(b'changed')
            with self.assertRaises(AssertionError): workspace(temp,corpus,manifest,True)
    def test_duplicate_chunks_do_not_inflate_graded_metrics(self):
        hit={'path':'a.rs','name':'a'}
        labels=[{'path':'a.rs','name':'a','grade':3}]
        result=metrics([hit,hit],labels)
        self.assertEqual(result['ndcg5'],1)
        self.assertEqual(result['mrr'],1)
        record={'query':{'id':'q1','split':'evaluation','query_language':'id','kind':'behavior','intent':'implementation','labels':labels},'mode':'semantic','elapsed_ms':1,'ranking':[hit]}
        summary=summarize([record],{},'baseline','postcheck failed','0.10.0','sha')
        self.assertEqual(summary['validation'],'postcheck failed')
        self.assertEqual(summary['completed_query_count'],1)

if __name__=='__main__': unittest.main()
