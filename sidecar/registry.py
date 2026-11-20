"""Modules and methods the shell may call.

A module is a class decorated with `@module(name)`; its methods decorated with
`@method` are callable as `<module>.<method>`. The registry creates one instance
per module and gives it the `Sidecar`, so a module can emit events.
"""

MODULES = {}


def module(name):
    def register(cls):
        if name in MODULES:
            raise ValueError('module %s is already registered' % name)
        cls.module_name = name
        MODULES[name] = cls
        return cls
    return register


def method(func):
    func.is_method = True
    return func


class Registry(object):

    def __init__(self, sidecar):
        self.instances = {name: cls(sidecar) for name, cls in MODULES.items()}

    def describe(self):
        "Module name -> list of its method names."
        return {name: sorted(n for n in dir(instance) if getattr(getattr(instance, n), 'is_method', False))
                for name, instance in self.instances.items()}

    def call(self, module_name, method_name, params):
        instance = self.instances.get(module_name)
        if instance is None:
            raise LookupError('unknown module %s' % module_name)
        handler = getattr(instance, method_name, None)
        if not getattr(handler, 'is_method', False):
            raise LookupError('unknown method %s.%s' % (module_name, method_name))
        return handler(**(params or {}))
