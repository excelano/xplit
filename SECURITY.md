# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub Security Advisories at https://github.com/excelano/xplit/security/advisories/new. If you would rather not use GitHub, email david.anderson@excelano.com instead. I aim to respond within seven days.

Please do not open public issues for security problems.

## Supported versions

The latest release receives security fixes. Older versions are not supported. A fix ships in a new release; update the way you installed.

## What xplit can access

xplit is a terminal program that runs locally on your machine. It starts your shell, or a command you name, in a pseudo-terminal for each pane, passes your keystrokes to the focused one, and draws their output. It runs with your operating-system user's permissions and nothing more, makes no network calls of any kind, and reads no files of its own.

When `$XDG_RUNTIME_DIR` is set, xplit listens on a Unix control socket there, `xplit-<pid>.sock`, and removes it on exit. Each pane's environment carries `XPLIT_SOCKET`, the socket's path, and `XPLIT_PANE`, the pane's number, so that `xplit` typed inside a pane asks the running copy to split, hide or focus a pane, or open the tray, instead of starting another. A request over the socket can open a pane running any command, in any directory and environment, so any process able to connect to it can do that; the socket sits in your per-user runtime directory, which other accounts cannot enter.

Copying and pasting run `wl-copy` and `wl-paste` under Wayland, or `xclip` under X11, passing the selected text to them on standard input.

## What xplit stores

xplit stores nothing. There is no config file, no history, no cache, no telemetry, and no remote logging. When the last shell exits, xplit restores the terminal and exits.

## Verifying releases

The Debian packages are attached to each GitHub release and published to the Excelano apt repository, whose indices are signed, so `apt` verifies every package it installs. Release artifacts are built by `build-release`, the release program in excelano/shipping, from a tagged commit with `cargo deb` and the `[package.metadata.deb]` configuration in this repository's `Cargo.toml`.
