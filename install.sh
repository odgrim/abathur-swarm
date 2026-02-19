#!/usr/bin/env bash
set -euo pipefail

REPO="odgrim/abathur-swarm"
BINARY="abathur"

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Linux)  OS_TARGET="unknown-linux-gnu" ;;
  Darwin) OS_TARGET="apple-darwin" ;;
  *)
    echo "Error: Unsupported OS: $OS" >&2
    exit 1
    ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)  ARCH_TARGET="x86_64" ;;
  aarch64|arm64) ARCH_TARGET="aarch64" ;;
  *)
    echo "Error: Unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"

# Fetch latest version
echo "Fetching latest release..."
VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep '"tag_name"' \
  | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')"

if [ -z "$VERSION" ]; then
  echo "Error: Could not determine latest version" >&2
  exit 1
fi

echo "Installing ${BINARY} ${VERSION} for ${TARGET}..."

# Create temp dir with cleanup
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Download and extract
TARBALL="${BINARY}-${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${TARBALL}"

curl -fsSL "$URL" -o "${TMPDIR}/${TARBALL}"
tar xzf "${TMPDIR}/${TARBALL}" -C "$TMPDIR"

# Install binary
if [ -w /usr/local/bin ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="${HOME}/.local/bin"
  mkdir -p "$INSTALL_DIR"
fi

cp "${TMPDIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"

echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

if [ "$INSTALL_DIR" = "${HOME}/.local/bin" ]; then
  case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
      echo ""
      echo "WARNING: ${INSTALL_DIR} is not in your PATH."
      echo "Add it with:"
      echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
      ;;
  esac
fi
