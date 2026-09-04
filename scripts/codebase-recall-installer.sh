#!/bin/sh
set -e

REPO="BknOrg/codebase-recall"
BIN_NAME="code-rcl"
PACKAGE_NAME="codebase-recall"
INSTALL_DIR="${HOME}/.local/bin"

mkdir -p "$INSTALL_DIR"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  linux)
    TARGET="x86_64-unknown-linux-musl"
    ;;
  darwin)
    case "$ARCH" in
      arm64|aarch64)
        TARGET="aarch64-apple-darwin"
        ;;
      x86_64)
        TARGET="x86_64-apple-darwin"
        ;;
      *)
        echo "Unsupported Mac architecture: $ARCH"
        exit 1
        ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS"
    exit 1
    ;;
esac

ASSET_NAME="${PACKAGE_NAME}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET_NAME}"

echo "Downloading ${PACKAGE_NAME} (${BIN_NAME}) for ${TARGET}..."
curl -fsSL "$URL" | tar -xz -C "$INSTALL_DIR"

chmod +x "${INSTALL_DIR}/${BIN_NAME}"
echo "${BIN_NAME} installed to ${INSTALL_DIR}"