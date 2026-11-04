import platform
import socket
import sys

from registry import method, module


@module('system')
class System(object):

    def __init__(self, sidecar):
        self.sidecar = sidecar

    @method
    def ping(self):
        return 'pong'

    @method
    def info(self):
        return dict(python=sys.version.split()[0], system=platform.system(), release=platform.release(),
                    machine=platform.machine(), hostname=socket.gethostname())
