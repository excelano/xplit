# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub Security Advisories at https://github.com/excelano/xplit/security/advisories/new. If you would rather not use GitHub, email david.anderson@excelano.com instead. I aim to respond within seven days.

Please do not open public issues for security problems.

## Supported versions

The latest release receives security fixes. Older versions are not supported.

## What xplit can access

xplit is a terminal program that runs locally on your machine. It starts your own shell in two pseudo-terminals, passes your keystrokes to the focused one, and draws their output. It runs with your operating-system user's permissions and nothing more, makes no network calls of any kind, and reads no files of its own.

## What xplit stores

xplit stores nothing. There is no config file, no history, no cache, no telemetry, and no remote logging. When the last shell exits, xplit restores the terminal and exits.

## Verifying releases

The Debian packages are attached to each GitHub release and published to the Excelano apt repository, whose indices are signed, so `apt` verifies every package it installs. Release artifacts are built by `build-release`, the release program in excelano/shipping, from a tagged commit with `cargo deb` and the `[package.metadata.deb]` configuration in this repository's `Cargo.toml`.
