#!/usr/bin/env python3
"""Exercise the real CLI/helper with a fake cloudflared; no public network.

cargo build --locked -p skiff --example fake-cloudflared
python3 tests/cloudflare-share.py target/debug/skiff target/debug/examples/fake-cloudflared
"""
import json
import http.server
import ssl
import os
from pathlib import Path
import queue
import signal
import shutil
import subprocess
import sys
import tempfile
import threading
import time

BINARY = str(Path(sys.argv[1]).resolve())
FIXTURE = str(Path(sys.argv[2]).resolve())


def eventually(fn, timeout=8):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if fn():
            return
        time.sleep(.05)
    raise AssertionError("condition did not become true")


def alive(pid):
    if os.name == "nt":
        # os.kill(pid, 0) terminates processes on Windows. Query a handle instead.
        import ctypes
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel.WaitForSingleObject.restype = wintypes.DWORD
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE
        if not handle:
            error = ctypes.get_last_error()
            if error == 87:  # ERROR_INVALID_PARAMETER: PID no longer exists
                return False
            raise ctypes.WinError(error)
        try:
            return kernel.WaitForSingleObject(handle, 0) == 258  # WAIT_TIMEOUT
        finally:
            kernel.CloseHandle(handle)
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def interrupt(process):
    if os.name == "nt":
        # CI has no interactive console. Terminate the owner and verify the
        # detached helper still reaps its child on stdin EOF.
        process.terminate()
    else:
        os.killpg(process.pid, signal.SIGINT)


