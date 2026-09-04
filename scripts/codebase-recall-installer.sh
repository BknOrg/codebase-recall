#!/bin/sh
set -e

REPO="BknOrg/codebase-recall"
BIN_NAME="codebase-recall"
INSTALL_DIR="${HOME}/.local/bin"

mkdir -p "$INSTALL_DIR"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

case "$OS" in
  linux)
    TARGET="x86_64-unknown-linux-musl"
    ;;
  darwin)
    TARGET="x86_64-apple-darwin"
    ;;
  *)
    echo "Unsupported OS: $OS"
    exit 1
    ;;
esac

ASSET_NAME="${BIN_NAME}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET_NAME}"

echo "Downloading ${BIN_NAME}..."
curl -fsSL "$URL" | tar -xz -C "$INSTALL_DIR"

chmod +x "${INSTALL_DIR}/${BIN_NAME}"
echo "${BIN_NAME} installed to ${INSTALL_DIR}"