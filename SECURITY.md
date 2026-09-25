# Security

Consus Launcher runs on your machine, holds a Consus API key in the system keychain, and writes configuration for other apps. We take reports about it seriously.

## Reporting a vulnerability

Please report privately, not in a public issue:

- Use GitHub's private reporting: the **Security** tab of this repository, then **Report a vulnerability**.

Include what you found, how to reproduce it, and the version (the release you installed, for example v0.1.0). We will acknowledge your report, keep you updated while we fix it, and credit you in the release notes if you would like.

Reports about the Consus gateway (`api.consus.io`) or portal (`portal.consus.io`) are welcome through the same channel.

## Supported versions

Fixes land in the latest release. Please check that the issue still reproduces there before reporting.

## What the launcher does, for reviewers

- Talks only to `api.consus.io` (one request, `GET /v1/models`, to check your key) and opens `portal.consus.io` in your browser.
- Stores the key only in the macOS Keychain; it is never written to a file by the launcher.
- Writes each tool's settings into that tool's own config location, listed in the README.
- Release builds are made only in GitHub Actions, with CycloneDX SBOMs and SHA-256 checksums attached to each release.
