#!/usr/bin/env bash
set -euo pipefail

die() {
  echo "hntui install: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

usage() {
  cat <<'EOF'
Usage: install.sh [--version X|vX] [--repo OWNER/REPO] [--to DIR]

Defaults:
  repo: rocrp/hntui
  to:   ~/.local/bin
  version: latest GitHub release
EOF
}

repo="${HNTUI_REPO:-rocrp/hntui}"
install_dir="${INSTALL_DIR:-$HOME/.local/bin}"
version="${HNTUI_VERSION:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo)
      repo="${2:-}"; shift 2 || true
      [[ -n "$repo" ]] || die "--repo requires value"
      ;;
    --to)
      install_dir="${2:-}"; shift 2 || true
      [[ -n "$install_dir" ]] || die "--to requires value"
      ;;
    --version)
      version="${2:-}"; shift 2 || true
      [[ -n "$version" ]] || die "--version requires value"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown arg: $1"
      ;;
  esac
done

need awk
need curl
need mktemp
need tar
need uname

os="$(uname -s)"
case "$os" in
  Darwin) os="darwin" ;;
  Linux) os="linux" ;;
  *) die "unsupported OS: $(uname -s)" ;;
esac

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) arch="amd64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) die "unsupported arch: $(uname -m)" ;;
esac

if [[ "$os" == "linux" && "$arch" == "arm64" ]]; then
  die "linux arm64 not supported yet"
fi

tag=""
if [[ -n "$version" ]]; then
  if [[ "$version" == v* ]]; then
    tag="$version"
  else
    tag="v$version"
  fi
else
  api="https://api.github.com/repos/${repo}/releases/latest"
  json="$(curl -fsSL "$api")" || die "failed fetching latest release: $api"
  tag="$(printf '%s' "$json" | awk -F'"' '/"tag_name"[[:space:]]*:/{print $4; exit}')"
fi
[[ -n "$tag" ]] || die "could not resolve release tag (try --version)"

bin="hntui"
asset="${bin}-${tag}-${os}-${arch}.tar.gz"
url="https://github.com/${repo}/releases/download/${tag}/${asset}"

tmp="$(mktemp -d)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

curl -fL "$url" -o "$tmp/$asset" || die "download failed: $url"
tar -xzf "$tmp/$asset" -C "$tmp" || die "extract failed: $asset"

exe="$tmp/${bin}"
[[ -f "$exe" ]] || die "archive missing ${bin}"
chmod +x "$exe"

install_dir="${install_dir%/}"
if mkdir -p "$install_dir"; then
  :
else
  need sudo
  sudo mkdir -p "$install_dir"
fi

dest="${install_dir}/${bin}"
if [[ -w "$install_dir" ]]; then
  mv "$exe" "$dest"
else
  need sudo
  sudo mv "$exe" "$dest"
fi

echo "installed: $dest"
case ":${PATH}:" in
  *":${install_dir}:"*) ;;
  *) echo "note: add to PATH: ${install_dir}" ;;
esac

