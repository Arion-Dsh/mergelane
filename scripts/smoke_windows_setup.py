"""Exercise install.ps1 and its token prompt in a Windows pseudoconsole."""

import json
from pathlib import Path
import re
from time import monotonic, sleep
from uuid import uuid4

from winpty import Backend, PtyProcess


PUBLIC_INSTALLER_URL = (
    "https://github.com/Arion-Dsh/mergelane/releases/latest/download/mergelane-installer.ps1"
)
ANSI = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]")


def smoke_windows_setup(root, env):
    config_dir = root / "setup-config"
    client_dir = root / "setup-claude"
    install_dir = root / "installed"
    env = {key: value for key, value in env.items() if key.casefold() != "psmodulepath"}
    env.update(MERGELANE_INSTALL_DIR=str(install_dir), XDG_CONFIG_HOME=str(config_dir),
               CLAUDE_CONFIG_DIR=str(client_dir))

    # Test the one-command wrapper against this build's installer and archive.
    source = Path(__file__).with_name("install.ps1").read_text(encoding="utf-8")
    assert source.count(PUBLIC_INSTALLER_URL) == 1, "Update the Windows installer fixture URL"
    wrapper = root / "install.ps1"
    installer_url = env["MERGELANE_DOWNLOAD_URL"] + "/mergelane-installer.ps1"
    wrapper.write_text(source.replace(PUBLIC_INSTALLER_URL, installer_url), encoding="utf-8")

    unique = uuid4().hex
    fake_token = f"release-smoke-{unique}"
    deadline = monotonic() + 120
    proc = PtyProcess.spawn(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(wrapper)],
        backend=str(Backend.ConPTY), dimensions=(32, 140), env=env,
    )
    transcript = ""

    def read_until(marker):
        nonlocal transcript
        while marker not in ANSI.sub("", transcript):
            remaining = deadline - monotonic()
            assert remaining > 0, f"Timed out waiting for {marker!r}"
            proc.fileobj.settimeout(remaining)
            try:
                transcript += proc.read(4096)
            except (EOFError, TimeoutError) as error:
                raise AssertionError(f"Did not see {marker!r} in the Windows console") from error

    try:
        read_until("Agent (codex, claude, cursor, all) [codex]:")
        proc.write("claude\r")
        read_until("Platform: gitea, plane, github, gitlab, jira [github]:")
        proc.write("\r")
        read_until("Instance URL [https://github.com]:")
        proc.write("\r")
        read_until("Paste the personal token you created on the platform:")
        assert proc.write(fake_token) == len(fake_token)
        proc.write("\r")
        read_until("MergeLane MCP and Skill installed.")
        while proc.isalive():
            assert monotonic() < deadline, "The Windows installer did not exit"
            sleep(0.01)
        assert proc.exitstatus == 0, "The Windows installer exited unsuccessfully"
        assert unique[:8] not in ANSI.sub("", transcript), "Windows console echoed the test token"
    finally:
        proc.close(force=True)

    profile = (config_dir / "mergelane" / "config.json").read_text(encoding="utf-8")
    assert fake_token not in profile, "The test token appeared in the profile"
    credentials = Path(json.loads(profile)["backends"]["github"]["token_file"])
    assert credentials.read_bytes() == fake_token.encode("ascii"), "The pasted token was not saved intact"
    binary = install_dir / "bin" / "mergelane.exe"
    mcp = json.loads((client_dir / ".claude.json").read_text(encoding="utf-8"))
    assert Path(mcp["mcpServers"]["mergelane"]["command"]).resolve() == binary.resolve()
    assert (client_dir / "skills" / "mergelane" / "SKILL.md").is_file()
    print("Windows one-command install kept the test token hidden during immediate paste.")
    return binary
