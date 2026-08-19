#!/usr/bin/env python3
"""Open SPA routes in headless Chrome and report console errors / uncaught exceptions.

Usage: console-errors.py <base-url> <email> <password> <path> [path ...]
Exits non-zero and prints every error it saw. Reuses the CDP plumbing style of
deploy/shot.py (stdlib websocket, no deps) — the point is to catch runtime errors a
`vite build` cannot see, e.g. `const f = (p) = expr` (an assignment, not an arrow fn:
valid JS, ReferenceError at runtime, which blanked the cluster page in 3.0.7).
"""
import json, os, sys, time, subprocess, http.client, urllib.request, urllib.error
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "deploy"))
import importlib.util
spec = importlib.util.spec_from_file_location(
    "shot", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "deploy", "shot.py"))
shot = importlib.util.module_from_spec(spec); spec.loader.exec_module(shot)

CHROME = shot.CHROME
HOST, PORT = shot.HOST, shot.PORT


def main():
    base, email, password, paths = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
    cookie = shot.login(base, email, password)
    if not cookie:
        print("login failed"); return 2
    proc = subprocess.Popen(
        [CHROME, "--headless=new", "--disable-gpu", "--no-sandbox",
         f"--remote-debugging-port={PORT}", "--window-size=1440,1100", "about:blank"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    failures = []
    try:
        ws_url = None
        for _ in range(60):
            try:
                c = http.client.HTTPConnection(HOST, PORT, timeout=1)
                c.request("GET", "/json")
                for t in json.loads(c.getresponse().read()):
                    if t.get("type") == "page":
                        ws_url = t["webSocketDebuggerUrl"]; break
                if ws_url:
                    break
            except Exception:
                time.sleep(0.25)
        if not ws_url:
            print("no chrome target"); return 2
        s = shot.ws_connect(ws_url)
        i = 0
        def nxt():
            nonlocal i; i += 1; return i
        shot.cmd(s, nxt(), "Network.enable")
        shot.cmd(s, nxt(), "Network.setCookie", {"name": "session", "value": cookie, "url": base})
        shot.cmd(s, nxt(), "Runtime.enable")
        shot.cmd(s, nxt(), "Console.enable")
        shot.cmd(s, nxt(), "Page.enable")
        for path in paths:
            shot.cmd(s, nxt(), "Page.navigate", {"url": base + path})
            errs, deadline = [], time.time() + 6
            while time.time() < deadline:
                s.settimeout(max(0.2, deadline - time.time()))
                try:
                    msg = json.loads(shot.ws_recv(s))
                except Exception:
                    break
                m = msg.get("method")
                if m == "Runtime.exceptionThrown":
                    d = msg["params"]["exceptionDetails"]
                    errs.append((d.get("exception", {}) or {}).get("description")
                                or d.get("text", "exception"))
                elif m == "Console.messageAdded" and msg["params"]["message"].get("level") == "error":
                    errs.append(msg["params"]["message"].get("text", "console error"))
            s.settimeout(None)
            first = [e.splitlines()[0] for e in errs]
            # Ignore benign network noise (a 404 favicon etc.); keep real JS errors.
            first = [e for e in first if "favicon" not in e]
            print(f"{'FAIL' if first else 'ok  '}  {path}" + (f"  <- {first[0]}" if first else ""))
            if first:
                failures.append((path, first))
    finally:
        proc.terminate()
    if failures:
        print("\nconsole errors:")
        for path, errs in failures:
            for e in errs:
                print(f"  {path}: {e}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
