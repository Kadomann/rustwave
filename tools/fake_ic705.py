#!/usr/bin/env python3
"""Simuliertes Icom-Funkgerät (Netzwerkprotokoll wie IC-705/RS-BA1) zum Testen ohne Hardware.

Feldlagen nach den Paketstrukturen von wfview (include/packettypes.h), unabhängig von der
Rust-Implementierung geschrieben. Ein Client gleichzeitig.

  python3 fake_ic705.py --user test --password geheim --rx-wav sstv.wav --tx-wav gesendet.wav --report bericht.json

- Steuerkanal 50001: Anmeldung (prüft Benutzer/Passwort), Token, Fähigkeiten, Verbindungsanfrage
- CI-V 50002: Frequenz setzen/lesen (05/03), Betriebsart (06), PTT (1C 00); verwirft jedes 7. CI-V-Paket
  einmalig und fordert es neu an (prüft die Neuübertragung des Clients)
- Audio 50003: sendet --rx-wav (48 kHz, 16 Bit mono) in 20-ms-Paketen in Schleife;
  zeichnet empfangenes Sendeaudio auf, solange PTT an ist
"""
import random, argparse, json, socket, struct, threading, time, wave

SEQ = [0x47,0x5d,0x4c,0x42,0x66,0x20,0x23,0x46,0x4e,0x57,0x45,0x3d,0x67,0x76,0x60,0x41,0x62,0x39,0x59,0x2d,0x68,0x7e,
       0x7c,0x65,0x7d,0x49,0x29,0x72,0x73,0x78,0x21,0x6e,0x5a,0x5e,0x4a,0x3e,0x71,0x2c,0x2a,0x54,0x3c,0x3a,0x63,0x4f,
       0x43,0x75,0x27,0x79,0x5b,0x35,0x70,0x48,0x6b,0x56,0x6f,0x34,0x32,0x6c,0x30,0x61,0x6d,0x7b,0x2f,0x4b,0x64,0x38,
       0x2b,0x2e,0x50,0x40,0x3f,0x55,0x33,0x37,0x25,0x77,0x24,0x26,0x74,0x6a,0x28,0x53,0x4d,0x69,0x22,0x5c,0x44,0x31,
       0x36,0x58,0x3b,0x7a,0x51,0x5f,0x52]

def passcode(s):
    out = bytearray(16)
    for i, ch in enumerate(s.encode()[:16]):
        p = ch + i
        if p > 126: p = 32 + p % 127
        out[i] = SEQ[p - 32]
    return bytes(out)

RADIO_ADDR = 0xA4
state = {"freq": 7_074_000, "mode": 1, "ptt": False, "login": None, "logins": 0, "ping_replies": 0,
         "retransmit_requested": 0, "retransmit_received": 0, "civ_cmds": [], "tx_frames": 0,
         "streams_opened": False, "civ_open": False, "conninfo": {}, "disconnects": 0, "gps_queries": 0, "power": 128, "power_sets": [], "knob_events": 0, "scope_on": False, "scope_out": False, "scope_frames": 0, "scope_mode": 0, "scope_span": 25000, "scope_edge": 1, "scope_edges": {}, "scope_cmds": [], "ctl": {"pre": 0, "att": 0, "nb": 0, "nr": 1, "agc": 2, "tuner": 1}, "filter": 2, "tune_started": None, "tunes": 0, "vd_raw": 111, "cw": [], "cw_speed": None, "breakin": None, "audio_sent": 0, "tx_played_bytes": 0, "tx_cut_bytes": 0, "audio_dropped": 0, "client_retx_requests": 0, "client_retx_served": 0, "token_renewals": 0}
lock = threading.Lock()

