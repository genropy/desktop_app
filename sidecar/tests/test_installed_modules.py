"""The sidecar loads a downloaded module from its folder, after its own modules."""
import json
import subprocess
import sys
from pathlib import Path

MAIN = Path(__file__).resolve().parent.parent / 'main.py'

SCALE = '''
from registry import module, method


@module('scale')
class Scale(object):

    def __init__(self, sidecar):
        self.sidecar = sidecar

    @method
    def weight(self, unit='g'):
        return {'value': 1250, 'unit': unit}
'''


def package(root, name, source):
    "A module folder as the shell writes it: <root>/<name>/<version>/<name>/__init__.py."
    folder = root / name / '1.0.0'
    (folder / name).mkdir(parents=True)
    (folder / name / '__init__.py').write_text(source)
    return folder


def run(modules, requests):
    "Starts the sidecar with `modules`, sends `requests`, returns its stdout lines and stderr."
    args = [sys.executable, str(MAIN)]
    for name, folder in modules:
        args += ['--module', name, str(folder)]
    stdin = ''.join(json.dumps(request) + '\n' for request in requests)
    done = subprocess.run(args, input=stdin, capture_output=True, text=True, timeout=30)
    return [json.loads(line) for line in done.stdout.splitlines()], done.stderr


def test_a_module_in_a_folder_is_loaded_and_answers(tmp_path):
    folder = package(tmp_path, 'scale', SCALE)
    lines, _ = run([('scale', folder)],
                   [{'id': 1, 'module': 'scale', 'method': 'weight', 'params': {'unit': 'kg'}}])
    assert lines[0]['modules']['scale'] == ['weight']
    assert 'system' in lines[0]['modules']
    assert lines[1] == {'id': 1, 'result': {'value': 1250, 'unit': 'kg'}}
    assert not list(folder.rglob('__pycache__'))


def test_a_package_that_registers_another_name_is_dropped(tmp_path):
    folder = package(tmp_path, 'scale', SCALE.replace("@module('scale')", "@module('reader')"))
    lines, stderr = run([('scale', folder)], [])
    assert 'reader' not in lines[0]['modules'] and 'scale' not in lines[0]['modules']
    assert 'module scale not loaded' in stderr


def test_a_package_cannot_replace_a_module_of_the_sidecar(tmp_path):
    folder = package(tmp_path, 'scale', SCALE.replace("@module('scale')", "@module('system')"))
    lines, stderr = run([('scale', folder)],
                        [{'id': 1, 'module': 'system', 'method': 'ping', 'params': {}}])
    assert lines[1] == {'id': 1, 'result': 'pong'}
    assert 'already registered' in stderr


def test_a_name_of_the_standard_library_is_not_taken_from_the_folder(tmp_path):
    folder = package(tmp_path, 'json', SCALE.replace("'scale'", "'json'"))
    lines, stderr = run([('json', folder)], [])
    assert 'json' not in lines[0]['modules']
    assert 'not from' in stderr
