"""Package a release with notices/native libraries; test it from an extracted folder."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
import tomllib

p = argparse.ArgumentParser()
p.add_argument('--target', required=True)
p.add_argument('--extension', choices=['zip', 'tar.gz'], required=True)
p.add_argument('--build-dir')
p.add_argument('--output-dir', default='.')
p.add_argument('--model-dir')
a = p.parse_args()
build = Path(a.build_dir or f'target/{a.target}/release').resolve()
name = 'code-rcl.exe' if 'windows' in a.target else 'code-rcl'
expected_runtime = 'musl' not in a.target
out = Path(a.output_dir).resolve()
out.mkdir(parents=True, exist_ok=True)
archive = out / f'codebase-recall-{a.target}.{a.extension}'
with tempfile.TemporaryDirectory(prefix='code-rcl-release-') as temp:
    temp = Path(temp)
    stage = temp / 'stage'
    stage.mkdir()
    shutil.copy2(build / name, stage / name)
    for file in build.iterdir():
        if file.is_file() and (file.suffix in ['.dll', '.dylib', '.so'] or '.so.' in file.name):
            shutil.copy2(file, stage / file.name)
    for file in ['LICENSE', 'THIRD_PARTY.md']:
        shutil.copy2(file, stage / file)
    notices = stage / 'licenses'
    notices.mkdir()
    cargo_home = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')))
    dependencies = {f"{p['name']}-{p['version']}" for p in tomllib.loads(Path('Cargo.lock').read_text())['package']}
    for registry in (cargo_home / 'registry/src').glob('*'):
        for crate in registry.iterdir():
            if not crate.is_dir() or crate.name not in dependencies:
                continue
            for pattern in ['LICENSE*', 'COPYING*', 'NOTICE*', 'ThirdPartyNotices*']:
                for file in crate.glob(pattern):
                    if file.is_file():
                        dest = notices / crate.name
                        dest.mkdir(exist_ok=True)
                        shutil.copy2(file, dest / file.name)
    if Path('third_party').exists():
        shutil.copytree('third_party', notices / 'bundled', dirs_exist_ok=True)
    # Preserve notices in runtime archives referenced by the native build script.
    for output in (build / 'build').glob('ort-sys-*/output'):
        for line in output.read_text(encoding='utf-8', errors='replace').splitlines():
            prefix = 'cargo:rustc-link-search=native='
            if line.startswith(prefix):
                runtime = Path(line[len(prefix):])
                for file in runtime.iterdir() if runtime.is_dir() else []:
                    if file.is_file() and (file.suffix.lower() in ['.dll', '.dylib', '.so'] or '.so.' in file.name):
                        shutil.copy2(file, stage / file.name)
                for pattern in ['LICENSE*', 'ThirdPartyNotices*', 'NOTICE*']:
                    for file in runtime.rglob(pattern):
                        if file.is_file():
                            dest = notices / 'onnx-runtime'
                            dest.mkdir(exist_ok=True)
                            shutil.copy2(file, dest / file.name)
    if 'windows' in a.target:
        # App-local deployment uses only the release CRT redistributable directory.
        # Never collect debug_nonredist or DLLs from the system installation.
        candidates = []
        redist = os.environ.get('VCToolsRedistDir')
        if redist:
            candidates.extend(Path(redist).glob('x64/Microsoft.VC*.CRT'))
        for env in ['ProgramFiles', 'ProgramFiles(x86)']:
            root = os.environ.get(env)
            if root:
                candidates.extend(Path(root).glob('Microsoft Visual Studio/*/*/VC/Redist/MSVC/*/x64/Microsoft.VC*.CRT'))
        if not candidates:
            raise RuntimeError('Visual C++ release redistributable directory not found')
        crt = sorted(candidates, key=lambda path: path.stat().st_mtime)[-1]
        for file in crt.glob('*.dll'):
            shutil.copy2(file, stage / file.name)
        required = ['msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll']
        if expected_runtime:
            required.append('DirectML.dll')
        for file in required:
            if not (stage / file).is_file():
                raise RuntimeError(f'Missing redistributable dependency: {file}')
    # Preserve upstream notices delivered with native runtime build artifacts.
    for pattern in ['**/LICENSE*', '**/ThirdPartyNotices*', '**/NOTICE*']:
        for file in (build / 'build').glob(pattern):
            if file.is_file():
                dest = notices / 'native-runtime'
                dest.mkdir(exist_ok=True)
                shutil.copy2(file, dest / (hashlib.sha256(str(file).encode()).hexdigest()[:8] + '-' + file.name))
    if a.extension == 'zip':
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, strict_timestamps=False) as z:
            for file in stage.rglob('*'):
                if file.is_file():
                    z.write(file, file.relative_to(stage))
    else:
        with tarfile.open(archive, 'w:gz') as t:
            for file in stage.iterdir():
                t.add(file, arcname=file.name)
    unpacked = temp / 'extracted'
    unpacked.mkdir()
    if a.extension == 'zip':
        with zipfile.ZipFile(archive) as z:
            z.extractall(unpacked)
    else:
        with tarfile.open(archive) as t:
            t.extractall(unpacked, filter='data')
    binary = str(unpacked / name)
    subprocess.run([binary, '--version'], check=True)
    project = temp / 'project'
    project.mkdir()
    (project / 'lib.rs').write_text('pub fn cancel_request() {}\npub fn sort_names() {}\n', encoding='utf-8')
    def call(*args):
        run = subprocess.run([binary, *args, '--project', str(project), '--json'], check=True, text=True, encoding='utf-8', stdout=subprocess.PIPE)
        return json.loads(run.stdout)
    status = call('embeddings', 'status')
    assert status['data']['runtime_available'] == expected_runtime
    lexical = call('search', 'cancel_request')
    assert lexical['data']['exact_symbols'][0]['name'] == 'cancel_request'
    if expected_runtime:
        args = ['embeddings', 'setup']
        if a.model_dir:
            args += ['--model-dir', str(Path(a.model_dir).resolve())]
        call(*args)
        call('embeddings', 'sync')
        assert call('search', 'cancel request', '--semantic')['data']['matches']
    else:
        run = subprocess.run([binary, 'search', 'cancel', '--semantic', '--project', str(project)], stdout=subprocess.PIPE)
        assert run.returncode != 0
sha = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_name(archive.name + '.sha256').write_text(f'{sha}  {archive.name}\n', encoding='ascii')
print(f'Packaged and verified: {archive}')