class Chan:
    """Ein UDP-Port des Funkgeräts. Header: len LE32, type LE16, seq LE16, sentid BE32, rcvdid BE32."""
    def __init__(self, port, my_id):
        self.s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.s.bind(("0.0.0.0", port)); self.s.settimeout(0.005)
        self.my_id, self.peer, self.peer_id, self.seq, self.ping_seq = my_id, None, 0, 1, 1
        self.last_ping = time.time()
    def hdr(self, length, typ, seq=None):
        if seq is None: seq = self.seq; self.seq = (self.seq + 1) & 0xffff
        return bytearray(struct.pack("<IHH", length, typ, seq) + struct.pack(">II", self.my_id, self.peer_id) + bytes(length - 16))
    def send(self, p):
        if self.peer: self.s.sendto(bytes(p), self.peer)
    def recv(self):
        try: return self.s.recvfrom(4096)
        except socket.timeout: return None, None
    def common(self, d, addr):
        """Handshake, Ping und Trennen; True = erledigt."""
        ln, typ, seq = struct.unpack("<IHH", d[:8])
        if len(d) == 16 and typ == 0x03:            # are you there
            self.peer, self.peer_id = addr, struct.unpack(">I", d[8:12])[0]
            self.send(self.hdr(16, 0x04, 0)); return True
        if len(d) == 16 and typ == 0x06:            # are you ready
            self.send(self.hdr(16, 0x06, 1)); return True
        if len(d) == 16 and typ == 0x05:
            with lock: state["disconnects"] += 1
            return True
        if len(d) == 21 and typ == 0x07:            # ping
            if d[16] == 0:                          # Anfrage des Clients -> beantworten
                p = self.hdr(21, 0x07, seq); p[16] = 1; p[17:21] = d[17:21]; self.send(p)
            else:
                with lock: state["ping_replies"] += 1
            return True
        if len(d) == 16 and typ == 0x00: return True  # idle
        return False
    def ping(self):
        if self.peer and time.time() - self.last_ping > 0.1:
            p = self.hdr(21, 0x07, self.ping_seq); p[16] = 0; p[17:21] = b"\x01\x02\x03\x04"
            self.ping_seq = (self.ping_seq + 1) & 0xffff; self.send(p); self.last_ping = time.time()

def control(args, stop):
    c = Chan(50001, 0x11223344)
    token, guid = b"\xaa\xbb\xcc\xdd", bytes(range(0x10, 0x20))
    while not stop.is_set():
        c.ping()
        d, addr = c.recv()
        if d is None or c.common(d, addr): continue
        if len(d) == 0x80 and d[0] == 0x80:        # login
            ok = d[0x40:0x50] == passcode(args.user) and d[0x50:0x60] == passcode(args.password) and d[0x60:0x67] == b"icom-pc"
            with lock: state["login"] = "ok" if ok else "falsch"; state["logins"] += 1
            r = c.hdr(0x60, 0); r[0x13] = 0x50; r[0x14] = 2; r[0x16:0x18] = d[0x16:0x18]
            r[0x1a:0x1c] = d[0x1a:0x1c]; r[0x1c:0x20] = token
            r[0x30:0x34] = b"\x00\x00\x00\x00" if ok else b"\xff\xff\xff\xfe"
            r[0x40:0x44] = b"FTTH"; c.send(r)
            if ok:                                  # Fähigkeiten (capabilities) mit einem Funkgerät
                cap = c.hdr(0xa8, 0); cap[0x13] = 0x98; cap[0x14] = 2; cap[0x15] = 2
                cap[0x40:0x42] = struct.pack(">H", 1)
                cap[0x42:0x52] = guid; cap[0x52:0x58] = b"IC-705"; cap[0x72:0x7d] = b"ICOM_VAUDIO"
                cap[0x42 + 0x52] = RADIO_ADDR; c.send(cap)
        elif len(d) == 0x40 and d[0] == 0x40:      # token
            kind = d[0x15]
            if d[0x1c:0x20] != token: print("FEHLER: falsches Token", d[0x1a:0x20].hex())
            if kind == 0x05:
                with lock: state["token_renewals"] += 1
            r = c.hdr(0x40, 0); r[0x13] = 0x30; r[0x14] = 2; r[0x15] = kind; r[0x16:0x18] = d[0x16:0x18]
            r[0x1a:0x20] = d[0x1a:0x20]; c.send(r)
        elif len(d) == 0x90 and d[0] == 0x90:      # conninfo / Stream-Anfrage
            info = {"guid_ok": d[0x20:0x30] == guid, "name": d[0x40:0x60].rstrip(b"\0").decode(),
                    "user_ok": d[0x60:0x70] == passcode(args.user), "rx": d[0x70], "tx": d[0x71],
                    "rxcodec": d[0x72], "txcodec": d[0x73], "rxsample": struct.unpack(">I", d[0x74:0x78])[0],
                    "txsample": struct.unpack(">I", d[0x78:0x7c])[0], "civport": struct.unpack(">I", d[0x7c:0x80])[0],
                    "audioport": struct.unpack(">I", d[0x80:0x84])[0], "txbuffer": struct.unpack(">I", d[0x84:0x88])[0]}
            with lock: state["conninfo"] = info; state["streams_opened"] = True
            r = c.hdr(0x90, 0); r[0x13] = 0x80; r[0x14] = 3; r[0x1a:0x20] = d[0x1a:0x20]
            r[0x20:0x30] = guid; r[0x40:0x46] = b"IC-705"; r[0x60:0x64] = struct.pack("<I", 1)
            r[0x64:0x6b] = b"icom-pc"; c.send(r)

