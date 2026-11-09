"""Native touch checks of the runtime's display server (native-touch.patch).

Imported by conformance_probe.py; also runs standalone against any RFB
socket (`touch_probe.py SOCKET [--page] [--off]`). It speaks RFB 3.8 with
security None and never requests a full framebuffer update, only 1x1 Raw
rectangles to read the probe page's colours, so it needs no decoder.

Wire extension under test: pseudo-encoding -1883 in SetEncodings, answered
once by server message 192 (u8 version 1, u8 contacts 10); client message
193 (u8 state 0 begin / 1 update / 2 end, u32 id, u16 x, u16 y, big endian).
"""
import http.server
import json
import socket
import struct
import sys
import threading
import time

TOUCH_ENCODING = -1883
ACK = 192
TOUCH = 193
BEGIN, UPDATE, END = 0, 1, 2

# The probe page paints what it saw into two halves of the viewport, every
# value a multiple of 8 so colour handling cannot blur it:
#   left  = (touchstart, touchend + touchcancel, touchmove) counts
#   right = (most simultaneous touches, active touches, last clientX / 8)
#   bottom 24 px stripe = capability bits (maxTouchPoints >= 10, any-pointer
#   coarse, any-pointer fine, any-hover hover) as red/green/blue/white blocks
PAGE = """<!doctype html><meta charset=utf-8><title>touch probe</title>
<style>html,body{margin:0;height:100%;overflow:hidden;touch-action:none}
#l,#r{position:fixed;top:0;bottom:24px;width:50%}#l{left:0}#r{right:0}
#c{position:fixed;bottom:0;height:24px;left:0;right:0;display:flex}#c div{flex:1}</style>
<div id=c><div></div><div></div><div></div><div></div></div><div id=l></div><div id=r></div>
<script>
let s=0,e=0,m=0,most=0,active=0,lastX=0;
const v=n=>Math.min(n,31)*8;
const paint=()=>{l.style.background=`rgb(${v(s)},${v(e)},${v(m)})`;
r.style.background=`rgb(${v(most)},${v(active)},${Math.min(255,Math.round(lastX/8))})`;};
const caps=[navigator.maxTouchPoints>=10,matchMedia('(any-pointer: coarse)').matches,
matchMedia('(any-pointer: fine)').matches,matchMedia('(any-hover: hover)').matches];
const blocks=document.querySelectorAll('#c div');
caps.forEach((on,i)=>blocks[i].style.background=on?'rgb(248,248,248)':'rgb(0,0,0)');
const track=t=>{active=t.touches.length;most=Math.max(most,active);
if(t.changedTouches.length)lastX=t.changedTouches[0].clientX;paint();};
addEventListener('touchstart',t=>{s+=t.changedTouches.length;track(t)},{passive:false});
addEventListener('touchmove',t=>{m++;track(t);t.preventDefault()},{passive:false});
addEventListener('touchend',t=>{e+=t.changedTouches.length;track(t)},{passive:false});
addEventListener('touchcancel',t=>{e+=t.changedTouches.length;track(t)},{passive:false});
paint();
</script>"""



def serve_page():
    """Serve PAGE on a loopback port (the launcher navigates only to web
    URLs); returns the URL and the server to shut down."""
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802 - http.server API
            body = PAGE.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/touch", server


