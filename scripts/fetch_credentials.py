#!/usr/bin/env python3
"""Fetch one camera's LAN credentials from your Owlet account into an env file.

Run it once, and again if you re-pair the camera. Standard library only.
Your Owlet password and the account tokens stay in memory and are never saved.

Endpoints and request shapes come from simonsickle/owlet-rtsp-bridge (AGPL-3.0).
"""

import argparse
import getpass
import ipaddress
import json
import os
import re
import secrets
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

# The Owlet Android app's public client key and identity. Not account secrets.
API_KEY = "AIzaSyDlfp3urNTbyhCtHOCxZBOjHQf4WuN_Aws"
APP = {
    "X-Android-Package": "com.owletcare.sleep",
    "X-Android-Cert": "2A3BC26DB0B8B0792DBE28E6FFDC2598F9B12B74",
}
SIGN_IN = "https://www.googleapis.com/identitytoolkit/v3/relyingparty/verifyPassword"
ACCOUNTS = "https://accounts.owletdata.com/v2/accounts"
KMS = "https://camera-kms.owletdata.com/kms"


class Problem(Exception):
    """A failure to show without a traceback. Never contains a secret."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """A redirect could carry the token to another host."""

    def redirect_request(self, *args, **kwargs):
        return None


def call(url, headers, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Accept": "application/json", "Content-Type": "application/json", **headers}
    request = urllib.request.Request(url, data=data, headers=headers)
    host = urllib.parse.urlsplit(url).hostname
    try:
        with urllib.request.build_opener(NoRedirect).open(request, timeout=25) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise Problem(f"{host} answered HTTP {error.code}{reason(error)}") from None
    except (urllib.error.URLError, TimeoutError) as error:
        raise Problem(f"cannot reach {host}: {getattr(error, 'reason', error)}") from None
    except ValueError:
        raise Problem(f"{host} did not answer with JSON") from None


def reason(error):
    """The short error code from Google, for example INVALID_LOGIN_CREDENTIALS."""
    try:
        message = json.load(error)["error"]["message"]
    except (ValueError, KeyError, TypeError):
        return ""
    return f" ({message})" if re.fullmatch(r"[A-Z_]{1,60}", str(message)) else ""


def part(value):
    return urllib.parse.quote(str(value), safe="")


def camera_list(services):
    cameras = []
    for service in services:
        dsn = (service.get("device") or {}).get("dsn")
        if service.get("serviceType") == "SERVICE_TYPE_CAMERA" and dsn:
            cameras.append((dsn, service.get("displayName") or dsn))
    return cameras


def choose(cameras, wanted):
    if wanted:
        cameras = [c for c in cameras if c[0] == wanted]
    if len(cameras) == 1:
        return cameras[0][0]
    if not cameras:
        raise Problem("no matching camera on this account")
    listing = "\n".join(f"  {dsn}  {name}" for dsn, name in cameras)
    raise Problem(f"several cameras found; run again with --dsn:\n{listing}")


def env_text(ip, credentials, rtsp_password):
    uid = credentials.get("tutkid") or credentials.get("tutk_uid")
    password = credentials.get("password")
    auth_key = credentials.get("authKey") or credentials.get("auth_key") or ""
    if not uid or not password:
        raise Problem("Owlet returned no credentials for this camera")
    values = {
        "OWLET_CAMERA_IP": ip,
        "OWLET_TUTK_UID": uid,
        "OWLET_TUTK_PASSWORD": password,
        "OWLET_TUTK_AUTHKEY": auth_key,
        "RTSP_PASSWORD": rtsp_password,
    }
    return "".join(f"{name}={quote(name, value)}\n" for name, value in values.items())


def quote(name, value):
    """Single quotes stop Compose from expanding $ in a value."""
    value = str(value)
    if "'" in value or "\\" in value or not value.isprintable():
        raise Problem(f"{name} contains a character this script cannot store")
    return f"'{value}'"


def write_private(path, text):
    """Create the file as mode 600 from the first byte, then move it into place."""
    temporary = path.with_name(f"{path.name}.{secrets.token_hex(4)}.tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(fd, "w") as file:
            file.write(text)
        os.replace(temporary, path)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ip", required=True, type=ipaddress.IPv4Address, help="camera LAN address")
    parser.add_argument("--dsn", help="camera serial, if the account has several")
    parser.add_argument("--out", type=Path, default=Path(".env"))
    parser.add_argument("--force", action="store_true", help="replace an existing file")
    args = parser.parse_args()
    if args.out.exists() and not args.force:
        raise Problem(f"{args.out} exists; use --force to replace it")
    email = input("Owlet email: ").strip()
    password = getpass.getpass("Owlet password (not shown): ")
    body = {"email": email, "password": password, "returnSecureToken": True}
    account = call(f"{SIGN_IN}?key={API_KEY}", APP, body)
    token, uid = account.get("idToken"), account.get("localId")
    if not token or not uid:
        raise Problem("sign-in returned no token")
    bearer = {"Authorization": f"Bearer {token}"}
    services = call(f"{ACCOUNTS}/{part(uid)}/services", bearer).get("services") or []
    dsn = choose(camera_list(services), args.dsn)
    # The camera service expects the bare token, without "Bearer".
    credentials = call(f"{KMS}/{part(dsn)}", {"Authorization": token})
    write_private(args.out, env_text(args.ip, credentials, secrets.token_urlsafe(24)))
    print(f"Wrote {args.out} (mode 600). Do not commit it or paste it anywhere.")


if __name__ == "__main__":
    try:
        main()
    except (Problem, OSError) as problem:
        sys.exit(f"error: {problem}")
    except (KeyboardInterrupt, EOFError):
        sys.exit("\ncancelled")
