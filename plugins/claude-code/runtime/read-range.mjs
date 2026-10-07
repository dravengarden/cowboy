// Execute through the bound native executor, never the runtime's filesystem.
// No cache, downloaded copy, shell interpolation or temporary target file.
export const READ_RANGE = String.raw`
import os, sys, stat, json, hashlib, base64
def emit(value):
    print(json.dumps(value, separators=(',', ':')))
def stamp(value):
    return (value.st_dev, value.st_ino, value.st_size,
            value.st_mtime_ns, value.st_ctime_ns)
try:
    path, offset, limit = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
    with os.fdopen(descriptor, 'rb') as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > 4194304:
            raise ValueError('Read requires a file of at most 4 MiB')
        data = source.read(4194305)
        after = os.fstat(source.fileno())
        current = os.stat(path)
        if len(data) != before.st_size or len(data) > 4194304 or stamp(before) != stamp(after) or stamp(after) != stamp(current):
            raise ValueError('File changed while reading. Read it again.')
    if (data.startswith((b'\x89PNG\r\n\x1a\n', b'\xff\xd8\xff', b'GIF', b'%PDF'))
            or data[:4] == b'RIFF' and data[8:12] == b'WEBP'):
        emit({'fallback': True})
    else:
        # Invalid bytes are replaced, as a whole-file Read shows them.
        lines = data.decode('utf-8', errors='replace').split('\n')
        selected = lines[offset-1:offset-1+limit]
        content = '\n'.join(selected).encode('utf-8')
        # Leave room under the existing 64 KiB retained process-output limit.
        # Larger selections use the unchanged full-read path and its limits.
        if len(content) > 32768:
            emit({'fallback': True})
        else:
            emit({'schema': 1, 'sha256': hashlib.sha256(data).hexdigest(),
                  'size': len(data), 'totalLines': len(lines),
                  'startLine': offset, 'numLines': len(selected),
                  'dataBase64': base64.b64encode(content).decode('ascii')})
except Exception:
    # File names, source contents and tracebacks do not belong in diagnostics.
    emit({'error': 'Target range read failed or file changed; read it again.'})
`;
