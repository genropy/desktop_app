"""Python sidecar of GenroApp.

The shell starts this process and talks to it with one JSON object per line:
requests on stdin, answers and events on stdout, log on stderr.

    {"id": 7, "module": "system", "method": "ping", "params": {}}   request
    {"id": 7, "result": "pong"}                                     answer
    {"id": 8, "error": {"code": "...", "message": "..."}}           failed answer
    {"event": "inbox.file", "module": "inbox", "data": {...}}       event
    {"modules": {"system": ["info", "ping"], ...}}                  first line

Each request runs in its own thread, so a slow device does not block the others.
Usage: main.py --config <path of the runner config.json> [--module <name> <folder>]...

`--module` loads a module downloaded by the shell, which has already checked its
signature and hashes: `<folder>` goes to the end of `sys.path` and the package
`<name>` is imported after the modules of the sidecar.
"""
import argparse
import importlib
import json
import os
import sys
import threading
import traceback

import modules  # noqa: F401 (imports every module, so they register)
from registry import MODULES, Registry


class Sidecar(object):

    def __init__(self, config):
        self.config = config
        self.write_lock = threading.Lock()
        self.registry = Registry(self)

    def write(self, message):
        line = json.dumps(message, ensure_ascii=False)
        with self.write_lock:
            sys.stdout.write(line + '\n')
            sys.stdout.flush()

    def emit(self, module_name, event, data):
        self.write(dict(event='%s.%s' % (module_name, event), module=module_name, data=data))

    def log(self, text):
        sys.stderr.write('[sidecar] %s\n' % text)
        sys.stderr.flush()

    def handle(self, request):
        request_id = request.get('id')
        try:
            result = self.registry.call(request['module'], request['method'], request.get('params'))
        except LookupError as e:
            self.write(dict(id=request_id, error=dict(code='unknown', message=str(e))))
        except Exception as e:
            self.log(traceback.format_exc())
            self.write(dict(id=request_id, error=dict(code=type(e).__name__, message=str(e))))
        else:
            self.write(dict(id=request_id, result=result))

    def run(self):
        self.write(dict(modules=self.registry.describe()))
        for line in sys.stdin:
            if not line.strip():
                continue
            request = json.loads(line)
            threading.Thread(target=self.handle, args=(request,), daemon=True).start()
        # stdin closed: the shell is gone.


def load_installed(name, folder):
    """Imports the downloaded package `name` from `folder`. The package must come from
    that folder and register exactly the module `name`; otherwise it is dropped."""
    folder = os.path.abspath(folder)
    before = set(MODULES)
    sys.path.append(folder)
    try:
        package = importlib.import_module(name)
        problem = None
    except Exception:
        package = None
        problem = traceback.format_exc()
    added = set(MODULES) - before
    if problem is None:
        source = os.path.abspath(getattr(package, '__file__', None) or os.sep)
        if os.path.commonpath([source, folder]) != folder:
            problem = 'the package comes from %s, not from %s\n' % (source, folder)
        elif added != {name}:
            problem = 'the package registers %s instead of %s\n' % (sorted(added), name)
    if problem is None:
        return True
    for extra in added:
        del MODULES[extra]
    sys.stderr.write('[sidecar] module %s not loaded: %s' % (name, problem))
    return False


def load_config(path):
    with open(path, encoding='utf-8') as f:
        return json.load(f)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', help='path of the runner config.json; missing file = empty config')
    parser.add_argument('--module', nargs=2, action='append', default=[], metavar=('NAME', 'FOLDER'),
                        help='a downloaded module, already verified by the shell')
    args = parser.parse_args()
    sys.dont_write_bytecode = True  # a __pycache__ in a module folder would fail the hash check of the shell
    for name, folder in args.module:
        load_installed(name, folder)
    try:
        config = load_config(args.config) if args.config else {}
    except FileNotFoundError:
        config = {}
    Sidecar(config).run()


if __name__ == '__main__':
    main()
