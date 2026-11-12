"""Compare heuristic/selective/full on the same labelled corpus, without installs.

Real LSPs are deliberately opt-in: pass --run-lsp. Prepare executables and build
context yourself. Failed strict runs are reported as errors, never as measurements
of successful verification. Metrics only cover explicitly labelled target pairs.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
from compare_resolution import measure


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('fixture', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--run-lsp', action='store_true')
    args = parser.parse_args()
    if not args.run_lsp:
        parser.error('explicit --run-lsp is required; no server is installed automatically')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    result = {'scope': 'labelled target pairs only; wall times include CLI startup and local analysis', 'modes': {}}
    with tempfile.TemporaryDirectory(prefix='precise-modes-', dir=args.output.parent.resolve()) as temp:
        for mode, flags in [('heuristic', []), ('selective', ['--precise-selective']), ('full', ['--precise'])]:
            work = Path(temp) / mode
            measurements = {}
            for phase in ['cold', 'warm']:
                try:
                    metrics = measure(args.binary.resolve(), args.fixture.resolve(), work, flags, reuse=phase=='warm')
                    languages = (metrics.get('coverage') or {}).get('languages', [])
                    metrics['query_count'] = sum(v.get('queried', 0) for v in languages)
                    metrics['server_starts'] = sum(v.get('server_starts', 0) for v in languages)
                    measurements[phase] = metrics
                except subprocess.CalledProcessError as error:
                    measurements[phase] = {'status': 'error', 'stderr': error.stderr, 'exit_code': error.returncode}
                    break
            result['modes'][mode] = measurements
    args.output.write_text(json.dumps(result, indent=2)+'\n', encoding='utf-8')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
