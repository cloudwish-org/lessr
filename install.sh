#!/bin/sh
# Lessr installer. Served from https://lessr.dev/install
#
#   curl -fsSL https://lessr.dev/install | sh
#
# Downloads the release binary for this machine, checks it against the
# published checksum, and puts it somewhere on PATH. No root, no package
# manager, nothing written outside the install directory.
#
# Environment:
#   LESSR_VERSION   a tag like v0.1.0. Default: the latest release.
#   LESSR_BIN_DIR   where to install. Default: the first writable of
#                   $XDG_BIN_HOME, ~/.local/bin, /usr/local/bin.
#   LESSR_NO_INIT   set to skip the closing "now run lessr init" advice.

set -eu

REPO="cloudwish-org/lessr"
RELEASES="https://github.com/${REPO}/releases"

say() { printf '%s\n' "$*"; }
err() { printf 'lessr: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "this installer needs $1"; }

# Which build to fetch. The names here must match the artefacts release.yml
# uploads; docs/RELEASE.md is the list.
target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Darwin)
      case "$arch" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64)        echo "x86_64-apple-darwin" ;;
        *) err "unsupported macOS architecture: $arch" ;;
      esac
      ;;
    Linux)
      # musl: one static binary that runs on any distribution, glibc or not.
      case "$arch" in
        x86_64|amd64)  echo "x86_64-unknown-linux-musl" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
        *) err "unsupported Linux architecture: $arch" ;;
      esac
      ;;
    *)
      err "unsupported OS: $os. Windows users: see ${RELEASES}"
      ;;
  esac
}

# Somewhere on PATH we can write without sudo.
bin_dir() {
  if [ -n "${LESSR_BIN_DIR:-}" ]; then
    echo "$LESSR_BIN_DIR"
    return
  fi
  for candidate in "${XDG_BIN_HOME:-}" "$HOME/.local/bin" "/usr/local/bin"; do
    [ -n "$candidate" ] || continue
    if [ -d "$candidate" ] && [ -w "$candidate" ]; then
      echo "$candidate"
      return
    fi
  done
  # Nothing existed; ~/.local/bin is the standard place to create.
  echo "$HOME/.local/bin"
}

fetch() {
  # $1 url, $2 destination
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https' --tlsv1.2 "$1" -o "$2"
  else
    wget -q --https-only "$1" -O "$2"
  fi
}

checksum() {
  # $1 file. Prints the lowercase hex sha256.
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    err "this installer needs sha256sum or shasum to verify the download"
  fi
}

main() {
  command -v curl >/dev/null 2>&1 || need wget
  need uname
  need tar

  TARGET=$(target)
  VERSION="${LESSR_VERSION:-latest}"
  if [ "$VERSION" = "latest" ]; then
    BASE="${RELEASES}/latest/download"
  else
    BASE="${RELEASES}/download/${VERSION}"
  fi

  ARCHIVE="lessr-${TARGET}.tar.gz"
  TMP=$(mktemp -d 2>/dev/null || mktemp -d -t lessr)
  # Leave nothing behind, including on failure.
  trap 'rm -rf "$TMP"' EXIT INT TERM

  say "Downloading lessr for ${TARGET}..."
  fetch "${BASE}/${ARCHIVE}" "${TMP}/${ARCHIVE}" \
    || err "could not download ${BASE}/${ARCHIVE}"

  # The download is verified before anything is unpacked or run. A release
  # without a checksum file is treated as a failure, not as a reason to skip
  # the check.
  fetch "${BASE}/${ARCHIVE}.sha256" "${TMP}/${ARCHIVE}.sha256" \
    || err "could not download the checksum for ${ARCHIVE}"

  want=$(cut -d' ' -f1 < "${TMP}/${ARCHIVE}.sha256")
  got=$(checksum "${TMP}/${ARCHIVE}")
  [ -n "$want" ] || err "the published checksum is empty"
  if [ "$want" != "$got" ]; then
    err "checksum mismatch for ${ARCHIVE}
  published: ${want}
  downloaded: ${got}
Not installing. Please report this at ${RELEASES}"
  fi

  tar -xzf "${TMP}/${ARCHIVE}" -C "$TMP" || err "could not unpack ${ARCHIVE}"
  [ -f "${TMP}/lessr" ] || err "${ARCHIVE} did not contain a lessr binary"

  DIR=$(bin_dir)
  mkdir -p "$DIR" || err "cannot create $DIR"
  install -m 755 "${TMP}/lessr" "${DIR}/lessr" 2>/dev/null \
    || { cp "${TMP}/lessr" "${DIR}/lessr" && chmod 755 "${DIR}/lessr"; } \
    || err "cannot write to $DIR. Set LESSR_BIN_DIR to somewhere you can write."

  say "Installed ${DIR}/lessr"
  say ""
  "${DIR}/lessr" --version || true

  case ":${PATH}:" in
    *":${DIR}:"*) ;;
    *)
      say ""
      say "${DIR} is not on your PATH. Add this to your shell profile:"
      say "    export PATH=\"${DIR}:\$PATH\""
      ;;
  esac

  if [ -z "${LESSR_NO_INIT:-}" ]; then
    say ""
    say "Next:"
    say "    lessr init          find your agents and install the hook"
    say "    lessr init --show   see every change first, change nothing"
  fi
}

main "$@"
