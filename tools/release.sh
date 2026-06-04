#!/usr/bin/env bash
# Cut a Peptide release: bump version, build Mac + Windows, package, tag, publish.
#
# This is the single source of truth for the release process. The version bump
# is part of the flow on purpose — the binary, the macOS bundle plist, the zip
# names, the git tag, and the GitHub release all derive from the one argument.
#
# Usage:
#   ./tools/release.sh 0.11                 build + package + tag + publish v0.11
#   ./tools/release.sh 0.11 --no-publish    build + package + bump only (no git/gh)
#
# Requirements:
#   - macOS host (for the .app bundle + ditto)
#   - cargo-xwin or the x86_64-pc-windows-gnu target (for the .exe; see make-win.sh)
#   - gh CLI authenticated (unless --no-publish)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

RAW_VERSION="${1:-}"
PUBLISH=1
[ "${2:-}" = "--no-publish" ] && PUBLISH=0

if [ -z "$RAW_VERSION" ]; then
  echo "Usage: $0 <version> [--no-publish]   e.g. $0 0.11" >&2
  exit 1
fi

# Strip a leading "v" if the caller typed one (v0.11 -> 0.11).
VERSION="${RAW_VERSION#v}"
TAG="v$VERSION"

# Cargo requires a 3-component semver. Normalize 0.11 -> 0.11.0; leave x.y.z as-is.
case "$VERSION" in
  *.*.*) CARGO_VERSION="$VERSION" ;;
  *.*)   CARGO_VERSION="$VERSION.0" ;;
  *)     CARGO_VERSION="$VERSION.0.0" ;;
esac

echo "==> Releasing Peptide $TAG (package version $CARGO_VERSION)"

# Refuse to publish a dirty tree — the version-bump commit must be the only change.
if [ "$PUBLISH" = "1" ] && [ -n "$(git status --porcelain)" ]; then
  echo "Working tree is dirty. Commit or stash before releasing (or pass --no-publish)." >&2
  exit 1
fi

# ---- 1. Bump version in both Cargo.toml files + refresh the lock --------------
echo "==> Bumping version to $CARGO_VERSION…"
sed -i '' "s/^version = \"[^\"]*\"/version = \"$CARGO_VERSION\"/" Cargo.toml
sed -i '' "s/^version = \"[^\"]*\"/version = \"$CARGO_VERSION\"/" crates/ssf2-converter/Cargo.toml
# Update only Peptide's own entries in Cargo.lock (keeps the build reproducible).
cargo update -p peptide -p ssf2_converter --precise "$CARGO_VERSION" 2>/dev/null \
  || cargo build --release -p peptide --bin peptide >/dev/null

# ---- 2. Build both platforms (version flows in via PEPTIDE_VERSION) -----------
export PEPTIDE_VERSION="$CARGO_VERSION"
echo "==> Building macOS .app…"
"$HERE/make-app.sh" --no-open
echo "==> Building Windows .exe…"
"$HERE/make-win.sh"

# ---- 3. Package the release zips ----------------------------------------------
echo "==> Packaging zips…"
cd build
MAC_ZIP="Mac - Peptide $VERSION.zip"
WIN_ZIP="Windows - Peptide $VERSION.zip"
rm -f "$MAC_ZIP" "$WIN_ZIP"
ditto -c -k --keepParent "Peptide.app" "$MAC_ZIP"
( cd windows && zip -r -q "../$WIN_ZIP" peptide.exe data )
cd "$ROOT"
echo "    build/$MAC_ZIP"
echo "    build/$WIN_ZIP"

if [ "$PUBLISH" = "0" ]; then
  echo "==> --no-publish: skipping commit/tag/gh. Artifacts are in build/."
  exit 0
fi

# ---- 4. Commit the version bump, tag, and publish -----------------------------
# Idempotent: re-running for the same version updates the tag + release assets
# in place rather than erroring on "already exists".
if [ -n "$(git status --porcelain Cargo.toml crates/ssf2-converter/Cargo.toml Cargo.lock)" ]; then
  echo "==> Committing version bump…"
  git add Cargo.toml crates/ssf2-converter/Cargo.toml Cargo.lock
  git commit -m "peptide: release $TAG

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
else
  echo "==> Version already at $CARGO_VERSION; no bump commit needed."
fi

# Find the previous tag for the changelog compare link (best-effort).
PREV_TAG="$(git tag --sort=-v:refname | grep -v "^$TAG\$" | head -1 || true)"
echo "==> Tagging $TAG…"
git tag -fa "$TAG" -m "Peptide $TAG"

# Releases land on main. Pushing the current HEAD to main fast-forwards when the
# branch was cut from an up-to-date main (the worktree case), matching how this
# repo cuts releases directly against main.
echo "==> Pushing bump commit to main + tag $TAG…"
git push origin HEAD:main
git push -f origin "$TAG"

REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
if [ -n "$PREV_TAG" ]; then
  NOTES="**Full Changelog**: https://github.com/$REPO/compare/$PREV_TAG...$TAG"
else
  NOTES="**Full Changelog**: https://github.com/$REPO/commits/$TAG"
fi

if gh release view "$TAG" >/dev/null 2>&1; then
  echo "==> Updating existing GitHub release $TAG…"
  gh release edit "$TAG" --title "Peptide $TAG" --notes "$NOTES"
  gh release upload "$TAG" "build/$MAC_ZIP" "build/$WIN_ZIP" --clobber
else
  echo "==> Creating GitHub release $TAG…"
  gh release create "$TAG" \
    --title "Peptide $TAG" \
    --notes "$NOTES" \
    "build/$MAC_ZIP" \
    "build/$WIN_ZIP"
fi

echo "==> Released: https://github.com/$REPO/releases/tag/$TAG"
