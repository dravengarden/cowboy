#!/usr/bin/env python3
"""Conformance probe for the Machine browser launcher, run as root inside the
pristine Ubuntu container `conformance.sh` prepares. It plays the Machine:
starts the launcher as the unprivileged test user inside a delegated cgroup,
waits for `ready`, verifies the isolation facts from outside the instance
(listeners, uid map, capabilities, mounts, namespaces, start times), drives
the RFB and control protocols (handshake, clipboard, native touch,
navigate, resize, status, snapshot, stop with a snapshot), then restores the
published copy in a second run. Standard library only; prints one JSON
summary.
"""
import argparse
import importlib.util
import json
import os
import re
import socket
import struct
import zlib
import subprocess
import sys
import time

CGROUP_ROOT = "/sys/fs/cgroup"
CHECKS = []
LAUNCHERS = []


def record(name, ok, **detail):
    CHECKS.append({"check": name, "ok": bool(ok), **detail})
    print(f"[{'ok' if ok else 'FAIL'}] {name} {json.dumps(detail, default=str)[:400]}", file=sys.stderr)
    return ok


def run(*cmd, check=True, **kwargs):
    return subprocess.run(cmd, check=check, capture_output=True, text=True, **kwargs)


def read(path, default=""):
    try:
        with open(path) as handle:
            return handle.read()
    except OSError:
        return default


class Control:
    def __init__(self, path, timeout=30):
        deadline = time.time() + timeout
        while True:
            try:
                self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.sock.connect(path)
                break
            except OSError:
                self.sock.close()
                if time.time() > deadline:
                    raise
                time.sleep(0.1)
        self.buffer = b""
        self.events = []
        self.next_id = 1

    def read_line(self, timeout):
        self.sock.settimeout(timeout)
        while b"\n" not in self.buffer:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("control socket closed")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line)

    def wait_event(self, name, timeout):
        deadline = time.time() + timeout
        while True:
            remaining = deadline - time.time()
            if remaining <= 0:
                raise TimeoutError(f"no {name} event")
            message = self.read_line(remaining)
            if message.get("event") == name:
                return message
            if message.get("event") == "error":
                raise RuntimeError(f"launcher error: {message}")
            self.events.append(message)

    def request(self, command, timeout=60, **fields):
        request_id = self.next_id
        self.next_id += 1
        self.sock.sendall((json.dumps({"id": request_id, "command": command, **fields}) + "\n").encode())
        deadline = time.time() + timeout
        while True:
            remaining = deadline - time.time()
            if remaining <= 0:
                raise TimeoutError(f"{command} timed out")
            message = self.read_line(remaining)
            if message.get("id") == request_id:
                return message
            self.events.append(message)

    def close(self):
        self.sock.close()