class Share:
    def __init__(self, env, *args):
        self.process = subprocess.Popen([BINARY, "share", "3000", *args, "--json"], env=env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.events = queue.Queue()
        self.seen = []

        def read():
            for line in self.process.stdout:
                try:
                    self.events.put(json.loads(line))
                except ValueError:
                    self.events.put({"bad": line})
        threading.Thread(target=read, daemon=True).start()

    def event(self, name, timeout=10):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                e = self.events.get(timeout=max(.01, end - time.monotonic()))
            except queue.Empty:
                break
            self.seen.append(e)
            assert "bad" not in e, e
            if e.get("event") == name:
                return e
            assert e.get("event") != "ended", e
        raise AssertionError(f"no {name} event: {self.seen}")


def main():
    with tempfile.TemporaryDirectory(prefix="skiff-cf-test-") as temp:
        root = Path(temp)
        fake = root / ("cloudflared.exe" if os.name == "nt" else "cloudflared")
        shutil.copyfile(FIXTURE, fake)
        fake.chmod(0o700)
        config = root / "share.json"
        env = {**os.environ, "SKIFF_SHARE_CONFIG": str(config), "SKIFF_CF_TEST": temp,
               "TUNNEL_TOKEN": "unrelated-token-must-be-removed", "TUNNEL_FORCE_PROVISIONING_DNS": "true"}
        # A local TLS edge exercises Skiff management without contacting Custom relay.
        cert, key = root / "edge.pem", root / "edge.key"
        openssl_config = root / "openssl.cnf"
        openssl_config.write_text("[req]\ndistinguished_name=dn\nx509_extensions=ext\n[dn]\n[ext]\nsubjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\n")
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=localhost", "-config", str(openssl_config), "-keyout", str(key), "-out", str(cert)], check=True, capture_output=True)
        edge_items, edge_tokens = [], []
        edge_status = 200
        class Edge(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                edge_tokens.append(self.headers.get("Authorization"))
                if edge_status is None:
                    self.close_connection = True
                    return
                self.send_response(edge_status)
                self.end_headers()
                self.wfile.write(json.dumps({"items": edge_items, "limit": 3}).encode())
            def do_DELETE(self):
                # Consume the host metadata before closing the TLS connection;
                # unread request bytes can make the socket close with a reset.
                self.rfile.read(int(self.headers.get("Content-Length", 0)))
                if edge_status == 401:
                    self.send_response(401)
                    self.end_headers()
                    return
                name = self.path.rsplit("/", 1)[-1]
                edge_items[:] = [item for item in edge_items if item["name"] != name]
                self.send_response(204)
                self.end_headers()
            def log_message(self, *args): pass
        edge = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Edge)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.load_cert_chain(cert, key)
        edge.socket = tls.wrap_socket(edge.socket, server_side=True)
        threading.Thread(target=edge.serve_forever, daemon=True).start()
        env.update(SKIFF_SHARE_SERVER=f"localhost:{edge.server_port}", SKIFF_SHARE_CONNECT=f"127.0.0.1:{edge.server_port}", SKIFF_SHARE_CA=str(cert), SKIFF_SHARE_TOKEN="ks_ephemeral_never_save")
        children = []

        def save(extra=None):
            config.write_text(json.dumps({"provider": "cloudflare", "cloudflare": {"executable": str(fake), **(extra or {})}}))

        def cli(*args, ok=True):
            p = subprocess.run([BINARY, "share", *args], env=env, capture_output=True, text=True, timeout=10)
            assert "ks_ephemeral_never_save" not in config.read_text(), "environment credential was persisted"
            if ok:
                assert p.returncode == 0, (p.stdout, p.stderr)
            return p

        def start(*args):
            share = Share(env, *args)
            children.append(share.process)
            return share

        def captures():
            return [json.loads(p.read_text()) for p in root.glob("[0-9]*.json")]

        def closed(capture):
            eventually(lambda: not alive(capture["pid"]))
            eventually(lambda: not Path(capture["runtime"]).exists())

        try:
            default = subprocess.run([BINARY, "share", "status", "--json"], env=env, capture_output=True, text=True, timeout=10)
            assert default.returncode == 0, default.stderr
            assert json.loads(default.stdout)["provider"] == "cloudflare"
            assert not edge_tokens, "a fresh install must not contact a hosted relay"
            print("PASS fresh installs default to Cloudflare without contacting a relay")
            save()
            unrelated = root / "keep-this-directory"
            unrelated.mkdir()
            marker = unrelated / "keep.txt"
            marker.write_text("keep")
            rejected = subprocess.run([BINARY, "__share-cloudflared", str(unrelated)], env=env, capture_output=True, text=True, timeout=5)
            assert rejected.returncode != 0 and marker.read_text() == "keep"
            assert json.loads(cli("status", "--json").stdout)["provider"] == "cloudflare"
            share = start()
            assert share.event("live")["url"] == "https://test-river.trycloudflare.com"
            capture = captures()[-1]
            assert capture["token"] is None
            assert capture["config"] == {}
            assert capture["args"][-2:] == ["--url", "http://localhost:3000"]
            (root / "offline").touch()
            share.event("reconnecting")
            (root / "offline").unlink()
            share.event("live")
            entries = json.loads(cli("list", "--json").stdout)["items"]
            assert len(entries) == 1 and entries[0]["kind"] == "cli", entries
            cli("stop", entries[0]["name"])
            assert share.event("ended")["error"] is False
            assert share.process.wait(timeout=5) == 0
            closed(capture)
            assert json.loads(cli("list", "--json").stdout)["items"] == []
            print("PASS temporary URL, readiness/reconnect, list and stop")

            before = {c["pid"] for c in captures()}
            share = start()
            share.event("live")
            capture = next(c for c in captures() if c["pid"] not in before)
            edge_items.append({"name": "edge-link", "url": "https://edge-link.example.test", "kind": "cli"})
            for provider in ("relay", "cloudflare"):
                cli("provider", provider)
                entries = json.loads(cli("list", "--json").stdout)["items"]
                assert len(entries) == 2, cli("list", "--json").stdout
                cloudflare_id = next(e["name"] for e in entries if e["name"].startswith("cloudflare:"))
            cli("stop", "edge-link")
            assert not edge_items, "Skiff link was not closed while Cloudflare was selected"
            assert edge_tokens and set(edge_tokens) == {"Bearer ks_ephemeral_never_save"}
            cli("provider", "relay")
            edge_status = 401
            entries = json.loads(cli("list", "--json").stdout)
            assert len(entries["items"]) == 1 and entries["warning"]
            for provider in ("relay", "cloudflare"):
                cli("provider", provider)
                for status in (404, 401, 503, None):
                    edge_status = status
                    before_requests = len(edge_tokens)
                    result = cli("list", "--json")
                    entries = json.loads(result.stdout)
                    assert len(edge_tokens) > before_requests, "existing Skiff links must still be queried"
                    assert [e["name"] for e in entries["items"]] == [cloudflare_id]
                    assert entries["unsupported"] == (status == 404)
                    assert bool(entries["warning"]) == (provider == "relay" and status != 404), (provider, status, entries)
                    if provider == "cloudflare" or status == 404:
                        assert not result.stderr and "run `skiff share login`" not in result.stdout
                        assert not cli("list").stderr, "background failures must not become CLI warnings"
                    if provider == "cloudflare" and status == 401:
                        stopped = cli("stop", "expired-edge-link", ok=False)
                        assert stopped.returncode != 0
                        assert "switch to Custom relay" in stopped.stderr
                        assert "run `skiff share login`" not in stopped.stderr
            print("PASS unsupported edges stay silent; stale logins and network failures do not interrupt Cloudflare")
            cli("provider", "relay")
            env.pop("SKIFF_SHARE_TOKEN")
            assert len(json.loads(cli("list", "--json").stdout)["items"]) == 1
            cli("stop", cloudflare_id)
            assert share.event("ended")["error"] is False
            share.process.wait(timeout=5)
            closed(capture)
            edge_status = 200
            env["SKIFF_SHARE_TOKEN"] = "ks_ephemeral_never_save"
            cli("provider", "cloudflare")
            assert "token" not in json.loads(config.read_text())
            print("PASS provider-independent listing/stopping, expired and absent Skiff credentials")

            before = {c["pid"] for c in captures()}
            share = start()
            share.event("live")
            capture = next(c for c in captures() if c["pid"] not in before)
            interrupt(share.process)
            share.process.wait(timeout=5)
            closed(capture)
            print("PASS owner termination reaps cloudflared and removes runtime files" if os.name == "nt" else "PASS terminal Ctrl+C reaps cloudflared and removes runtime files")

            before = {c["pid"] for c in captures()}
            share = start()
            share.event("live")
            capture = next(c for c in captures() if c["pid"] not in before)
            share.process.kill()
            share.process.wait(timeout=5)
            closed(capture)
            print("PASS forced parent termination closes and reaps cloudflared")

            before = {c["pid"] for c in captures()}
            (root / "offline").touch()
            share = start()
            share.event("connecting")
            eventually(lambda: any(c["pid"] not in before for c in captures()))
            capture = next(c for c in captures() if c["pid"] not in before)
            cli("stop", Path(capture["runtime"]).name)
            assert share.event("ended")["error"] is False
            assert not any(e.get("event") == "live" for e in share.seen)
            share.process.wait(timeout=5)
            closed(capture)
            (root / "offline").unlink()
            print("PASS cancellation before readiness and helper directory validation")

            tunnel = "01234567-89ab-cdef-0123-456789abcdef"
            credentials = root / "credentials with spaces.json"
            credentials.write_text(json.dumps({"TunnelID": tunnel, "TunnelSecret": "not-a-real-secret"}))
            original = json.loads(config.read_text())
            original.update(provider="relay", token="ks_preserved", login="test-user")
            config.write_text(json.dumps(original))
            added = cli("domain", "add", " Example.COM. ", "WEB", str(credentials))
            assert "https://web.example.com" in added.stdout
            assert f"{tunnel}.cfargotunnel.com" in added.stdout
            saved = json.loads(config.read_text())
            assert saved["provider"] == "cloudflare"
            assert saved["cloudflare"]["executable"] == str(fake)
            assert saved["cloudflare"]["base_domain"] == "example.com"
            assert Path(saved["cloudflare"]["tunnels"]["web"]["credentials_file"]).samefile(credentials)
            assert (saved["token"], saved["login"], saved["device"]) == (original["token"], original["login"], original["device"])
            invalid = root / "invalid.json"
            invalid.write_text('{"TunnelID":"bad-id","TunnelSecret":"private-token-must-not-leak"}')
            malformed = root / "malformed.json"
            malformed.write_text("private-token-must-not-leak")
            before_config = config.read_bytes()
            for base, name, path in [
                ("https://example.com", "web", credentials),
                ("*.example.com", "web", credentials),
                ("other.example.com", "web", credentials),
                ("example.com", "bad/name", credentials),
                ("example.com", "duplicate", credentials),
                ("example.com", "web", invalid),
                ("example.com", "web", malformed),
                ("example.com", "web", root / "missing.json"),
            ]:
                failed = cli("domain", "add", base, name, str(path), ok=False)
                assert failed.returncode != 0, (base, name, path)
                assert "private-token-must-not-leak" not in failed.stdout + failed.stderr
                assert config.read_bytes() == before_config, "invalid setup changed the saved config"
            other = root / "api.json"
            other_id = "11234567-89ab-cdef-0123-456789abcdef"
            other.write_text(json.dumps({"TunnelID": other_id.upper(), "TunnelSecret": "not-a-real-secret"}))
            cli("domain", "add", "example.com", "api", str(other))
            configured = json.loads(cli("domain", "list", "--json").stdout)
            assert [r["hostname"] for r in configured["routes"]] == ["api.example.com", "web.example.com"]
            assert configured["routes"][0]["id"] == other_id
            assert "not-a-real-secret" not in config.read_text() + json.dumps(configured)
            print("PASS custom-domain setup, normalization, preservation, DNS instructions and invalid-input rollback")
            before = {c["pid"] for c in captures()}
            share = start("--name", "web")
            assert share.event("live")["url"] == "https://web.example.com"
            capture = next(c for c in captures() if c["pid"] not in before)
            assert capture["config"]["ingress"] == [{"hostname": "web.example.com", "service": "http://localhost:3000"}, {"service": "http_status:404"}]
            assert "not-a-real-secret" not in json.dumps(capture)
            duplicate = start("--name", "web")
            assert "already running" in duplicate.event("ended")["reason"]
            assert duplicate.process.wait(timeout=5) != 0
            missing = cli("3000", "--name", "missing", "--json", ok=False)
            assert missing.returncode != 0 and "configure cloudflare.tunnels.missing" in missing.stdout
            assert cli("login", ok=False).returncode != 0
            entry = json.loads(cli("list", "--json").stdout)["items"][0]
            cli("stop", entry["name"])
            assert share.event("ended")["error"] is False
            share.process.wait(timeout=5)
            closed(capture)
            print("PASS named ingress, credential isolation, duplicate lock and missing-name errors")

            cli("domain", "remove", "WEB")
            configured = json.loads(cli("domain", "list", "--json").stdout)
            assert [r["name"] for r in configured["routes"]] == ["api"]
            before = {c["pid"] for c in captures()}
            share = start("--name", "api")
            assert share.event("live")["url"] == "https://api.example.com"
            capture = next(c for c in captures() if c["pid"] not in before)
            # Editing the saved routes must not interrupt an already live link.
            cli("domain", "clear")
            assert alive(capture["pid"])
            assert json.loads(cli("domain", "list", "--json").stdout) == {"base_domain": None, "routes": []}
            entry = json.loads(cli("list", "--json").stdout)["items"][0]
            cli("stop", entry["name"])
            assert share.event("ended")["error"] is False
            share.process.wait(timeout=5)
            closed(capture)
            assert credentials.exists() and other.exists()
            assert json.loads(config.read_text())["cloudflare"]["executable"] == str(fake)
            print("PASS multiple custom hosts, route removal and returning to temporary links")

            def setups():
                return [json.loads(p.read_text()) for p in root.glob("setup-*.json")]

            connected = cli("domain", "connect", "example.com", "--json")
            assert "https://dash.cloudflare.com/argotunnel?" in connected.stdout
            assert "evil.example" not in connected.stdout
            assert "secret-must-not-leak" not in connected.stdout + connected.stderr
            first = cli("domain", "create", "example.com", "auto", "--json")
            assert json.loads(first.stdout.splitlines()[-1])["result"]["hostname"] == "auto.example.com"
            saved = json.loads(config.read_text())
            auto = saved["cloudflare"]["tunnels"]["auto"]
            assert "cloudflare-managed" in auto["credentials_file"]
            creation_count = sum(s["command"][0] == "create" for s in setups())
            cli("domain", "create", "example.com", "auto", "--json")
            assert sum(s["command"][0] == "create" for s in setups()) == creation_count
            assert json.loads(config.read_text())["cloudflare"]["tunnels"]["auto"] == auto
            before_config = config.read_bytes()
            (root / "wrong_zone").touch()
            assert cli("domain", "create", "example.com", "wrong-zone", "--json", ok=False).returncode != 0
            assert config.read_bytes() == before_config
            (root / "wrong_zone").unlink()
            before_config = config.read_bytes()
            (root / "fail_dns").touch()
            failed = cli("domain", "create", "example.com", "retry", "--json", ok=False)
            assert failed.returncode != 0
            assert "Existing records were not replaced" in failed.stderr
            assert "secret-must-not-leak" not in failed.stdout + failed.stderr
            assert config.read_bytes() == before_config
            creation_count = sum(s["command"][0] == "create" for s in setups())
            (root / "fail_dns").unlink()
            cli("domain", "create", "example.com", "retry", "--json")
            assert sum(s["command"][0] == "create" for s in setups()) == creation_count
            for s in setups():
                assert Path(s["home"]).parent.parent.parent.samefile(root)
                assert "cloudflare-managed" in s["home"]
                assert s["token"] is None and s["force_dns"] is None
            assert "secret-must-not-leak" not in config.read_text()
            print("PASS browser authorization, isolated credentials, automated DNS and idempotent retries")

            (root / "hold_login").touch()
            before = {s["pid"] for s in setups()}
            login = subprocess.Popen([BINARY, "share", "domain", "connect", "cancel.example.com", "--json"], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
            children.append(login)
            eventually(lambda: any(s["pid"] not in before for s in setups()))
            capture = next(s for s in setups() if s["pid"] not in before)
            blocked = cli("domain", "create", "example.com", "blocked", ok=False)
            assert blocked.returncode != 0 and "another Cloudflare setup" in blocked.stderr
            interrupt(login); login.wait(timeout=5)
            closed(capture)
            assert not (Path(capture["home"]) / ".cloudflared/cert.pem").exists()
            (root / "hold_login").unlink()
            print("PASS interrupted browser authorization reaps the setup process")

            assert json.loads(config.read_text())["token"] == "ks_preserved"
            print("PASS domain edits and automated setup preserve saved login under an environment override")

            save({"executable": "cloudflared"})
            env["PATH"] = str(root)
            before = {c["pid"] for c in captures()}
            share = start()
            share.event("live")
            capture = next(c for c in captures() if c["pid"] not in before)
            assert Path(json.loads((Path(capture["runtime"]) / "plan.json").read_text())["executable"]).samefile(fake)
            interrupt(share.process)
            share.process.wait(timeout=5)
            closed(capture)
            cli("domain", "connect", "path.example.com", "--json")
            env["PATH"] = os.environ.get("PATH", "")
            print("PASS bare executable discovery for sharing and browser setup")

            save()
            (root / "fail").touch()
            share = start()
            ended = share.event("ended")
            assert ended["error"] is True
            assert share.process.wait(timeout=5) != 0
            assert "private-token-must-not-leak" not in json.dumps(share.seen)
            (root / "fail").unlink()
            cli("provider", "relay")
            assert json.loads(config.read_text())["provider"] == "relay"
            print("PASS child failure, log redaction and switching providers")
        finally:
            edge.shutdown()
            edge.server_close()
            for child in children:
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)
            # A failed assertion must still leave no tunnel fixture behind.
            for capture in captures():
                if alive(capture["pid"]):
                    eventually(lambda c=capture: not alive(c["pid"]))


if __name__ == "__main__":
    main()
