#!/usr/bin/env python3
"""Exercise the generated shell/PowerShell installer using local release archives."""

import argparse
from functools import partial
import hashlib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import subprocess
import tempfile
from threading import Thread
import tomllib

from smoke_package import smoke


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("target")
    args = parser.parse_args()
    packages = args.directory.resolve()
    windows = "windows" in args.target
    installer = packages / ("mergelane-installer.ps1" if windows else "mergelane-installer.sh")
    archive = f"mergelane-{args.target}" + (".zip" if windows else ".tar.xz")
    checksum = (packages / (archive + ".sha256")).read_text().split()[0]
    assert hashlib.sha256((packages / archive).read_bytes()).hexdigest() == checksum
    # cargo-dist 0.34.0 embeds hashes in shell installers, but not PowerShell.
    if not windows:
        assert checksum in installer.read_text(), "shell installer must embed this archive's checksum"
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("MERGELANE_", "INSTALLER_", "CARGO_DIST_FORCE_INSTALL_DIR"))}
    handler = partial(SimpleHTTPRequestHandler, directory=str(packages))
    with tempfile.TemporaryDirectory(prefix="mergelane-installer-") as temp:
        root = Path(temp)
        env["MERGELANE_UNMANAGED_INSTALL"] = str(root / "installed")
        env["MERGELANE_NO_MODIFY_PATH"] = "1"
        with ThreadingHTTPServer(("127.0.0.1", 0), handler) as server:
            thread = Thread(target=server.serve_forever, daemon=True)
            thread.start()
            env["MERGELANE_DOWNLOAD_URL"] = f"http://127.0.0.1:{server.server_port}"
            command = (["pwsh", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
                       if windows else ["sh"])
            try:
                subprocess.run([*command, str(installer)], env=env, check=True, timeout=120)
                smoke(root / "installed" / ("mergelane.exe" if windows else "mergelane"),
                      root, version)
            finally:
                server.shutdown()
                thread.join(timeout=5)
    print(f"Generated installer passed: {installer.name}")


if __name__ == "__main__":
    main()
