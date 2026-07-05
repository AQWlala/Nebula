#!/bin/bash
set -e

# T-E-C-20: Docker entrypoint — initialize volumes + write keychain files from env vars.

# Create required directories
mkdir -p /data /keychain /logs

# Write API keys from environment variables into keychain volume files.
# This allows Docker/K8s deployments to inject secrets via env vars,
# which the Rust keychain.rs will read as a fallback when the OS
# keyring backend is unavailable (headless Linux without D-Bus).
for key in DEEPSEEK_API_KEY ANTHROPIC_API_KEY; do
    if [ -n "${!key}" ]; then
        echo "${!key}" > "/keychain/${key}"
    fi
done

exec "$@"