class Client:
    """RFB 3.8 client that reads only small server messages."""

    def __init__(self, path, encodings):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(15)
        self.sock.connect(path)
        self.exact(12)
        self.sock.sendall(b"RFB 003.008\n")
        count = self.exact(1)[0]
        if 1 not in self.exact(count):
            raise RuntimeError("security type None not offered")
        self.sock.sendall(b"\x01")
        if struct.unpack(">I", self.exact(4))[0] != 0:
            raise RuntimeError("security result failed")
        self.sock.sendall(b"\x01")
        self.width, self.height = struct.unpack(">HH", self.exact(4))
        fmt = self.exact(16)
        self.bpp = fmt[0] // 8
        self.big_endian = fmt[2] != 0
        self.max = struct.unpack(">HHH", fmt[4:10])
        self.shift = fmt[10:13]
        self.exact(struct.unpack(">I", self.exact(4))[0])
        self.sock.sendall(struct.pack(">BxH", 2, len(encodings)) + b"".join(struct.pack(">i", e) for e in encodings))

    def exact(self, count):
        data = b""
        while len(data) < count:
            chunk = self.sock.recv(count - len(data))
            if not chunk:
                raise EOFError("rfb socket closed")
            data += chunk
        return data

    def next_message(self, timeout):
        """One server message; None on timeout."""
        self.sock.settimeout(timeout)
        try:
            kind = self.exact(1)[0]
        except socket.timeout:
            return None
        finally:
            self.sock.settimeout(15)
        if kind == ACK:
            version, contacts = self.exact(2)
            return {"type": "touch_ack", "version": version, "contacts": contacts}
        if kind == 0:
            self.exact(1)
            (count,) = struct.unpack(">H", self.exact(2))
            pixels = []
            for _ in range(count):
                x, y, w, h, encoding = struct.unpack(">HHHHi", self.exact(12))
                if encoding != 0:
                    raise RuntimeError(f"unexpected encoding {encoding}")
                pixels.append(self.exact(w * h * self.bpp))
            return {"type": "update", "pixels": pixels}
        if kind == 2:
            return {"type": "bell"}
        if kind == 3:
            self.exact(3)
            (length,) = struct.unpack(">i", self.exact(4))
            self.exact(abs(length))
            return {"type": "cuttext"}
        raise RuntimeError(f"unexpected server message {kind}")

    def touch(self, state, ident, x, y):
        self.sock.sendall(struct.pack(">BBIHH", TOUCH, state, ident, x, y))

    def pixel(self, x, y):
        """The (r, g, b) at framebuffer (x, y), 0-255."""
        self.sock.sendall(struct.pack(">BBHHHH", 3, 0, x, y, 1, 1))
        deadline = time.time() + 10
        while time.time() < deadline:
            message = self.next_message(deadline - time.time())
            if message and message["type"] == "update" and message["pixels"]:
                raw = message["pixels"][0]
                value = int.from_bytes(raw, "big" if self.big_endian else "little")
                return tuple(
                    ((value >> shift) & top) * 255 // top for shift, top in zip(self.shift, self.max)
                )
        raise RuntimeError("no pixel update")

    def alive(self):
        """Whether the server still answers on this connection."""
        try:
            self.pixel(0, 0)
            return True
        except (EOFError, OSError, RuntimeError):
            return False

    def close(self):
        self.sock.close()


def wait_ack(client, timeout=3):
    deadline = time.time() + timeout
    while time.time() < deadline:
        message = client.next_message(deadline - time.time())
        if message and message["type"] == "touch_ack":
            return message
    return None


def protocol_checks(path, record, accept=True):
    """Negotiation and robustness, no page needed. With accept=False the
    server runs without AcceptTouchEvents and must look stock."""
    stock = Client(path, [0])
    record("stock_client_gets_no_touch_ack", wait_ack(stock, 1.5) is None)
    stock.sock.sendall(struct.pack(">BBHH", 5, 0, 10, 10))
    record("stock_client_pointer_still_works", stock.alive())
    asking = Client(path, [0, TOUCH_ENCODING])
    ack = wait_ack(asking)
    if not accept:
        record("touch_ack_withheld_when_disabled", ack is None, ack=ack)
        asking.touch(BEGIN, 1, 10, 10)
        record("touch_without_ack_is_an_unknown_message", not asking.alive())
        record("stock_client_survives_other_disconnect", stock.alive())
        stock.close()
        return
    record("touch_ack", bool(ack) and ack["version"] == 1 and ack["contacts"] == 10, ack=ack)
    # A client that was never acknowledged gets the stock answer to 193.
    rogue = Client(path, [0])
    rogue.touch(BEGIN, 1, 10, 10)
    record("unnegotiated_touch_disconnects_like_stock", not rogue.alive())
    # An invalid state is a protocol error for that connection only.
    bad = Client(path, [0, TOUCH_ENCODING])
    wait_ack(bad)
    bad.sock.sendall(struct.pack(">BBIHH", TOUCH, 3, 1, 10, 10))
    record("invalid_touch_state_disconnects", not bad.alive())
    # Out-of-range coordinates are clamped, unknown ids ignored.
    asking.touch(BEGIN, 7, 65535, 65535)
    asking.touch(UPDATE, 99, 10, 10)
    asking.touch(END, 98, 10, 10)
    asking.touch(END, 7, 65535, 65535)
    record("out_of_range_and_unknown_ids_tolerated", asking.alive())
    record("stock_client_unaffected", stock.alive())
    asking.close()
    stock.close()


