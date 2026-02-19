#!/usr/bin/env bash
set -euo pipefail

REPO="diskd-ai/ccbox"
BIN_NAME="ccbox"

INSTALL_DIR="${CCBOX_INSTALL_DIR:-"${HOME}/.local/bin"}"
TAG="${CCBOX_TAG:-""}"
TMP_DIR=""

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: missing required command: ${cmd}" >&2
    exit 1
  fi
}

install_binary() {
  local src dst
  src="$1"
  dst="$2"

  if command -v install >/dev/null 2>&1; then
    install -m 755 "${src}" "${dst}"
    return 0
  fi

  cp "${src}" "${dst}"
  chmod 755 "${dst}"
}

resolve_tag() {
  if [[ -n "${TAG}" ]]; then
    return 0
  fi

  local tag
  tag="$(
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
      | grep -m1 '"tag_name"' \
      | sed -E 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/'
  )"

  if [[ -z "${tag}" ]]; then
    echo "error: failed to resolve latest release tag" >&2
    exit 1
  fi

  TAG="${tag}"
}

resolve_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}" in
    Darwin)
      case "${arch}" in
        arm64|aarch64)
          echo "aarch64-apple-darwin"
          ;;
        x86_64)
          echo "x86_64-apple-darwin"
          ;;
        *)
          echo "error: unsupported CPU architecture on macOS: ${arch}" >&2
          exit 1
          ;;
      esac
      ;;
    Linux)
      case "${arch}" in
        x86_64)
          echo "x86_64-unknown-linux-gnu"
          ;;
        *)
          echo "error: unsupported CPU architecture on Linux: ${arch}" >&2
          exit 1
          ;;
      esac
      ;;
    *)
      echo "error: unsupported OS: ${os}" >&2
      exit 1
      ;;
  esac
}

verify_sha256() {
  local dir asset
  dir="$1"
  asset="$2"

  local sha_path expected actual
  sha_path="${dir}/${asset}.sha256"
  if [[ ! -f "${sha_path}" ]]; then
    echo "error: missing checksum file: ${sha_path}" >&2
    exit 1
  fi

  expected="$(
    sed -n '1p' "${sha_path}" \
      | tr -s '[:space:]' ' ' \
      | cut -d ' ' -f1 \
      | tr 'A-F' 'a-f'
  )"
  if [[ -z "${expected}" ]]; then
    echo "error: failed to parse checksum file: ${sha_path}" >&2
    exit 1
  fi

  if command -v shasum >/dev/null 2>&1; then
    actual="$(
      shasum -a 256 "${dir}/${asset}" \
        | cut -d ' ' -f1 \
        | tr 'A-F' 'a-f'
    )"
    if [[ "${expected}" != "${actual}" ]]; then
      echo "error: sha256 mismatch for ${asset}" >&2
      exit 1
    fi
    return 0
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(
      sha256sum "${dir}/${asset}" \
        | cut -d ' ' -f1 \
        | tr 'A-F' 'a-f'
    )"
    if [[ "${expected}" != "${actual}" ]]; then
      echo "error: sha256 mismatch for ${asset}" >&2
      exit 1
    fi
    return 0
  fi

  echo "warning: no sha256 tool found; skipping checksum verification" >&2
}

main() {
  require_cmd curl
  require_cmd tar
  require_cmd uname
  require_cmd grep
  require_cmd sed
  require_cmd mktemp

  resolve_tag

  local version target base_url asset
  version="${TAG#v}"
  target="$(resolve_target)"
  base_url="https://github.com/${REPO}/releases/download/${TAG}"
  asset="${BIN_NAME}-${version}-${target}.tar.gz"

  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "${TMP_DIR}"' EXIT

  echo "downloading: ${asset}" >&2
  curl -fsSL -o "${TMP_DIR}/${asset}" "${base_url}/${asset}"
  curl -fsSL -o "${TMP_DIR}/${asset}.sha256" "${base_url}/${asset}.sha256"

  verify_sha256 "${TMP_DIR}" "${asset}"

  tar -C "${TMP_DIR}" -xzf "${TMP_DIR}/${asset}"

  mkdir -p "${INSTALL_DIR}"
  install_binary "${TMP_DIR}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"

  echo "installed: ${INSTALL_DIR}/${BIN_NAME}" >&2

  if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
    echo "note: add ${INSTALL_DIR} to PATH to run ${BIN_NAME} without a full path." >&2
  fi
}

main "$@"

