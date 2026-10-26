"""Explicit real-model CI runner; project fixtures stay in a temporary directory."""
import argparse
import json
import os
import sys
from pathlib import Path
import subprocess
import tempfile

p = argparse.ArgumentParser()
p.add_argument('--binary', default='target/debug/code-rcl.exe' if os.name == 'nt' else 'target/debug/code-rcl')
p.add_argument('--model-dir')
p.add_argument('--cargo-target-dir')
p.add_argument('--eval-retrieval',action='store_true')
a = p.parse_args()
binary = str(Path(a.binary).resolve())
log_path = Path('target/embedding-model-test.log')
log_path.parent.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='code-rcl-model-') as fixture, log_path.open('w', encoding='utf-8') as log:
    command = [binary, 'embeddings', 'setup', '--project', fixture, '--json']
    if a.model_dir:
        command += ['--model-dir', a.model_dir]
    setup = subprocess.run(command, check=True, text=True, encoding='utf-8', stdout=subprocess.PIPE)
    result = json.loads(setup.stdout)
    env = dict(os.environ, CODE_RCL_EMBEDDING_MODEL_DIR=result['data']['model_dir'])
    for suite in [['--bin', 'code-rcl', 'embedding_model_accuracy'], ['--bin', 'code-rcl', 'real_model_auto_and_explicit_effective_threads_share_runtime'], ['--test', 'embeddings_cli', 'real_model_setup_index_search_and_partial_update'], ['--test', 'mcp_cli', 'real_model_mcp_index_and_semantic_contract']]:
        command = ['cargo', 'test', '--locked']
        if a.cargo_target_dir:
            command += ['--target-dir', a.cargo_target_dir]
        command += suite + ['--', '--ignored', '--nocapture']
        run = subprocess.run(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='replace')
        print(run.stdout)
        log.write(run.stdout)
        log.flush()
        run.check_returncode()

    for batching in ['file','length']:
        run = subprocess.run([sys.executable, 'scripts/test-embedding-cancellation.py', '--binary', binary, '--model-dir', result['data']['model_dir'], '--batching', batching], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='replace')
        print(run.stdout)
        log.write(run.stdout)
        run.check_returncode()

    if a.eval_retrieval:
        subprocess.run([sys.executable, 'scripts/evaluate-retrieval.py', '--binary', binary, '--model-dir', result['data']['model_dir']], check=True, env=env)
