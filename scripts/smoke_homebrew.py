#!/usr/bin/env python3
"""Install a generated formula using local archives, then remove the test tap."""

from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from threading import Thread
import tomllib
import uuid

from smoke_package import smoke


def main():
    packages = Path(sys.argv[1]).resolve()
    formula = (packages / "mergelane.rb").read_text()
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    env = os.environ | {
        "HOMEBREW_NO_AUTO_UPDATE": "1", "HOMEBREW_NO_ANALYTICS": "1",
        "HOMEBREW_NO_INSTALL_CLEANUP": "1", "HOMEBREW_DEVELOPER": "1",
    }

    def brew(*args, check=True):
        return subprocess.run(["brew", *args], env=env, check=check,
                              text=True, capture_output=True, timeout=300)

    # Never replace an existing installation on a developer's machine.
    installed = brew("list", "--formula", "--versions", "mergelane", check=False)
    if installed.stdout.strip():
        raise RuntimeError("Uninstall the existing mergelane before running this isolated smoke test")
    tap = "mergelane-smoke/test" + uuid.uuid4().hex[:12]
    handler = partial(SimpleHTTPRequestHandler, directory=str(packages))
    with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base = f"http://127.0.0.1:{server.server_port}/"
        local, count = re.subn(
            r'https://github\.com/Arion-Dsh/mergelane/releases/download/[^/"\s]+/([^"\s]+)',
            lambda match: base + match[1], formula,
        )
        if count != 4:
            raise RuntimeError(f"Expected four native Homebrew archive URLs, found {count}")
        tap_created = False
        try:
            brew("tap-new", "--no-git", tap)
            tap_created = True
            tap_path = Path(brew("--repository", tap).stdout.strip())
            (tap_path / "Formula" / "mergelane.rb").write_text(local)
            result = brew("install", "--formula", f"{tap}/mergelane")
            print(result.stdout)
            prefix = Path(brew("--prefix", f"{tap}/mergelane").stdout.strip())
            with tempfile.TemporaryDirectory(prefix="mergelane-brew-") as temp:
                smoke(prefix / "bin" / "mergelane", Path(temp), version)
            print("Homebrew installed the generated formula with the original SHA-256 values")
        finally:
            # These commands refer only to the unique tap created above.
            if tap_created:
                brew("uninstall", "--formula", f"{tap}/mergelane", check=False)
                brew("untap", tap)
            server.shutdown()
            thread.join(timeout=5)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(error.stdout, file=sys.stderr)
        print(error.stderr, file=sys.stderr)
        raise
