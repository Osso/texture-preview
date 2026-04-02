#!/usr/bin/env bash
set -euo pipefail

cargo install --path .

# Install yazi plugin
plugin_dir="$HOME/.config/yazi/plugins/texture-preview.yazi"
mkdir -p "$plugin_dir"
cp plugins/main.lua "$plugin_dir/main.lua"

echo "Installed texture-preview binary and yazi plugin"
echo "Make sure yazi.toml has the [plugin] prepend_previewers entries for *.blp and *.ktx2"