class Rfb:
    """Minimal RFB 3.8 client (security None)."""

    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(15)
        self.sock.connect(path)
        self.version = self.exact(12).decode().strip()
        self.sock.sendall(b"RFB 003.008\n")
        count = self.exact(1)[0]
        self.security_types = list(self.exact(count))
        if 1 not in self.security_types:
            raise RuntimeError(f"security type None not offered: {self.security_types}")
        self.sock.sendall(bytes([1]))
        (self.security_result,) = struct.unpack(">I", self.exact(4))
        self.sock.sendall(bytes([1]))
        self.width, self.height = struct.unpack(">HH", self.exact(4))
        pixel_format = self.exact(16)
        self.bpp = pixel_format[0] // 8
        self.big_endian = pixel_format[2] != 0
        self.shifts = (pixel_format[10], pixel_format[11], pixel_format[12])
        self.fb = bytearray(self.width * self.height * self.bpp)
        (name_length,) = struct.unpack(">I", self.exact(4))
        self.name = self.exact(name_length).decode(errors="replace")
        encodings = [0, -223, -308]
        self.sock.sendall(struct.pack(">BBH", 2, 0, len(encodings)) + b"".join(struct.pack(">i", e) for e in encodings))

    def exact(self, count):
        data = b""
        while len(data) < count:
            chunk = self.sock.recv(count - len(data))
            if not chunk:
                raise RuntimeError("rfb socket closed")
            data += chunk
        return data

    def paint(self, x, y, w, h, data):
        if len(self.fb) != self.width * self.height * self.bpp:
            self.fb = bytearray(self.width * self.height * self.bpp)
        row = w * self.bpp
        for line in range(h):
            if y + line >= self.height or x + w > self.width:
                break
            start = ((y + line) * self.width + x) * self.bpp
            self.fb[start:start + row] = data[line * row:(line + 1) * row]

    def save_png(self, path):
        """Write the tracked framebuffer (raw rects only) as an RGB PNG."""
        order = "big" if self.big_endian else "little"
        rows = bytearray()
        for line in range(self.height):
            rows.append(0)
            for column in range(self.width):
                start = (line * self.width + column) * self.bpp
                pixel = int.from_bytes(self.fb[start:start + self.bpp], order)
                rows.extend(((pixel >> shift) & 0xFF) for shift in self.shifts)
        def chunk(kind, body):
            return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        header = struct.pack(">IIBBBBB", self.width, self.height, 8, 2, 0, 0, 0)
        with open(path, "wb") as handle:
            handle.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(rows))) + chunk(b"IEND", b""))

    def request_update(self, incremental):
        self.sock.sendall(struct.pack(">BBHHHH", 3, 1 if incremental else 0, 0, 0, self.width, self.height))

    def message(self, timeout):
        self.sock.settimeout(timeout)
        kind = self.exact(1)[0]
        if kind == 0:
            self.exact(1)
            (count,) = struct.unpack(">H", self.exact(2))
            rects = []
            for _ in range(count):
                x, y, w, h, encoding = struct.unpack(">HHHHi", self.exact(12))
                if encoding == 0:
                    self.paint(x, y, w, h, self.exact(w * h * self.bpp))
                elif encoding == -308:
                    screens = self.exact(4)[0]
                    self.exact(16 * screens)
                elif encoding == -223:
                    pass
                elif encoding == -239:
                    self.exact(w * h * self.bpp + ((w + 7) // 8) * h)
                elif encoding != -232:
                    raise RuntimeError(f"unexpected encoding {encoding}")
                if encoding in (-223, -308):
                    self.width, self.height = w, h
                rects.append({"x": x, "y": y, "w": w, "h": h, "encoding": encoding})
            return {"type": "update", "rects": rects}
        if kind == 1:
            self.exact(1)
            _, count = struct.unpack(">HH", self.exact(4))
            self.exact(6 * count)
            return {"type": "colourmap"}
        if kind == 2:
            return {"type": "bell"}
        if kind == 3:
            self.exact(3)
            (length,) = struct.unpack(">i", self.exact(4))
            if length < 0:
                raise RuntimeError("extended clipboard not expected")
            return {"type": "cuttext", "text": self.exact(length).decode("latin-1")}
        raise RuntimeError(f"unknown server message {kind}")

    def wait_for(self, predicate, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                message = self.message(max(0.1, deadline - time.time()))
            except socket.timeout:
                return None
            if predicate(message):
                return message
            if message["type"] == "update":
                self.request_update(True)
        return None


def nspid(pid):
    for line in read(f"/proc/{pid}/status").splitlines():
        if line.startswith("NSpid:"):
            return [int(value) for value in line.split()[1:]]
    return []


def cgroup_procs(path):
    pids = []
    for root, _dirs, files in os.walk(path):
        if "cgroup.procs" in files:
            pids.extend(int(p) for p in read(os.path.join(root, "cgroup.procs")).split())
    return pids


def status_fields(pid):
    fields = {}
    for line in read(f"/proc/{pid}/status").splitlines():
        if ":" in line:
            key, value = line.split(":", 1)
            fields[key.strip()] = value.strip()
    return fields


def stat_start_time(pid):
    stat = read(f"/proc/{pid}/stat")
    after = stat.rsplit(")", 1)[1].split()
    return int(after[19]) if len(after) > 19 else None


def parse_mountinfo(text):
    entries = []
    for line in text.splitlines():
        if " - " not in line:
            continue
        left, right = line.split(" - ", 1)
        left = left.split(" ")
        right = right.split(" ", 2)
        if len(left) < 6 or len(right) < 3:
            continue
        entries.append({
            "major_minor": left[2],
            "root": left[3].replace("\\040", " "),
            "target": left[4].replace("\\040", " "),
            "vfs": left[5].split(","),
            "fstype": right[0],
            "source": right[1],
            "super": right[2].split(","),
        })
    return entries


def mount_of(entries, target):
    matches = [entry for entry in entries if entry["target"] == target]
    return matches[-1] if matches else None


def start_launcher(args, snapshot_dir, log_path, masks):
    flags = [
        "run",
        "--control-dir", args.control,
        "--runtime-dir", args.runtime,
        "--staging-dir", args.staging,
        "--display", args.display,
        "--geometry", args.geometry,
        "--scale", "1",
        "--profile-bytes", str(256 << 20),
        "--instance-bytes", str(128 << 20),
        "--tmp-bytes", str(128 << 20),
        "--policy-file", "usr/share/cowboy-browser/policy.json",
        "--policy-directory", "/etc/opt/chrome/policies/managed",
        "--browser", "opt/google/chrome/chrome",
        "--display-server", "usr/bin/Xtigervnc",
        "--window-manager", "usr/bin/matchbox-window-manager",
        "--xrandr", "usr/bin/xrandr",
        "--xkbcomp", "usr/bin/xkbcomp",
        "--xauth", "usr/bin/xauth",
    ]
    for mask in masks:
        flags += ["--mask", mask]
    if snapshot_dir:
        flags += ["--snapshot-dir", snapshot_dir]
    cmd = ["setpriv", f"--reuid={args.user}", f"--regid={args.user}", "--init-groups", "--reset-env", args.launcher, *flags]

    def place_in_cgroup():
        with open(os.path.join(args.cgroup, "cgroup.procs"), "w") as handle:
            handle.write(str(os.getpid()))

    for stale in ("control.sock", "rfb.sock"):
        try:
            os.unlink(os.path.join(args.control, stale))
        except FileNotFoundError:
            pass
    log = open(log_path, "ab")
    process = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=log, stderr=log, preexec_fn=place_in_cgroup)
    LAUNCHERS.append(process)
    return process


def instance_pids(args, ready):
    """Host pids of the init and the three sealed processes, matched through NSpid."""
    found = {}
    unit_pids = cgroup_procs(args.cgroup)
    for pid in unit_pids:
        ns = nspid(pid)
        if len(ns) == 2 and ns[1] == 1:
            found["init"] = pid
    for name in ("display_server", "window_manager", "browser"):
        wanted = ready["pids"][name]
        for pid in unit_pids:
            ns = nspid(pid)
            if len(ns) == 2 and ns[1] == wanted:
                found[name] = pid
    return found


def copy_instance_logs(init_pid, destination):
    root = f"/proc/{init_pid}/root/mnt/instance/log"
    os.makedirs(destination, exist_ok=True)
    try:
        for name in os.listdir(root):
            with open(os.path.join(root, name), "rb") as source, open(os.path.join(destination, name), "wb") as target:
                target.write(source.read())
    except OSError as error:
        print(f"could not copy instance logs: {error}", file=sys.stderr)


def tree_names(root):
    names = []
    for directory, dirs, files in os.walk(root):
        for name in dirs + files:
            path = os.path.join(directory, name)
            relative = os.path.relpath(path, root)
            kind = "link" if os.path.islink(path) else ("dir" if os.path.isdir(path) else ("file" if os.path.isfile(path) else "other"))
            names.append((relative, kind))
    return names


def make_generation(args, generation):
    path = os.path.join(args.staging, f"gen-{generation}")
    os.mkdir(path, 0o700)
    uid = int(run("id", "-u", args.user).stdout)
    gid = int(run("id", "-g", args.user).stdout)
    os.chown(path, uid, gid)
    return path


def verify_outside(args, ready, pids):
    listeners = [line for line in run("ss", "-H", "-tlnup").stdout.splitlines() if line.strip()]
    record("no_tcp_udp_listeners", not listeners, listeners=listeners)
    abstract = [line for line in run("ss", "-H", "-xlnp").stdout.splitlines() if "@" in line]
    record("no_abstract_unix_listeners", not abstract, sockets=abstract[:5])
    uid = int(run("id", "-u", args.user).stdout)
    for name in ("display_server", "window_manager", "browser"):
        pid = pids.get(name)
        if not pid:
            record(f"{name}_found", False)
            continue
        uid_map = " ".join(read(f"/proc/{pid}/uid_map").split())
        gid_map = " ".join(read(f"/proc/{pid}/gid_map").split())
        record(f"{name}_uid_map_is_self_only", uid_map == f"{uid} {uid} 1" and uid_map == ready["uid_map"], uid_map=uid_map, gid_map=gid_map)
        fields = status_fields(pid)
        caps = {key: fields.get(key) for key in ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")}
        record(f"{name}_capabilities_empty", all(int(value or "1", 16) == 0 for value in caps.values()), **caps)
        record(f"{name}_no_new_privs", fields.get("NoNewPrivs") == "1", NoNewPrivs=fields.get("NoNewPrivs"))
        namespaces = {ns: os.readlink(f"/proc/{pid}/ns/{ns}") for ns in ("user", "mnt", "pid", "ipc")}
        own = {ns: os.readlink(f"/proc/self/ns/{ns}") for ns in ("user", "mnt", "pid", "ipc")}
        record(
            f"{name}_namespaces_match_report_and_differ_from_host",
            namespaces == ready["namespaces"] and all(namespaces[ns] != own[ns] for ns in namespaces),
            namespaces=namespaces,
        )
        record(f"{name}_start_time_matches", stat_start_time(pid) == ready["start_times"].get(name), reported=ready["start_times"].get(name), observed=stat_start_time(pid))
    browser = pids.get("browser")
    if browser:
        mounts = parse_mountinfo(read(f"/proc/{browser}/mountinfo"))
        runtime = mount_of(parse_mountinfo(read("/proc/self/mountinfo")), args.runtime)
        for target in ("/usr", "/opt"):
            entry = mount_of(mounts, target)
            ok = bool(entry) and entry["major_minor"] == runtime["major_minor"] and entry["root"].endswith(target) and {"ro", "nosuid", "nodev"} <= set(entry["vfs"])
            record(f"{target}_is_the_artifact_read_only", ok, mount=entry)
        sockets = mount_of(mounts, "/mnt/sockets")
        record("/mnt/sockets_read_only", bool(sockets) and "ro" in sockets["vfs"], mount=sockets)
        for target in ("/home", f"/run/user/{uid}", "/etc"):
            entry = mount_of(mounts, target)
            record(f"{target}_is_read_only_tmpfs", bool(entry) and entry["fstype"] == "tmpfs" and "ro" in entry["vfs"], mount=entry)
        for target in ("/tmp", "/dev/shm", "/mnt/profile", "/mnt/instance"):
            entry = mount_of(mounts, target)
            record(f"{target}_is_tmpfs", bool(entry) and entry["fstype"] == "tmpfs", mount=entry)
        proc = mount_of(mounts, "/proc")
        record("/proc_is_fresh_proc", bool(proc) and proc["fstype"] == "proc", mount=proc)
        record("no_snapshot_or_publish_alias_mounted", not mount_of(mounts, "/mnt/snapshot") and not mount_of(mounts, "/mnt/publish"))
        policy = mount_of(mounts, "/etc/opt/chrome/policies/managed/policy.json")
        record("policy_file_bound_from_artifact", bool(policy) and policy["major_minor"] == runtime["major_minor"] and "ro" in policy["vfs"], mount=policy)
        for mask in args.mask:
            entry = mount_of(mounts, mask)
            masked_listing = os.listdir(f"/proc/{pids['init']}/root{mask}") if os.path.isdir(f"/proc/{pids['init']}/root{mask}") else None
            record(f"{mask}_masked", bool(entry) and entry["fstype"] == "tmpfs" and masked_listing == [], mount=entry, listing=masked_listing)
        resolv = read(f"/proc/{pids['init']}/root/etc/resolv.conf")
        record("resolv_conf_forces_tcp", "options use-vc" in resolv, resolv=resolv.strip())
        hosts_dir = os.listdir(f"/proc/{pids['init']}/root/etc")
        record("etc_scaffold_contents", set(hosts_dir) >= {"hosts", "passwd", "group", "resolv.conf", "nsswitch.conf", "opt"}, entries=sorted(hosts_dir))
    reported_mounts = {entry["target"] for entry in ready.get("mounts", [])}
    record("ready_reports_mount_table", {"/usr", "/opt", "/mnt/profile", "/mnt/sockets", "/etc", "/home"} <= reported_mounts, targets=sorted(reported_mounts))
    runtime_dir_entries = sorted(os.listdir(args.control))
    record("runtime_directory_holds_two_sockets", runtime_dir_entries == ["control.sock", "rfb.sock"], entries=runtime_dir_entries)


def clipboard_check(args, rfb, init_pid):
    marker = f"COWBOY-CONFORMANCE-{int(time.time())}"
    env = dict(os.environ, XAUTHORITY="/mnt/instance/xauth", DISPLAY=args.display, LD_LIBRARY_PATH=f"{args.xclip_dir}/lib")
    cmd = [
        "setpriv", f"--reuid={args.user}", f"--regid={args.user}", "--init-groups",
        "nsenter", "-U", "--preserve-credentials", "-m", "-p", "-t", str(init_pid),
        f"{args.xclip_dir}/xclip", "-selection", "clipboard",
    ]
    # xclip forks a child that keeps serving the selection and inherits the
    # standard streams: never wait on its pipes, log stderr to a file instead.
    xclip_log = os.path.join(args.out, "xclip.log")
    try:
        with open(xclip_log, "ab") as log:
            xclip = subprocess.run(cmd, input=marker.encode(), stdout=subprocess.DEVNULL, stderr=log, timeout=30, env=env)
    except subprocess.TimeoutExpired:
        return record("clipboard_forwarded", False, error="xclip timed out", log=read(xclip_log)[-400:])
    if xclip.returncode != 0:
        return record("clipboard_forwarded", False, error=read(xclip_log)[-400:])
    rfb.request_update(True)
    got = rfb.wait_for(lambda message: message["type"] == "cuttext", 15)
    return record("clipboard_forwarded", bool(got) and got["text"] == marker, received=got and got["text"])


def load_touch_probe():
    # -I keeps the probe's own directory off sys.path; load the sibling file.
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "touch_probe.py")
    spec = importlib.util.spec_from_file_location("touch_probe", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def touch_check(args, control):
    """Native touch through the real launcher: negotiation and stock-client
    compatibility, then a page in the instance's Chromium that reports the
    touch events it receives (touch_probe.py)."""
    touch = load_touch_probe()
    path = os.path.join(args.control, "rfb.sock")
    touch.protocol_checks(path, record)
    url, server = touch.serve_page()
    try:
        reply = control.request("navigate", url=url)
        record("touch_page_navigated", reply.get("ok") is True, reply=reply)
        time.sleep(5)
        touch.page_checks(path, record)
    finally:
        server.shutdown()


def phase_fresh(args):
    log_path = os.path.join(args.out, "launcher-1.log")
    process = start_launcher(args, None, log_path, args.mask)
    control = Control(os.path.join(args.control, "control.sock"))
    ready = control.wait_event("ready", 90)
    record("ready", True, pids=ready["pids"], labels=ready["labels"])
    pids = instance_pids(args, ready)
    record("all_instance_processes_found", {"init", "display_server", "window_manager", "browser"} <= set(pids), pids=pids)
    record("supervisor_is_launcher_main_pid", ready["pids"]["supervisor"] in cgroup_procs(args.cgroup), supervisor=ready["pids"]["supervisor"])
    time.sleep(2)
    verify_outside(args, ready, pids)

    rfb = Rfb(os.path.join(args.control, "rfb.sock"))
    width, height = (int(value) for value in args.geometry.split("x"))
    record("rfb_handshake", rfb.security_result == 0 and (rfb.width, rfb.height) == (width, height), version=rfb.version, size=[rfb.width, rfb.height], desktop_name=rfb.name)
    rfb.request_update(False)
    first = rfb.wait_for(lambda message: message["type"] == "update", 30)
    record("rfb_first_framebuffer_update", bool(first), rects=len(first["rects"]) if first else 0)
    # The start page as the owner first sees it: Chromium's new-tab page with
    # the policy's shortcuts (inspected by eye, not asserted).
    time.sleep(6)
    rfb.request_update(False)
    rfb.wait_for(lambda message: message["type"] == "update" and any(rect["encoding"] == 0 for rect in message["rects"]), 15)
    start_page = os.path.join(args.out, "start-page.png")
    rfb.save_png(start_page)
    record("start_page_captured", os.path.isfile(start_page), path=start_page, fatal=False)
    # SetDesktopSize must be refused or ignored (AcceptSetDesktopSize=0).
    rfb.sock.sendall(struct.pack(">BBHHBB", 251, 0, 640, 480, 1, 0) + struct.pack(">IHHHHI", 0, 0, 0, 640, 480, 0))
    rfb.request_update(True)
    resized = rfb.wait_for(lambda message: any(rect["encoding"] in (-223, -308) and rect["x"] == 0 and (rect["w"], rect["h"]) == (640, 480) for rect in message.get("rects", [])), 5)
    record("rfb_client_resize_refused", resized is None, size=[rfb.width, rfb.height])
    clipboard_check(args, rfb, pids["init"])
    touch_check(args, control)

    reply = control.request("navigate", url="https://example.com/")
    record("navigate_accepted", reply.get("ok") is True, reply=reply)
    time.sleep(5)
    established = [line for line in run("ss", "-H", "-tnp", check=False).stdout.splitlines() if "ESTAB" in line and ":443" in line]
    record("navigate_opened_a_tls_connection", bool(established), connections=established[:3], fatal=False)

    reply = control.request("resize", width=1024, height=700)
    record("resize_accepted", reply.get("ok") is True, reply=reply)
    rfb.request_update(True)
    changed = rfb.wait_for(lambda message: any(rect["encoding"] in (-223, -308) and (rect["w"], rect["h"]) == (1024, 700) for rect in message.get("rects", [])), 15)
    record("resize_visible_over_rfb", bool(changed), size=[rfb.width, rfb.height])

    reply = control.request("attach", attachment_id="a-1")
    record("attach_accepted", reply.get("ok") is True)
    reply = control.request("status")
    record("status_matches_ready", reply.get("ok") is True and reply.get("pids") == ready["pids"] and reply.get("namespaces") == ready["namespaces"] and reply.get("snapshot_in_progress") is False, reply_keys=sorted(reply))
    bad = control.request("resize", width=10, height=10)
    record("out_of_bounds_resize_refused", bad.get("ok") is False and bad.get("kind") == "internal", reply=bad)

    generation_1 = make_generation(args, 1)
    reply = control.request("snapshot", generation=1, timeout=120)
    names = tree_names(generation_1)
    kinds = {kind for _, kind in names}
    record("snapshot_copies_profile", reply.get("ok") is True and reply.get("entries", 0) > 0 and len(names) == reply.get("entries"), reply=reply, entries=len(names))
    record("snapshot_has_local_state", any(name == "Local State" and kind == "file" for name, kind in names))
    record("snapshot_has_only_files_and_directories", kinds <= {"file", "dir"}, kinds=sorted(kinds))
    record("snapshot_excludes_caches_and_singletons", not any(name.split("/")[-1] in ("Cache", "SingletonLock", "SingletonSocket", "SingletonCookie", "lockfile") for name, _ in names))
    missing = control.request("snapshot", generation=77)
    record("snapshot_of_missing_generation_refused", missing.get("ok") is False and missing.get("kind") == "host", reply=missing)

    copy_instance_logs(pids["init"], os.path.join(args.out, "instance-1"))
    generation_2 = make_generation(args, 2)
    reply = control.request("stop", graceful_ms=10000, snapshot_generation=2, timeout=120)
    record("stop_graceful_with_snapshot", reply.get("ok") is True and reply.get("graceful") is True and isinstance(reply.get("snapshot"), dict) and reply["snapshot"].get("entries", 0) > 0, reply=reply)
    record("stop_snapshot_has_local_state", os.path.isfile(os.path.join(generation_2, "Local State")))
    try:
        code = process.wait(timeout=40)
    except subprocess.TimeoutExpired:
        process.kill()
        code = "timeout"
    record("launcher_exits_zero_after_stop", code == 0, code=code)
    leftover = cgroup_procs(args.cgroup)
    record("cgroup_empty_after_stop", not leftover, pids=leftover)
    control.close()
    rfb.sock.close()
    return generation_2


def phase_restore(args, snapshot_dir):
    log_path = os.path.join(args.out, "launcher-2.log")
    process = start_launcher(args, snapshot_dir, log_path, args.mask)
    control = Control(os.path.join(args.control, "control.sock"))
    ready = control.wait_event("ready", 90)
    pids = instance_pids(args, ready)
    restored = re.search(r"restored snapshot: (\d+) bytes in (\d+) entries", read(log_path))
    record("restore_reported", bool(restored) and int(restored.group(2)) > 0, log_line=restored.group(0) if restored else None)
    profile_root = f"/proc/{pids['init']}/root/mnt/profile"
    record("restored_profile_has_local_state", os.path.isfile(os.path.join(profile_root, "Local State")))
    reply = control.request("status")
    record("status_after_restore", reply.get("ok") is True and reply.get("pids", {}).get("browser", 0) > 0)
    reply = control.request("stop", graceful_ms=10000, timeout=60)
    record("stop_without_snapshot", reply.get("ok") is True and "snapshot" not in reply and "snapshot_error" not in reply, reply=reply)
    try:
        code = process.wait(timeout=40)
    except subprocess.TimeoutExpired:
        process.kill()
        code = "timeout"
    record("launcher_exits_zero_after_second_stop", code == 0, code=code)
    control.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--launcher", required=True)
    parser.add_argument("--runtime", required=True)
    parser.add_argument("--control", required=True)
    parser.add_argument("--staging", required=True)
    parser.add_argument("--cgroup", required=True)
    parser.add_argument("--display", default=":7")
    parser.add_argument("--geometry", default="1280x800")
    parser.add_argument("--user", default="tester")
    parser.add_argument("--xclip-dir", required=True)
    parser.add_argument("--mask", action="append", default=[])
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    failure = None
    try:
        snapshot = phase_fresh(args)
        phase_restore(args, snapshot)
    except Exception as error:  # noqa: BLE001 - the summary carries it
        failure = f"{type(error).__name__}: {error}"
        print(f"probe aborted: {failure}", file=sys.stderr)
        for pid in cgroup_procs(args.cgroup):
            if nspid(pid)[-1:] == [1]:
                copy_instance_logs(pid, os.path.join(args.out, "instance-aborted"))
        for process in LAUNCHERS:
            if process.poll() is None:
                process.kill()
    fatal = [check for check in CHECKS if not check["ok"] and check.get("fatal", True)]
    summary = {
        "ok": failure is None and not fatal,
        "aborted": failure,
        "checks": CHECKS,
        "failed": [check["check"] for check in fatal],
        "launcher_logs": [os.path.join(args.out, name) for name in ("launcher-1.log", "launcher-2.log")],
    }
    with open(os.path.join(args.out, "summary.json"), "w") as handle:
        json.dump(summary, handle, indent=2, default=str)
    print(json.dumps({"ok": summary["ok"], "aborted": failure, "failed": summary["failed"], "checks": len(CHECKS)}))
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
