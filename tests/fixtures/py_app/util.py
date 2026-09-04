def greet(name):
    return _decorate(name)


def _decorate(name):
    return f"hello, {name}"
