"""Bounded subprocess capture; a contract owner may own its entire process group."""
import os
import selectors
import signal
import subprocess
import time


def bounded_run(args, *, cwd, timeout, cap=2 * 1024 * 1024, input=None, group=False, check=True):
    child = subprocess.Popen(args, cwd=cwd, stdin=subprocess.PIPE if input is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=group)
    selector = selectors.DefaultSelector()
    buffers = {'stdout': bytearray(), 'stderr': bytearray()}
    deadline = time.monotonic() + timeout
    written = 0
    for name, pipe in (('stdout', child.stdout), ('stderr', child.stderr)):
        os.set_blocking(pipe.fileno(), False)
        selector.register(pipe, selectors.EVENT_READ, name)
    if input is not None:
        os.set_blocking(child.stdin.fileno(), False)
        selector.register(child.stdin, selectors.EVENT_WRITE, 'stdin')
    try:
        while selector.get_map() or child.poll() is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(args, timeout)
            for key, _ in selector.select(min(remaining, 0.05)):
                if key.data == 'stdin':
                    if written < len(input):
                        try:
                            written += os.write(key.fd, input[written:written + 4096])
                        except BrokenPipeError:
                            written = len(input)
                    if written == len(input):
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                    continue
                data = os.read(key.fd, 4096)
                if not data:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                else:
                    if sum(map(len, buffers.values())) + len(data) > cap:
                        raise RuntimeError('subprocess output budget exceeded')
                    buffers[key.data].extend(data)
        result = subprocess.CompletedProcess(args, child.wait(),
            buffers['stdout'].decode('utf-8', errors='replace'), buffers['stderr'].decode('utf-8', errors='replace'))
        if check and result.returncode:
            raise subprocess.CalledProcessError(result.returncode, args, result.stdout, result.stderr)
        return result
    finally:
        if group:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if child.poll() is None:
            child.kill()
        child.wait()
        selector.close()
        for pipe in (child.stdin, child.stdout, child.stderr):
            if pipe is not None:
                pipe.close()
