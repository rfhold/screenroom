"""Portable locking and atomic local backend file operations."""
import fcntl
import json
import os
from pathlib import Path
import secrets
import stat
import sys
import tempfile
import uuid

LIMIT = 1024 * 1024


def private_directory(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
        raise ValueError('runtime must be an owned directory')


def read(path, private=False):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as file:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or (private and
            (info.st_uid != os.getuid() or info.st_mode & 0o077)):
            raise ValueError('invalid runtime file ownership or mode')
        data = file.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError('runtime file exceeds 1 MiB')
    return data


def publish(path, data, mode, replace=False):
    fd, temporary = tempfile.mkstemp(prefix='.bootstrap-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as file:
            os.fchmod(fd, mode)
            file.write(data)
            file.flush()
            os.fsync(fd)
        if replace:
            os.replace(temporary, path)
        else:
            os.link(temporary, path, follow_symlinks=False)
    finally:
        Path(temporary).unlink(missing_ok=True)


def unique_fields(pairs):
    fields = dict(pairs)
    if len(fields) != len(pairs):
        raise ValueError('duplicate JSON field')
    return fields


def main(action, *args):
    if action == 'lock':
        directory, mode, script, *command = args
        directory = Path(directory)
        private_directory(directory)
        fd = os.open(directory / 'lifecycle.lock',
                     os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise ValueError('invalid lifecycle lock')
        fcntl.flock(fd, (fcntl.LOCK_SH if mode == 'shared' else fcntl.LOCK_EX) | fcntl.LOCK_NB)
        os.set_inheritable(fd, True)
        os.environ['SCREENROOM_INTERNAL_LOCK_FD'] = str(fd)
        os.execvp('bash', ['bash', script, '--locked', *command])
    if action == 'verify-lock':
        directory, mode = args
        fd = int(os.environ.get('SCREENROOM_INTERNAL_LOCK_FD', '-1'))
        current = os.fstat(fd)
        named = (Path(directory) / 'lifecycle.lock').lstat()
        if not os.path.samestat(current, named):
            raise ValueError('invalid lifecycle lock')
        fcntl.flock(fd, (fcntl.LOCK_SH if mode == 'shared' else fcntl.LOCK_EX) | fcntl.LOCK_NB)
        return
    if action == 'uuid':
        print(uuid.uuid4() if not args else uuid.UUID(args[0]))
        return
    path = Path(args[0])
    if action == 'project-users':
        users = json.loads(read(path).decode('utf-8'), object_pairs_hook=unique_fields)
        if not isinstance(users, list) or not 1 <= len(users) <= 1000:
            raise ValueError('invalid users.json')
        ids = set()
        for user in users:
            if not isinstance(user, dict) or not isinstance(user.get('display_name'), str):
                raise ValueError('invalid users.json user')
            if 'links' in user and user.pop('links') != []:
                raise ValueError('users.json contains unsupported links; preserve the file and remove the legacy links field explicitly before setup')
            identity = str(uuid.UUID(user.get('id', '')))
            if identity in ids:
                raise ValueError('duplicate existing users.json identity')
            ids.add(identity)
        data = json.dumps(users, ensure_ascii=False, separators=(',', ':')).encode()
        if len(data) > LIMIT:
            raise ValueError('projected users.json exceeds 1 MiB')
        sys.stdout.buffer.write(data)
    elif action in ('read', 'read-private'):
        sys.stdout.buffer.write(read(path, action == 'read-private'))
    elif action in ('create', 'replace'):
        content = sys.stdin.buffer.read(LIMIT + 1)
        if len(content) > LIMIT:
            raise ValueError('runtime input exceeds 1 MiB')
        publish(path, content, int(args[1], 8), replace=action == 'replace')
    elif action == 'create-env':
        content = [f'GARAGE_RPC_SECRET={secrets.token_hex(32)}']
        for role in ('CATALOG', 'WORKER'):
            content.extend([f'{role}_S3_KEY=GK{secrets.token_hex(12)}',
                            f'{role}_S3_SECRET={secrets.token_hex(32)}'])
        publish(path, ('\n'.join(content) + '\n').encode(), 0o600)
    else:
        raise ValueError('unknown file operation')


if __name__ == '__main__':
    try:
        main(*sys.argv[1:])
    except BlockingIOError:
        sys.exit('backend: lifecycle lock is busy')
    except (ValueError, TypeError, AttributeError, OSError, RecursionError) as error:
        allowed = ('users.json contains unsupported links',)
        if isinstance(error, ValueError) and str(error).startswith(allowed):
            sys.exit('backend: ' + str(error))
        sys.exit('backend: invalid or unavailable runtime file')
