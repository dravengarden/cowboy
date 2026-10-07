"""File tool cases whose results and on-disk effects must match native Claude Code.

Shared by the native-local baseline probe and the packaged remote acceptance.
`setup` creates the same files under a project root; `CASES` are the tool
calls (relative paths) made in order within one turn; `effects` reports what
each file looks like afterwards, independently of its location.
"""
import os
import re
import stat

UTF16 = "\ufeffline one\r\nline two\r\n".encode("utf-16-le")
LATIN1 = "caf\xe9 old\n".encode("latin-1")


def setup(root):
    def write(name, data, mode=0o644):
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        os.chmod(path, mode)
    write("exec.sh", b"#!/bin/sh\necho old\n", 0o755)
    write("private.txt", b"secret old\n", 0o600)
    write("linked.txt", b"shared old\n")
    os.link(root / "linked.txt", root / "linked-alias.txt")
    write("crlf.txt", b"alpha old\r\nbeta\r\n")
    write("bom.txt", b"\xef\xbb\xbfbom old\n")
    write("utf16.txt", UTF16)
    write("latin1.txt", LATIN1)
    write("readonly.txt", b"locked old\n", 0o444)
    write("nonl.txt", b"no newline old")
    write("unread.txt", b"never read\n")
    write("empty.txt", b"")
    (root / "adir").mkdir()
    return {name: os.stat(root / name).st_ino for name in
            ["exec.sh", "private.txt", "linked.txt", "crlf.txt", "bom.txt", "utf16.txt",
             "latin1.txt", "readonly.txt", "nonl.txt", "empty.txt"]}


def _read(name):
    return ("Read", {"file_path": name})


def _edit(name, old, new):
    return ("Edit", {"file_path": name, "old_string": old, "new_string": new})


CASES = [
    ("read_exec", _read("exec.sh")), ("edit_exec", _edit("exec.sh", "old", "new")),
    ("read_private", _read("private.txt")),
    ("write_private", ("Write", {"file_path": "private.txt", "content": "secret new\n"})),
    ("read_linked", _read("linked.txt")), ("edit_linked", _edit("linked.txt", "old", "new")),
    ("read_crlf", _read("crlf.txt")), ("edit_crlf", _edit("crlf.txt", "alpha old", "alpha new\nadded")),
    ("read_bom", _read("bom.txt")), ("edit_bom", _edit("bom.txt", "bom old", "bom new")),
    ("read_utf16", _read("utf16.txt")), ("edit_utf16", _edit("utf16.txt", "line one", "line uno")),
    ("read_latin1", _read("latin1.txt")), ("edit_latin1", _edit("latin1.txt", "old", "new")),
    ("read_readonly", _read("readonly.txt")), ("edit_readonly", _edit("readonly.txt", "old", "new")),
    ("read_nonl", _read("nonl.txt")), ("edit_nonl", _edit("nonl.txt", "old", "new")),
    ("write_unread", ("Write", {"file_path": "unread.txt", "content": "overwritten\n"})),
    ("write_new_nested", ("Write", {"file_path": "new/deeper/file.txt", "content": "created\n"})),
    ("write_directory", ("Write", {"file_path": "adir", "content": "x\n"})),
    ("read_empty", _read("empty.txt")), ("edit_empty", _edit("empty.txt", "", "filled\n")),
    ("edit_missing", _edit("missing.txt", "", "made by edit\n")),
]


NAMES = ["exec.sh", "private.txt", "linked.txt", "linked-alias.txt", "crlf.txt", "bom.txt", "utf16.txt",
         "latin1.txt", "readonly.txt", "nonl.txt", "unread.txt", "empty.txt", "missing.txt", "new/deeper/file.txt"]


def effects(root, inodes):
    report = {}
    for name in NAMES:
        path = root / name
        if not path.is_file():
            report[name] = None
            continue
        info = os.stat(path)
        report[name] = {
            "mode": oct(stat.S_IMODE(info.st_mode)),
            "nlink": info.st_nlink,
            "same_inode": inodes.get(name) == info.st_ino if name in inodes else None,
            "bytes": path.read_bytes().hex(),
        }
    return report


def normalize(text, project):
    text = re.sub(r"\n*<system-reminder>.*?</system-reminder>\n*", "", text, flags=re.S)
    return text.replace(str(project), "<project>")
