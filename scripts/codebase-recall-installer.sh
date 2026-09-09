#!/bin/sh
set -e

REPO="BknOrg/codebase-recall"
BIN_NAME="code-rcl"
PACKAGE_NAME="codebase-recall"
BASE_DIR="${HOME}/.code-rcl"
INSTALL_DIR="${BASE_DIR}/bin"
PLUGIN_DIR="${BASE_DIR}/plugins"

mkdir -p "$INSTALL_DIR"
mkdir -p "$PLUGIN_DIR"

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

# --- Automatic add to PATH Unix ---
case ":$PATH:" in
  *":$INSTALL_DIR:"*)
    echo "$INSTALL_DIR is already in your PATH."
    ;;
  *)
    EXPORT_LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""
    ADDED=false

    add_to_file() {
      file="$1"
      if [ -f "$file" ]; then
        if ! grep -qsF "$INSTALL_DIR" "$file"; then
          printf "\n# codebase-recall\n%s\n" "$EXPORT_LINE" >> "$file"
          ADDED=true
        fi
      fi
    }

    add_to_file "${HOME}/.zshrc"
    add_to_file "${HOME}/.bashrc"

    if [ "$ADDED" = false ] && [ -f "${HOME}/.profile" ]; then
      add_to_file "${HOME}/.profile"
    fi

    if [ "$ADDED" = true ]; then
      echo "Added ${INSTALL_DIR} to your shell profile."
      echo "Please restart your terminal or run: export PATH=\"${INSTALL_DIR}:\$PATH\""
    else
      echo "Please manually add to your PATH: export PATH=\"${INSTALL_DIR}:\$PATH\""
    fi
    ;;
esac

printf "\nDone! You can now run '%s --help' from any directory.\n" "$BIN_NAME"