def bcd(v, n):
    s = f"{v:0{2 * n}d}"
    return bytes(int(s[i:i + 2], 16) for i in range(0, 2 * n, 2))

T0 = time.time()
def gps_data(args):
    """27 Byte wie IC-705: optional Fahrt (--gps-kmh) mit 90°-Kurve alle 60 s, Uhr um --gps-clock-offset s verschoben"""
    import math
    t = time.time() - T0
    lat, lon, course = 48 + 8.5 / 60, 11 + 34.75 / 60, 45.0
    v = args.gps_kmh
    if v > 0:
        seg, rest = int(t // 60), t % 60
        for k in range(seg + 1):
            c = (45 + 90 * k) % 360; d = (60 if k < seg else rest) * v / 3.6
            lat += d * math.cos(math.radians(c)) / 111320; lon += d * math.sin(math.radians(c)) / (111320 * math.cos(math.radians(lat)))
            course = c
    def dm(x, deg_digits):
        x = abs(x); d = int(x); m = (x - d) * 60
        mm = int(round(m * 1000))
        return d, mm // 1000, mm % 1000
    d, m, f = dm(lat, 2)
    la = bcd(d, 1) + bcd(m, 1) + bcd(f // 10, 1) + bytes([(f % 10) << 4, 0x01 if lat >= 0 else 0x00])
    d, m, f = dm(lon, 3)
    lo = bytes([d // 100]) + bcd(d % 100, 1) + bcd(m, 1) + bcd(f // 10, 1) + bytes([(f % 10) << 4, 0x01 if lon >= 0 else 0x00])
    alt = bcd(5200 * 100, 4)            # 520,0 m, 0, +
    cs = bcd(int(course), 2); sp = bcd(int(v * 10), 3)
    ts = time.gmtime(time.time() + args.gps_clock_offset)
    dt = bcd(int(time.strftime("%Y%m%d%H%M%S", ts)), 7)
    return la + lo + alt + cs + sp + dt

def civ(args, stop):
    c = Chan(50002, 0x55667788)
    count, dropped = 0, set()
    def reply(frame):
        p = c.hdr(0x15 + len(frame), 0); p[0x10] = 0xc1; p[0x11:0x13] = struct.pack("<H", len(frame))
        p[0x13:0x15] = struct.pack(">H", 0); p[0x15:] = frame; c.send(p)
    def bcd_le(v, n):
        s = f"{v:0{2 * n}d}"
        return bytes(int(s[i - 2:i], 16) for i in range(2 * n, 0, -2))
    t_knob, t_scope, knob_n = time.time() + 6, time.time(), 0
    while not stop.is_set():
        c.ping()
        now = time.time()
        # Knopf am Gerät drehen: Frequenz +1 kHz und (einmal) Leistung ändern; mit „CI-V Transceive“ meldet das Gerät das von selbst
        if args.knob and c.peer and now >= t_knob:
            t_knob = now + 6; knob_n += 1
            with lock:
                state["freq"] += 1000; f = state["freq"]
                if knob_n == 2: state["power"] = 200
                state["knob_events"] += 1
            reply(bytes([0xfe, 0xfe, 0x00, RADIO_ADDR, 0x00]) + bcd_le(f, 5) + b"\xfd")
        # Spektrum (WLAN: alles in einer Meldung); Mittenmodus: Mitte ± Spannweite, Festmodus: Kanten der gewählten Kante.
        # Signale bei −12 und +5 kHz neben der Gerätefrequenz (feste Frequenzen, wandern also im Festmodus nicht mit dem VFO)
        if state.get("scope_out") and c.peer and now >= t_scope:
            t_scope = now + 0.1
            with lock:
                fq, mode, sp = state["freq"], state["scope_mode"], state["scope_span"]
                edges = state["scope_edges"].get(str(state["scope_edge"]))
            if mode == 1 and edges: lo, hi, f1, f2 = edges[0], edges[1], edges[0], edges[1]
            else: mode, lo, hi, f1, f2 = 0, fq - sp, fq + sp, fq, sp
            pts = bytearray(min(160, 18 + random.randint(0, 8)) for _ in range(475))
            for sig in (7_062_000, 7_079_000, 144_800_000, 145_500_000):
                if lo <= sig <= hi:
                    k = int((sig - lo) / (hi - lo) * 474)
                    for j in range(-3, 4):
                        if 0 <= k + j < 475: pts[k + j] = max(pts[k + j], 130 - 15 * abs(j))
            fr = bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0x27, 0x00, 0x00, 0x01, 0x01, mode]) + bcd_le(f1, 5) + bcd_le(f2, 5) + b"\x00" + bytes(pts) + b"\xfd"
            reply(fr)
            with lock: state["scope_frames"] += 1
        d, addr = c.recv()
        if d is None or c.common(d, addr): continue
        if len(d) == 0x16 and d[0x10] == 0xc0:
            with lock: state["civ_open"] = d[0x15] == 0x05
            continue
        if len(d) >= 0x16 and d[0x10] == 0xc1:
            seq = struct.unpack("<H", d[6:8])[0]
            count += 1
            if count % 7 == 0 and seq not in dropped:   # Paket "verlieren" und neu anfordern
                dropped.add(seq)
                with lock: state["retransmit_requested"] += 1
                r = c.hdr(16, 0x01, seq); c.send(r); continue
            if seq in dropped:
                with lock: state["retransmit_received"] += 1
                dropped.discard(seq)
            n = struct.unpack("<H", d[0x11:0x13])[0]
            f = d[0x15:0x15 + n]
            if not (f[:2] == b"\xfe\xfe" and f[2] == RADIO_ADDR and f[-1] == 0xfd): continue
            cmd = f[4]
            with lock: state["civ_cmds"].append(f.hex(" "))
            if cmd == 0x05:
                with lock: state["freq"] = int("".join(f"{b:02x}" for b in reversed(f[5:10])))
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))
            elif cmd == 0x03:
                s = f"{state['freq']:010d}"
                bcd = bytes(int(s[i - 2:i], 16) for i in range(10, 0, -2))
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0x03]) + bcd + b"\xfd")
            elif cmd == 0x06:
                with lock:
                    state["mode"] = f[5]
                    if len(f) > 7: state["filter"] = f[6]
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))
            elif cmd == 0x04:
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0x04, state["mode"], state["filter"], 0xfd]))
            elif cmd == 0x14 and f[5] == 0x0a:
                if len(f) == 9:                         # setzen
                    with lock: state["power"] = int(f[6:8].hex()); state["power_sets"].append(state["power"])
                    reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))
                else:
                    reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0x14, 0x0a]) + bytes.fromhex(f"{state['power']:04d}") + b"\xfd")
            elif cmd == 0x27 and f[5] in (0x14, 0x15, 0x16, 0x1e):
                def val(b): return int(bytes(reversed(b)).hex())
                ok = True
                with lock:
                    if f[5] == 0x14: state["scope_mode"] = f[7]
                    elif f[5] == 0x15: state["scope_span"] = val(f[7:12])
                    elif f[5] == 0x16: state["scope_edge"] = int(f"{f[7]:x}")
                    else:
                        lo, hi = val(f[8:13]), val(f[13:18])
                        ok = hi - lo <= 1_000_000          # Annahme: höchstens 1 MHz
                        if ok: state["scope_edges"][str(int(f"{f[7]:x}"))] = [lo, hi]
                    state["scope_cmds"].append([f"{f[5]:02x}", f[6:-1].hex(), ok])
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb if ok else 0xfa, 0xfd]))
            elif cmd == 0x27 and f[5] in (0x10, 0x11):
                with lock: state["scope_on" if f[5] == 0x10 else "scope_out"] = f[6] == 1
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))
            elif cmd == 0x17:                           # CW-Text (oder FF = Stopp)
                with lock: state["cw"].append([round(time.time() - T0, 2), "<STOP>" if f[5] == 0xff else f[5:-1].decode("ascii")])
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))
            elif cmd == 0x14 and f[5] == 0x0c:
                with lock: state["cw_speed"] = int(f[6:8].hex())
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))
            elif cmd == 0x16 and f[5] == 0x47:
                with lock: state["breakin"] = f[6]
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))
            elif cmd == 0x23 and f[5] == 0x00:          # eigene GPS-Position (Start 48°08.500' N, 011°34.750' E)
                with lock: state["gps_queries"] += 1
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0x23, 0x00]) + gps_data(args) + b"\xfd")
            elif cmd == 0x15 and f[5] in (0x11, 0x12, 0x13, 0x15, 0x16):   # Messwerte: Po, SWR (1,3), ALC – nur beim Senden; Vd, Id
                with lock:
                    state["meter_queries"] = state.get("meter_queries", 0) + 1
                    tuning = state["tune_started"] is not None and time.time() - state["tune_started"] < 3
                    if not tuning and state["tune_started"] is not None: state["ctl"]["tuner"] = 1; state["tune_started"] = None
                    keyed = state["ptt"] or tuning or (state["cw"] and time.time() - T0 - state["cw"][-1][0] < 3)
                    swr = max(20, 120 - int((time.time() - state["tune_started"]) * 40)) if tuning else 30
                    v = {0x11: state["power"] * 213 // 255 if keyed else 0, 0x12: swr if keyed else 0, 0x13: 40 if keyed else 0,
                         0x15: state["vd_raw"], 0x16: 100 if keyed else 15}[f[5]]
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0x15, f[5]]) + bytes.fromhex(f"{v:04d}") + b"\xfd")
            elif (cmd == 0x16 and f[5] in (0x02, 0x22, 0x40, 0x12)) or cmd == 0x11 or (cmd == 0x1c and f[5] == 0x01):
                key = {(0x16, 0x02): "pre", (0x16, 0x22): "nb", (0x16, 0x40): "nr", (0x16, 0x12): "agc", (0x1c, 0x01): "tuner"}.get((cmd, f[5]), "att")
                data = f[5:-1] if cmd == 0x11 else f[6:-1]
                with lock:
                    if data:                                # setzen
                        state["ctl"][key] = data[0]
                        if key == "tuner" and data[0] == 2: state["tune_started"] = time.time(); state["tunes"] += 1
                        r = bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd])
                    else:
                        if key == "tuner" and state["tune_started"] and time.time() - state["tune_started"] >= 3: state["ctl"]["tuner"] = 1; state["tune_started"] = None
                        r = bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR]) + bytes(f[4:6] if cmd != 0x11 else f[4:5]) + bytes([state["ctl"][key]]) + b"\xfd"
                reply(r)
            elif cmd == 0x1c and f[5] == 0x00:
                with lock: state["ptt"] = f[6] == 1
                reply(bytes([0xfe, 0xfe, 0xe0, RADIO_ADDR, 0xfb, 0xfd]))

