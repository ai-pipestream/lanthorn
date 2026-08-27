# Docker: lanthorn as a server

lanthorn is a terminal app, which means it containerizes and serves cleanly:
everything it draws — the TUI, the live map, even Kitty-protocol graphics — is
bytes over a pty. One image supports two ways of running it.

```sh
docker build -t lanthorn .
```

The multi-stage build compiles the workspace with the repo's pinned toolchain
(`rust-toolchain.toml`) inside a Rust builder image and ships a small
Debian-slim runtime carrying all four release binaries (`lanthorn`,
`zvm-cli`, `gvm-cli`, `scott-cli`). No Rust toolchain is needed on the host.

## Mode 1: play in your own terminal

```sh
docker run -it --rm \
  -v ~/if-games:/stories \
  -v lanthorn-data:/data \
  lanthorn
```

This is full-fidelity lanthorn: your terminal talks to the app through the
container's pty, so everything that works locally works here — including the
Kitty graphics protocol for v6 artwork, if your terminal speaks it. With no
arguments the story picker opens on `/stories`; any lanthorn arguments work in
place of the default (`docker run -it --rm ... lanthorn /stories/zork1.z5`,
`... lanthorn --help`).

Two host-terminal facts pass through automatically: `docker run -it` conveys
your terminal size and resizes, and lanthorn's capability probes (Kitty
graphics, colours) travel over the pty like any other escape sequences. If
your terminal's `TERM` is something unusual, forward it: `-e TERM=$TERM`.

## Mode 2: serve it to browsers

```sh
docker run -d --name lanthorn \
  -p 7681:7681 \
  -v ~/if-games:/stories \
  -v lanthorn-data:/data \
  lanthorn serve
```

Then open <http://localhost:7681>. The container runs
[ttyd](https://github.com/tsl0922/ttyd) (a pinned static release binary,
fetched at image-build time), which serves an xterm.js terminal in the browser and spawns **one lanthorn process
per connection** — several people can play at once, each in their own session,
sharing the `/stories` library and the `/data` save directory. A game saved in
one session restores in the next.

Arguments after `serve` go to lanthorn (`... lanthorn serve /stories/zork1.z5`
pins every connection to one story); with none, each connection gets the
picker on `/stories`.

Serve-mode knobs, as environment variables:

| variable | meaning | default |
|---|---|---|
| `LANTHORN_WEB_PORT` | port ttyd listens on | `7681` |
| `LANTHORN_WEB_CREDENTIAL` | HTTP basic auth, `user:pass` | unset (no auth) |

**Do not expose an unauthenticated port beyond localhost** — a lanthorn
session includes a story picker that can browse and download into `/stories`,
and any writable terminal is an interactive program running on your machine.
For anything public, set `LANTHORN_WEB_CREDENTIAL` and terminate TLS in front
of it with your usual reverse proxy (Caddy, nginx, Traefik); ttyd itself
speaks plain HTTP/WebSocket here.

`docker-compose.yml` at the repo root is a ready-made example of this mode:
`mkdir -p stories && docker compose up -d`.

### What the browser mode can and cannot show

xterm.js does not implement the Kitty graphics protocol, and lanthorn's
capability probe discovers that honestly — graphical v6 stories and Blorb
cover art fall back to half-block cell rendering, exactly as they do in any
terminal without image support. Text games, the automap, mouse support, and
the full TUI are unaffected. When graphics fidelity matters, use mode 1 (or
SSH to the host and run mode 1 there: Kitty graphics survive SSH).

Audio does not play in either mode: the container has no sound device, and the
binaries degrade to silence exactly as they do on a Linux host without one
(`libasound2` is present in the image, so nothing fails to load).

## The two volumes

| mount | contents |
|---|---|
| `/stories` | the game library; the picker opens here. The repo ships no stories (commercial games are gitignored), so this is yours to fill — or use the picker's built-in IFDB search (`/`) to download freely available ones into it. |
| `/data` | the container user's `$HOME`. Saves, `config.toml` / `style.toml`, and `.lanthorn` map archives live in `/data/.lanthorn`. Name it a volume and saves persist across image upgrades. |

The container runs as an unprivileged user (`lanthorn`, uid 1000). If you
bind-mount host directories and see permission errors, either `chown` them to
uid 1000 or run with `--user "$(id -u):$(id -g)"` (then `$HOME` is still
`/data`, so keep that mount writable by your uid).

## Publishing

`.github/workflows/docker.yml` builds the image on every version tag and
pushes it to GitHub Container Registry as
`ghcr.io/<owner>/lanthorn:<version>` / `:latest` (pre-release tags skip
`latest`), so "serve it up" can be one line with no checkout at all:

```sh
docker run -d -p 7681:7681 -v ~/if-games:/stories -v lanthorn-data:/data \
  ghcr.io/sharkusk/lanthorn:latest serve
```

The published image is `linux/amd64`; on Apple-Silicon Docker Desktop it runs
under Rosetta, or build natively from a checkout with the one `docker build`
line at the top of this page.
