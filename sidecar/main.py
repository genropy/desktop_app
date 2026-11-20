"""Python sidecar of GenroApp.

The shell starts this process and talks to it with one JSON object per line:
requests on stdin, answers and events on stdout, log on stderr.

    {"id": 7, "module": "system", "method": "ping", "params": {}}   request
    {"id": 7, "result": "pong"}                                     answer
    {"id": 8, "error": {"code": "...", "message": "..."}}           failed answer
    {"event": "inbox.file", "module": "inbox", "data": {...}}       event
    {"modules": {"system": ["info", "ping"], ...}}                  first line

Each request runs in its own thread, so a slow device does not block the others.
Usage: main.py --config <path of the runner config.json>
"""
import argparse
import json
import sys
import threading
import traceback

import modules  # noqa: F401 (imports every module, so they register)
from registry import Registry


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


def load_config(path):
    with open(path, encoding='utf-8') as f:
        return json.load(f)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', help='path of the runner config.json; missing file = empty config')
    args = parser.parse_args()
    try:
        config = load_config(args.config) if args.config else {}
    except FileNotFoundError:
        config = {}
    Sidecar(config).run()


if __name__ == '__main__':
    main()
