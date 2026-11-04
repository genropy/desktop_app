"""Folders watched for documents to hand to a GenroPy page.

config.json of the runner:

    "inbox": [{"folder": "~/GenroPy Inbox/Invoices", "page": "invoices"},
              {"folder": "~/GenroPy Inbox", "page": null}]

A page asks only for the files of its own name (`pending(page)`) and listens to the
`inbox.file` event filtering on `page`. A file is offered once its size stops changing,
and stays in the folder until the page calls `done`: then it moves to `Sent/` or
`Errors/`. A file dropped while the app is closed, or while no page of that name is
open, is offered later. Only regular, non-hidden files directly in the folder count.
"""
import base64
import os
import threading
import time

from watchdog.events import FileSystemEventHandler
from watchdog.observers import Observer

from registry import method, module

STABLE_SECONDS = 1.0
SENT = 'Sent'
ERRORS = 'Errors'


class Folder(object):

    def __init__(self, index, path, page):
        self.index = index
        self.path = path
        self.page = page

    def file_id(self, name):
        return '%i:%s' % (self.index, name)


class Handler(FileSystemEventHandler):

    def __init__(self, inbox, folder):
        self.inbox = inbox
        self.folder = folder

    def on_created(self, event):
        self.inbox.watch(self.folder, event.src_path)

    def on_moved(self, event):
        self.inbox.watch(self.folder, event.dest_path)


@module('inbox')
class Inbox(object):

    def __init__(self, sidecar):
        self.sidecar = sidecar
        self.lock = threading.Lock()
        self.ready = {}        # file id -> (folder, path)
        self.watching = set()  # paths whose size is being checked
        self.folders = []
        self.observer = Observer()
        self.observer.daemon = True
        for index, entry in enumerate(sidecar.config.get('inbox', [])):
            folder = Folder(index, os.path.expanduser(entry['folder']), entry.get('page'))
            for path in (folder.path, os.path.join(folder.path, SENT), os.path.join(folder.path, ERRORS)):
                os.makedirs(path, exist_ok=True)
            self.folders.append(folder)
            self.observer.schedule(Handler(self, folder), folder.path, recursive=False)
        self.observer.start()
        for folder in self.folders:
            for name in os.listdir(folder.path):
                self.watch(folder, os.path.join(folder.path, name))

    def watch(self, folder, path):
        "Offers `path` once its size is stable, if it is a regular visible file directly in `folder`."
        name = os.path.basename(path)
        if os.path.dirname(path) != folder.path or name.startswith('.') or not os.path.isfile(path):
            return
        with self.lock:
            if path in self.watching or folder.file_id(name) in self.ready:
                return
            self.watching.add(path)
        threading.Thread(target=self.wait_stable, args=(folder, path), daemon=True).start()

    def wait_stable(self, folder, path):
        try:
            size = -1
            while os.path.isfile(path) and os.path.getsize(path) != size:
                size = os.path.getsize(path)
                time.sleep(STABLE_SECONDS)
            if not os.path.isfile(path):
                return
            name = os.path.basename(path)
            with self.lock:
                self.ready[folder.file_id(name)] = (folder, path)
            self.sidecar.emit('inbox', 'file', dict(page=folder.page, id=folder.file_id(name), name=name, size=size))
        finally:
            with self.lock:
                self.watching.discard(path)

    def entry(self, file_id):
        with self.lock:
            if file_id not in self.ready:
                raise LookupError('no file %s waiting in the inbox' % file_id)
            return self.ready[file_id]

    @method
    def folders_list(self):
        return [dict(folder=f.path, page=f.page) for f in self.folders]

    @method
    def pending(self, page=None):
        "Files waiting for `page` (null: the folders with no page)."
        with self.lock:
            items = [(file_id, path) for file_id, (folder, path) in self.ready.items() if folder.page == page]
        return [dict(id=file_id, name=os.path.basename(path), size=os.path.getsize(path)) for file_id, path in items]

    @method
    def read(self, id):
        path = self.entry(id)[1]
        with open(path, 'rb') as f:
            data = f.read()
        return dict(id=id, name=os.path.basename(path), size=len(data), data=base64.b64encode(data).decode())

    @method
    def done(self, id, ok=True, message=None):
        "Moves the file to Sent/ (ok) or Errors/ (not ok). An existing name gets a timestamp suffix."
        folder, path = self.entry(id)
        name = os.path.basename(path)
        target = os.path.join(folder.path, SENT if ok else ERRORS, name)
        if os.path.exists(target):
            stem, ext = os.path.splitext(name)
            target = os.path.join(os.path.dirname(target), '%s-%s%s' % (stem, time.strftime('%Y%m%d-%H%M%S'), ext))
        os.replace(path, target)
        with self.lock:
            del self.ready[id]
        if not ok:
            self.sidecar.log('inbox: %s -> %s (%s)' % (name, target, message))
        return dict(id=id, moved_to=target)
