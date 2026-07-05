# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Nebula, please report it responsibly:

- **Email**: security@nebula-ai.app
- **PGP Key**: Available on request

Please do **not** file public GitHub issues for security vulnerabilities.

## Response Timeline

| Stage | Target |
|-------|--------|
| Acknowledgment | 24 hours |
| Initial assessment | 72 hours |
| Fix / mitigation | 7 days (critical), 30 days (non-critical) |

## Supported Versions

| Version | Supported |
|---------|-----------|
| 2.0.x | Yes |
| 1.1.x | Yes |
| < 1.0 | No |

## Security Features

- **E2EE Sync**: X25519 + AES-256-GCM with per-identity salt derivation
- **KeyVault**: OS Keychain (macOS Keychain / Windows Credential Vault / Linux Secret Service) with AES-256-GCM file fallback
- **SSRF Protection**: Internal network address blocking
- **Injection Detection**: Prompt injection and credential leak scanning
- **Shell Whitelist**: Only pre-authorized commands can execute
- **CSP**: Strict Content Security Policy with nonce-based script/style loading
- **Updater**: Ed25519 signed updates with signature verification