#!/usr/bin/env bash
set -euo pipefail

# Cross-platform-ish launcher for macOS/Linux (Git Bash/WSL on Windows works too).
# It creates .venv if missing, installs deps once, then runs the app inside it.

VENV_BIN=".venv/bin/python"
REQ_FILE="requirements.txt"

find_base_python() {
  if command -v python3 >/dev/null 2>&1; then
    command -v python3
  elif command -v python >/dev/null 2>&1; then
    command -v python
  else
    echo "Python was not found on PATH. Install Python 3.11+ and retry." >&2
    exit 1
  fi
}

if [ ! -x "$VENV_BIN" ]; then
  BASE_PYTHON=$(find_base_python)
  echo "Creating virtual environment in .venv..."
  "$BASE_PYTHON" -m venv .venv
  echo "Installing dependencies..."
  "$VENV_BIN" -m pip install --upgrade pip
  if [ -f "$REQ_FILE" ]; then
    "$VENV_BIN" -m pip install -r "$REQ_FILE"
  else
    echo "$REQ_FILE not found; skipping dependency install."
  fi
fi

"$VENV_BIN" -m flow_stt "$@"
