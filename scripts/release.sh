#!/usr/bin/env bash
set -euo pipefail

die() {
  echo "hntui release: $*" >&2
  exit 1
}

usage() {
  echo "Usage: scripts/release.sh <version>"
}

[[ $# -eq 1 ]] || {
  usage >&2
  exit 2
}

version="$1"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || die "version must use X.Y.Z form: $version"
tag="v${version}"

cd "$(dirname "$0")/.."

[[ -z "$(git status --porcelain)" ]] || die "working tree is dirty"
branch="$(git symbolic-ref --quiet --short HEAD)" \
  || die "cannot release from detached HEAD"
git remote get-url origin >/dev/null 2>&1 || die "missing git remote: origin"

if git rev-parse --quiet --verify "refs/tags/${tag}" >/dev/null; then
  die "tag already exists locally: ${tag}"
fi

remote_tag_status=0
git ls-remote --exit-code --tags origin "refs/tags/${tag}" >/dev/null 2>&1 \
  || remote_tag_status=$?
case "$remote_tag_status" in
  0) die "tag already exists on origin: ${tag}" ;;
  2) ;;
  *) die "failed checking origin for tag: ${tag}" ;;
esac

temporary="$(mktemp)"
rollback=false
cleanup() {
  status=$?
  trap - EXIT
  rm -f "$temporary"
  if [[ "$rollback" == true ]]; then
    git restore --staged --worktree -- Cargo.toml Cargo.lock
  fi
  exit "$status"
}
trap cleanup EXIT

if ! awk -v version="$version" '
  BEGIN { in_package = 0; changed = 0 }
  /^\[package\]$/ { in_package = 1 }
  in_package && /^version[[:space:]]*=/ {
    print "version = \"" version "\""
    changed = 1
    in_package = 0
    next
  }
  { print }
  END { if (!changed) exit 1 }
' Cargo.toml >"$temporary"; then
  die "could not find package version in Cargo.toml"
fi

cp "$temporary" Cargo.toml
rollback=true
cargo check
git add Cargo.toml Cargo.lock

if ! git diff --cached --quiet; then
  git commit -m "chore(release): ${tag}"
fi
rollback=false

git tag -a "$tag" -m "$tag"
if ! git push --atomic origin "$branch" "$tag"; then
  git tag --delete "$tag" >/dev/null
  die "failed pushing ${branch} and ${tag}"
fi

echo "released ${tag}"
