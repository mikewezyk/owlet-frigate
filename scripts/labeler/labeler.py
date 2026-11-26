#!/usr/bin/env python3
"""Keyboard-driven labeling UI for Frigate's crib_state classification images.

Runs on the Frigate host, listens on 127.0.0.1 only (reach it with an SSH tunnel).
Labels are kept in labels.json next to this file until you press Apply, which then
hands them to Frigate's own API (categorize / train delete). Standard library only.

    python3 labeler.py [--port 8099] [--model crib_state]
"""
import argparse, json, os, random, threading, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
FRIGATE = "http://127.0.0.1:5000"
HEAD = {"Content-Type": "application/json", "remote-user": "admin", "remote-role": "admin"}
LABELS = ["empty", "asleep", "awake"]
LOCK = threading.Lock()


def frigate(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(FRIGATE + path, data, HEAD, method=method)
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def load(name, default):
    try:
        with open(os.path.join(HERE, name)) as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def save(name, obj):
    tmp = os.path.join(HERE, name + ".tmp")
    with open(tmp, "w") as f:
        json.dump(obj, f, indent=1)
    os.replace(tmp, os.path.join(HERE, name))


class App:
    def __init__(self, model):
        self.model = model

    def state(self):
        files = frigate("GET", f"/api/classification/{self.model}/train")
        labels = load("labels.json", {})
        for gone in [f for f in labels if f not in files]:  # Frigate rotated it out
            del labels[gone]
        save("labels.json", labels)
        order = sorted(files, key=lambda f: random.Random(f).random())  # stable shuffle
        todo = [f for f in order if f not in labels]
        try:
            ds = frigate("GET", f"/api/classification/{self.model}/dataset")
            applied = {k: len(v) for k, v in ds.get("categories", {}).items()}
        except Exception:
            applied = {}
        counts = {l: sum(1 for v in labels.values() if v == l) for l in LABELS + ["discard"]}
        return {"total": len(files), "todo": todo[:8], "remaining": len(todo),
                "counts": counts, "applied": applied, "crop": load("crop.json", None)}

    def label(self, f, label):
        labels = load("labels.json", {})
        if label is None:
            labels.pop(f, None)
        elif label in LABELS + ["discard"]:
            labels[f] = label
        save("labels.json", labels)

    def apply(self):
        labels = load("labels.json", {})
        done, failed = 0, []
        discards = [f for f, l in labels.items() if l == "discard"]
        if discards:
            frigate("POST", f"/api/classification/{self.model}/train/delete", {"ids": discards})
            for f in discards:
                labels.pop(f)
            done += len(discards)
        for f, l in list(labels.items()):
            r = frigate("POST", f"/api/classification/{self.model}/dataset/categorize",
                        {"training_file": f, "category": l})
            if r.get("success"):
                labels.pop(f)
                done += 1
            else:
                failed.append(f"{f}: {r.get('message')}")
        save("labels.json", labels)
        return {"applied": done, "failed": failed}


PAGE = open(os.path.join(HERE, "index.html"), "rb").read() if os.path.exists(os.path.join(HERE, "index.html")) else b""


class Handler(BaseHTTPRequestHandler):
    app = None

    def log_message(self, *a):
        pass

    def send(self, code, body, ctype="application/json"):
        if not isinstance(body, bytes):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store" if ctype != "image/webp" else "max-age=3600")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        try:
            if self.path == "/":
                page = open(os.path.join(HERE, "index.html"), "rb").read()
                self.send(200, page, "text/html; charset=utf-8")
            elif self.path == "/api/state":
                with LOCK:
                    self.send(200, self.app.state())
            elif self.path.startswith("/img/"):
                name = os.path.basename(self.path[5:])
                with urllib.request.urlopen(f"{FRIGATE}/clips/{self.app.model}/train/{name}", timeout=20) as r:
                    self.send(200, r.read(), "image/webp")
            else:
                self.send(404, {"error": "not found"})
        except Exception as e:
            self.send(500, {"error": str(e)})

    def do_POST(self):
        try:
            n = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(n) or b"{}")
            with LOCK:
                if self.path == "/api/label":
                    self.app.label(os.path.basename(body["file"]), body.get("label"))
                    self.send(200, {"ok": True})
                elif self.path == "/api/apply":
                    self.send(200, self.app.apply())
                elif self.path == "/api/crop":
                    save("crop.json", body)
                    self.send(200, {"ok": True})
                else:
                    self.send(404, {"error": "not found"})
        except Exception as e:
            self.send(500, {"error": str(e)})


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8099)
    ap.add_argument("--model", default="crib_state")
    a = ap.parse_args()
    Handler.app = App(a.model)
    print(f"labeler on http://127.0.0.1:{a.port}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", a.port), Handler).serve_forever()
