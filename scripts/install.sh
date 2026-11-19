#!/bin/sh
set -eu

# Keep the terminal available to the setup wizard when this script is piped to sh.
exec 3</dev/tty
mergelane_install_root=${MERGELANE_INSTALL_DIR:-"$HOME/.mergelane"}
mergelane_installer=$(mktemp)
trap 'rm -f "$mergelane_installer"' EXIT
trap 'exit 1' HUP INT TERM

curl --proto '=https' --proto-redir '=https' -fsSL \
  https://github.com/Arion-Dsh/mergelane/releases/latest/download/mergelane-installer.sh \
  -o "$mergelane_installer"
MERGELANE_INSTALL_DIR="$mergelane_install_root" sh "$mergelane_installer" </dev/null
"$mergelane_install_root/bin/mergelane" setup <&3
