#!/usr/bin/env python3
"""Mint a reusable Mattermost session token, once, so the password leaves the loop.

Mattermost session tokens are long-lived (server default 30 days, refreshed by
activity), which is exactly how the official desktop app avoids logging in at
every startup. Minting one here lets the Rust probes -- and Claude driving them
-- authenticate without anyone re-entering a password.

Your password is read with getpass, sent only to the server, and never written
anywhere. The token IS a bearer credential worth the same care as a password:
it lands in a gitignored file, and `--revoke` kills it server-side when done.

    python mint_token.py            mint and store a token
    python mint_token.py --status   check the stored token and its expiry
    python mint_token.py --revoke   revoke server-side and delete the file
"""

import argparse
import http.client
import json
import os
import ssl
import sys
import time
from getpass import getpass
from urllib.parse import urlsplit

HERE = os.path.dirname(os.path.abspath(__file__))
TOKEN_PATH = os.path.join(HERE, ".mm_token.json")
DEFAULT_SERVER = os.environ.get("MATTERLESS_SERVER", "")
API = "/api/v4"


class Client:
    def __init__(self, base_url, token=None):
        parts = urlsplit(base_url)
        self.base_url = base_url.rstrip("/")
        self.host = parts.hostname
        self.port = parts.port or (443 if parts.scheme == "https" else 80)
        self.secure = parts.scheme != "http"
        self.token = token

    def request(self, method, path, body=None):
        if self.secure:
            connection = http.client.HTTPSConnection(
                self.host, self.port, timeout=30, context=ssl.create_default_context()
            )
        else:
            connection = http.client.HTTPConnection(self.host, self.port, timeout=30)
        headers = {"Accept": "application/json", "User-Agent": "MatterLess-MintToken/1.0"}
        payload = None
        if body is not None:
            payload = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        try:
            connection.request(method, path, body=payload, headers=headers)
            response = connection.getresponse()
            raw = response.read()
            head = {key.lower(): value for key, value in response.getheaders()}
            parsed = None
            if raw:
                try:
                    parsed = json.loads(raw)
                except ValueError:
                    parsed = raw[:400].decode(errors="replace")
            return response.status, head, parsed
        finally:
            connection.close()


def describe_expiry(expires_at_ms, create_at_ms=None):
    if not expires_at_ms:
        return "no expiry set (session does not expire)"
    now_ms = time.time() * 1000.0
    days_left = (expires_at_ms - now_ms) / 86_400_000.0
    when = time.strftime("%Y-%m-%d %H:%M", time.localtime(expires_at_ms / 1000.0))
    text = "expires %s (%.1f days from now)" % (when, days_left)
    if create_at_ms:
        length_days = (expires_at_ms - create_at_ms) / 86_400_000.0
        text += "; configured session length %.1f days" % length_days
    return text


def current_session(client):
    """The session we just created is the most recently made one."""
    status, _, sessions = client.request("GET", API + "/users/me/sessions")
    if status != 200 or not isinstance(sessions, list) or not sessions:
        return None
    return max(sessions, key=lambda session: session.get("create_at") or 0)


def mint(server):
    client = Client(server)
    status, _, _ = client.request("GET", API + "/system/ping")
    if status != 200:
        sys.exit("server unreachable: %s (status %s)" % (server, status))

    login_id = input("login (email or username): ").strip()
    password = getpass("password (not echoed, not stored): ")

    status, headers, body = client.request(
        "POST", API + "/users/login", {"login_id": login_id, "password": password}
    )
    if status != 200 and isinstance(body, dict) and "mfa" in str(body.get("id", "")).lower():
        code = input("MFA code: ").strip()
        status, headers, body = client.request(
            "POST",
            API + "/users/login",
            {"login_id": login_id, "password": password, "token": code},
        )
    if status != 200:
        detail = body.get("message") if isinstance(body, dict) else body
        sys.exit("login failed (%s): %s" % (status, detail))

    token = headers.get("token")
    if not token:
        sys.exit("login succeeded but no Token response header was returned")
    del password

    client.token = token
    session = current_session(client) or {}

    record = {
        "server": server,
        "token": token,
        "user_id": body.get("id", ""),
        "username": body.get("username", ""),
        "minted_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "session_id": session.get("id", ""),
        "session_create_at": session.get("create_at", 0),
        "session_expires_at": session.get("expires_at", 0),
    }
    with open(TOKEN_PATH, "w", encoding="utf-8") as handle:
        json.dump(record, handle, indent=2)
    try:
        os.chmod(TOKEN_PATH, 0o600)
    except OSError:
        pass

    print("\ntoken stored for %s" % record["username"])
    print("  file    %s  (gitignored -- never commit it)" % TOKEN_PATH)
    print("  session %s" % describe_expiry(
        record["session_expires_at"], record["session_create_at"]
    ))
    print("\nThe Rust probes now pick this up automatically:")
    print("  cargo run -p matterless-probe -- --minutes 60")
    print("\nWhen you are done: python mint_token.py --revoke")


def load_record():
    if not os.path.exists(TOKEN_PATH):
        sys.exit("no stored token at %s -- run without flags to mint one" % TOKEN_PATH)
    with open(TOKEN_PATH, encoding="utf-8") as handle:
        return json.load(handle)


def status():
    record = load_record()
    client = Client(record["server"], record["token"])
    code, _, body = client.request("GET", API + "/users/me")
    print("file      %s" % TOKEN_PATH)
    print("server    %s" % record["server"])
    print("username  %s" % record.get("username", "?"))
    print("minted    %s" % record.get("minted_at_utc", "?"))
    if code != 200:
        detail = body.get("message") if isinstance(body, dict) else body
        print("token     REJECTED (%s): %s" % (code, detail))
        print("\nMint a new one: python mint_token.py")
        return
    print("token     valid")
    session = current_session(client)
    if session:
        print("session   %s" % describe_expiry(
            session.get("expires_at"), session.get("create_at")
        ))
        last = session.get("last_activity_at")
        if last:
            print("last used %s" % time.strftime(
                "%Y-%m-%d %H:%M", time.localtime(last / 1000.0)
            ))
        # Whether expires_at moves between runs tells us if the server extends
        # sessions with activity.
        stored = record.get("session_expires_at") or 0
        if stored and session.get("expires_at", 0) > stored:
            print("note      expiry has moved forward since minting -> the server "
                  "extends sessions with activity")


def revoke():
    record = load_record()
    client = Client(record["server"], record["token"])
    code, _, _ = client.request("POST", API + "/users/logout")
    os.remove(TOKEN_PATH)
    if code == 200:
        print("session revoked server-side and %s deleted" % TOKEN_PATH)
    else:
        print("logout returned %s; the local file was deleted anyway" % code)
        print("if that token may still be live, revoke the session from "
              "Mattermost > Profile > Security > View and Log Out of Active Sessions")


def main():
    parser = argparse.ArgumentParser(description="Mint a reusable Mattermost session token")
    parser.add_argument("--server", default=os.environ.get("MATTERLESS_SERVER", DEFAULT_SERVER))
    parser.add_argument("--status", action="store_true", help="check the stored token")
    parser.add_argument("--revoke", action="store_true", help="revoke it and delete the file")
    args = parser.parse_args()

    if args.status:
        status()
    elif args.revoke:
        revoke()
    else:
        mint(args.server)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
