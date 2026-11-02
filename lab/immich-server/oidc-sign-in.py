#!/usr/bin/env python3
"""Sign in with `immich-mount login` through the lab's Dex, playing the person at the browser.

    lab/immich-server/oidc-sign-in.py [path/to/immich-mount]

Needs ./oidc.sh to have run (it writes DEX_EMAIL and DEX_PASSWORD into .env.local). The CLI runs with a private
HOME and XDG directories, so the real configuration is never touched. It prints the address to open on stderr and
reads the address the browser ends on from stdin; this script opens the first, fills in Dex's login form, stops
at the redirect to app.immich://, and hands that back. Exits non-zero unless the CLI signs in and `status` then
reaches the library with the new session. Standard library only.
"""

import html
import http.cookiejar
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))


def load_env(path):
    values = {}
    with open(path) as env_file:
        for line in env_file:
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                key, value = line.split("=", 1)
                values[key] = value
    return values


class StopAtRedirects(urllib.request.HTTPRedirectHandler):
    """Every redirect becomes an HTTPError, so the custom-scheme one can be caught instead of followed."""

    def redirect_request(self, *arguments, **keywords):
        return None


def browser_sign_in(start_address, email, password):
    """Follow the provider's pages like a browser; return the app.immich:// address it finally redirects to."""
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), StopAtRedirects)
    address, form = start_address, None
    for _ in range(12):
        if address.startswith("app.immich:"):
            return address
        try:
            data = urllib.parse.urlencode(form).encode() if form else None
            page = opener.open(address, data=data, timeout=20).read().decode("utf-8", "replace")
            form = None
        except urllib.error.HTTPError as error:
            if error.code in (301, 302, 303, 307, 308):
                address, form = urllib.parse.urljoin(address, error.headers["Location"]), None
                continue
            raise SystemExit(f"the provider answered {error.code} at {address}")
        action = re.search(r'<form[^>]*action="([^"]*)"', page)
        if not action or 'name="password"' not in page:
            raise SystemExit(f"expected a login form at {address}")
        address = urllib.parse.urljoin(address, html.unescape(action.group(1)))
        form = {"login": email, "password": password}
    raise SystemExit("too many steps without reaching app.immich://")


def main():
    cli = sys.argv[1] if len(sys.argv) > 1 else "immich-mount"
    env_values = load_env(os.path.join(HERE, ".env.local"))
    for key in ("IMMICH_URL", "DEX_EMAIL", "DEX_PASSWORD"):
        if key not in env_values:
            raise SystemExit(f"{key} missing from .env.local; run ./seed.sh and ./oidc.sh")
    home = tempfile.mkdtemp(prefix="immich-mount-oidc-")
    environment = dict(os.environ, HOME=home, XDG_CONFIG_HOME=f"{home}/.config", XDG_CACHE_HOME=f"{home}/.cache",
                       XDG_STATE_HOME=f"{home}/.local/state", NO_COLOR="1")

    login = subprocess.Popen([cli, "login", "--server", env_values["IMMICH_URL"], "--json"], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=environment)
    start_address = None
    while start_address is None:
        line = login.stderr.readline()
        if not line:
            raise SystemExit(f"login ended before printing an address: {login.stdout.read()}")
        found = re.search(r"https?://\S+", line)
        if found:
            start_address = found.group(0)
    redirect = browser_sign_in(start_address, env_values["DEX_EMAIL"], env_values["DEX_PASSWORD"])
    print(f"browser ended on {redirect.split('?')[0]}?code=…&state=…")
    output, errors = login.communicate(redirect + "\n", timeout=60)
    if login.returncode != 0:
        raise SystemExit(f"login failed ({login.returncode}): {errors.strip()}")
    report = json.loads(output)
    print(f"login: {report['method']} as {report['user']} on Immich {report['server_version']}")

    status = json.loads(subprocess.run([cli, "status", "--json"], capture_output=True, text=True, env=environment,
                                       check=True, timeout=60).stdout)
    print(f"status: {status['state']} with a {status['account']['credential']} credential")
    if status["state"] not in ("online", "unmounted") or status["account"]["credential"] != "session":
        raise SystemExit("the new session does not reach the library")
    print("OK")


if __name__ == "__main__":
    main()
