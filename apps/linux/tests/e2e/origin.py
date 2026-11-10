#!/usr/bin/env python3
import os
import sys
from urllib.parse import urlsplit


def resolve(origin, environ):
    parsed = urlsplit(origin)
    if (parsed.scheme != 'http' or parsed.hostname != '127.0.0.1'
            or parsed.username or parsed.password or parsed.path
            or parsed.query or parsed.fragment or not parsed.port
            or origin != f'http://127.0.0.1:{parsed.port}'):
        raise ValueError('Expected canonical http://127.0.0.1:<numeric port> origin')
    for key in ('SCREENROOM_ENDPOINT', 'SCREENROOM_E2E_API_ENDPOINT'):
        if key in environ and environ[key] != origin:
            raise ValueError(f'{key} conflicts with SCREENROOM_API_ENDPOINT')
    return parsed.port


if __name__ == '__main__':
    try:
        port = resolve(os.environ['SCREENROOM_API_ENDPOINT'], os.environ)
        if sys.argv[1:] == ['--forward']:
            os.execvp('socat', ['socat', f'TCP-LISTEN:{port},bind=127.0.0.1,reuseaddr,fork',
                               f"TCP:{os.environ['SCREENROOM_BACKEND_UPSTREAM']}"])
        print(port)
    except (KeyError, ValueError) as error:
        sys.exit(str(error))
