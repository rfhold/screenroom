import os
from pathlib import Path
import runpy
import shutil
import socket
import subprocess
import sys
import threading
import time
import unittest

SCRIPT = Path(sys.argv.pop(1)) if len(sys.argv) > 1 else Path(__file__).with_name('origin.py')
resolve = runpy.run_path(str(SCRIPT))['resolve']


class OriginTest(unittest.TestCase):
    def test_canonical_origin_and_aliases(self):
        origin = 'http://127.0.0.1:51234'
        self.assertEqual(resolve(origin, {'SCREENROOM_ENDPOINT': origin}), 51234)
        for invalid in ('http://127.0.0.1:0', 'http://127.0.0.1:65536',
                        'http://127.0.0.1:bad', 'http://127.0.0.1:1234/path',
                        'http://other:1234', 'https://127.0.0.1:1234',
                        'http://127.0.0.1', 'http://127.0.0.1:01234'):
            with self.assertRaises(ValueError):
                resolve(invalid, {})
        for alias in ('SCREENROOM_ENDPOINT', 'SCREENROOM_E2E_API_ENDPOINT'):
            with self.assertRaises(ValueError):
                resolve(origin, {alias: 'http://127.0.0.1:50051'})

    @unittest.skipUnless(shutil.which('socat'), 'socat required for byte tunnel test')
    def test_nondefault_actual_forwarder_transports_unmodified_bytes(self):
        with socket.socket() as echo, socket.socket() as reserved:
            echo.bind(('127.0.0.1', 0))
            echo.listen()
            echo.settimeout(5)
            reserved.bind(('127.0.0.1', 0))
            port = reserved.getsockname()[1]
            self.assertNotEqual(port, 50051)
            reserved.close()

            def respond():
                connection, _ = echo.accept()
                with connection:
                    connection.settimeout(3)
                    body = bytearray()
                    while chunk := connection.recv(1024):
                        body.extend(chunk)
                    connection.sendall(body)

            worker = threading.Thread(target=respond, daemon=True)
            worker.start()
            environment = dict(os.environ)
            for alias in ('SCREENROOM_ENDPOINT', 'SCREENROOM_E2E_API_ENDPOINT'):
                environment.pop(alias, None)
            environment.update(SCREENROOM_API_ENDPOINT=f'http://127.0.0.1:{port}',
                               SCREENROOM_BACKEND_UPSTREAM=f'127.0.0.1:{echo.getsockname()[1]}')
            forwarder = subprocess.Popen([sys.executable, str(SCRIPT), '--forward'], env=environment)
            try:
                deadline = time.monotonic() + 3
                while True:
                    try:
                        client = socket.create_connection(('127.0.0.1', port), timeout=1)
                        break
                    except ConnectionRefusedError:
                        if time.monotonic() >= deadline:
                            raise
                        time.sleep(.02)
                with client:
                    data = b'transparent\x00bytes\xff'
                    client.sendall(data)
                    client.shutdown(socket.SHUT_WR)
                    result = bytearray()
                    while chunk := client.recv(1024):
                        result.extend(chunk)
                    self.assertEqual(result, data)
                worker.join(timeout=3)
                self.assertFalse(worker.is_alive())
            finally:
                forwarder.terminate()
                forwarder.wait(timeout=3)


if __name__ == '__main__':
    unittest.main()