def audio(args, stop):
    c = Chan(50003, 0x99aabbcc)
    w = wave.open(args.rx_wav); assert w.getframerate() == 48000 and w.getsampwidth() == 2 and w.getnchannels() == 1
    rx = w.readframes(w.getnframes()); pos = 0
    out = wave.open(args.tx_wav, "wb"); out.setnchannels(1); out.setsampwidth(2); out.setframerate(48000)
    nxt, aseq = time.time(), 0
    hist, held = {}, None                           # gesendete Pakete (für Neuübertragung), zurückgehaltenes Paket
    txq, play_t0, played = bytearray(), 0.0, 0         # TX-Puffer des Geräts
    def play_tx():
        nonlocal played
        if not txq: return
        due = int((time.time() - play_t0) * 48000) * 2 - played
        if due <= 0: return
        chunk = bytes(txq[:due]); del txq[:due]; played += len(chunk)
        with lock:
            if state["ptt"]: out.writeframes(chunk); state["tx_played_bytes"] += len(chunk)
            else: state["tx_cut_bytes"] += len(chunk)   # Audio kam an, Sender war aber schon aus
    rnd = random.Random(7)
    while not stop.is_set():
        c.ping()
        play_tx()
        if c.peer and time.time() >= nxt:            # 20 ms RX-Audio senden
            frame = rx[pos:pos + 1920]; pos = (pos + 1920) % (len(rx) - 1920)
            for part in (frame[:1364], frame[1364:]):
                p = c.hdr(0x18 + len(part), 0); p[0x10:0x12] = b"\x80\x00"
                p[0x12:0x14] = struct.pack(">H", aseq); p[0x16:0x18] = struct.pack(">H", len(part)); p[0x18:] = part
                aseq = (aseq + 1) & 0xffff
                seq = struct.unpack("<H", p[6:8])[0]; hist[seq] = bytes(p); hist.pop((seq - 300) & 0xffff, None)
                with lock: state["audio_sent"] += 1
                if rnd.random() < args.loss:          # WLAN verliert das Paket
                    with lock: state["audio_dropped"] += 1
                    continue
                if held is None and rnd.random() < args.reorder: held = p; continue   # kommt später
                c.send(p)
                if held is not None: c.send(held); held = None
            nxt += 0.02
        d, addr = c.recv()
        if d is None or c.common(d, addr): continue
        if len(d) == 16 and struct.unpack("<H", d[4:6])[0] == 0x01:   # Client fordert Paket neu an
            seq = struct.unpack("<H", d[6:8])[0]
            with lock: state["client_retx_requests"] += 1
            if seq in hist:
                c.send(hist[seq])
                with lock: state["client_retx_served"] += 1
            continue
        if len(d) >= 0x20 and struct.unpack("<I", d[:4])[0] == len(d) and d[0x10] == 0x80:
            n = struct.unpack(">H", d[0x16:0x18])[0]
            # wie das echte Gerät: TX-Audio erst puffern (txbuffer ms), dann im Echtzeittakt ausspielen
            if not txq: play_t0 = time.time() + state["conninfo"].get("txbuffer", 0) / 1000.0; played = 0
            txq.extend(d[0x18:0x18 + n])
            with lock: state["tx_frames"] += 1
        play_tx()
    out.close()

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--user", default="test"); ap.add_argument("--password", default="geheim")
    ap.add_argument("--rx-wav", required=True); ap.add_argument("--tx-wav", required=True)
    ap.add_argument("--report", required=True); ap.add_argument("--seconds", type=float, default=600)
    ap.add_argument("--knob", action="store_true", help="alle 6 s am Abstimmknopf drehen, einmal die Leistung ändern")
    ap.add_argument("--gps-kmh", type=float, default=0.0, help="simulierte Fahrt mit diesem Tempo")
    ap.add_argument("--gps-clock-offset", type=float, default=0.0, help="GPS-Zeit gegenüber der echten Zeit (s)")
    ap.add_argument("--loss", type=float, default=0.0, help="Anteil verlorener Audiopakete (WLAN)")
    ap.add_argument("--reorder", type=float, default=0.0, help="Anteil vertauschter Audiopakete")
    a = ap.parse_args()
    stop = threading.Event()
    ts = [threading.Thread(target=f, args=(a, stop)) for f in (control, civ, audio)]
    for t in ts: t.start()
    print("simuliertes IC-705 bereit", flush=True)
    t0 = time.time()
    try:
        while time.time() - t0 < a.seconds:
            time.sleep(0.5)
            with lock: json.dump(state, open(a.report, "w"), indent=1)
    finally:
        stop.set()
        for t in ts: t.join()
        with lock: json.dump(state, open(a.report, "w"), indent=1)