def page_checks(path, record, settle=0.6):
    """Chrome must already show PAGE (serve_page) on this display."""
    client = Client(path, [0, TOUCH_ENCODING])
    if not wait_ack(client):
        record("page_touch_ack", False)
        return
    w, h = client.width, client.height
    left, right = (w // 4, h * 3 // 4), (w * 3 // 4, h * 3 // 4)
    caps_y = h - 12
    samples = [client.pixel(w * (2 * i + 1) // 8, caps_y) for i in range(4)]
    halves = [client.pixel(*left), client.pixel(*right)]
    found = halves == [(0, 0, 0), (0, 0, 0)] and all(
        sample == (0, 0, 0) or min(sample) >= 240 for sample in samples
    )
    record("page_found", found, halves=halves, capabilities=samples)
    if not found:
        client.close()
        return
    caps = [min(sample) >= 240 for sample in samples]
    record(
        "page_sees_touchscreen_laptop",
        caps == [True, True, True, True],
        max_touch_points_10=caps[0], any_pointer_coarse=caps[1], any_pointer_fine=caps[2], any_hover=caps[3],
    )
    def read():
        time.sleep(settle)
        a, b = client.pixel(*left), client.pixel(*right)
        def n(value):
            return round(value / 8)
        return {"start": n(a[0]), "end": n(a[1]), "move": n(a[2]), "most": n(b[0]), "active": n(b[1]), "x8": b[2]}

    y = h * 3 // 4
    client.touch(BEGIN, 1, w // 2, y)
    for step in range(1, 6):
        client.touch(UPDATE, 1, w // 2, y - 20 * step)
        time.sleep(0.02)
    client.touch(END, 1, w // 2, y - 100)
    state = read()
    record("touch_start_move_end", state["start"] == 1 and state["end"] == 1 and state["move"] >= 1 and state["active"] == 0, state=state)
    record("touch_coordinates_mapped", abs(state["x8"] * 8 - w // 2) <= 8, last_x=state["x8"] * 8, expected=w // 2)
    client.touch(BEGIN, 2, w // 2 - 40, y)
    client.touch(BEGIN, 3, w // 2 + 40, y)
    for step in range(1, 6):
        client.touch(UPDATE, 2, w // 2 - 40 - 15 * step, y)
        client.touch(UPDATE, 3, w // 2 + 40 + 15 * step, y)
        time.sleep(0.02)
    mid = read()
    client.touch(END, 2, w // 2 - 115, y)
    client.touch(END, 3, w // 2 + 115, y)
    state = read()
    record("two_finger_pinch", mid["active"] == 2 and state["most"] == 2 and state["start"] == 3 and state["end"] == 3 and state["active"] == 0, during=mid, after=state)
    # A connection that drops with contacts down must not leave them stuck.
    leaver = Client(path, [0, TOUCH_ENCODING])
    wait_ack(leaver)
    for ident in (1, 2, 3):
        leaver.touch(BEGIN, ident, w // 4 + 60 * ident, y)
    held = read()
    leaver.close()
    released = read()
    record("touches_end_when_their_connection_closes", held["active"] == 3 and released["active"] == 0, held=held, released=released)
    before = released["start"]
    client.touch(BEGIN, 1, w // 2, y)
    client.touch(END, 1, w // 2, y)
    after = read()
    record("no_stuck_touch_after_reconnect", after["start"] == before + 1 and after["active"] == 0, state=after)
    # At most ten contacts at once; the eleventh is ignored.
    for ident in range(100, 111):
        client.touch(BEGIN, ident, 40 + 50 * (ident - 100), y)
    crowded = read()
    for ident in range(100, 111):
        client.touch(END, ident, 40 + 50 * (ident - 100), y)
    cleared = read()
    record("contacts_bounded_to_ten", crowded["active"] == 10 and cleared["active"] == 0, crowded=crowded, cleared=cleared)
    client.close()


def main():
    path = sys.argv[1]
    checks = []

    def record(name, ok, **detail):
        checks.append({"check": name, "ok": bool(ok), **detail})

    if "--off" in sys.argv:
        protocol_checks(path, record, accept=False)
    else:
        protocol_checks(path, record)
        if "--page" in sys.argv:
            page_checks(path, record)
    print(json.dumps({"ok": all(check["ok"] for check in checks), "checks": checks}, indent=1))
    return 0 if all(check["ok"] for check in checks) else 1


if __name__ == "__main__":
    sys.exit(main())
