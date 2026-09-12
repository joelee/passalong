#!/usr/bin/env bash
# Fails unless a release tag is exactly `v` followed by the workspace version
# in Cargo.toml. The release workflow runs it before building or publishing.
#
# Usage: scripts/check-release-tag.sh <tag> [path/to/Cargo.toml]
set -euo pipefail

tag="${1:-}"
manifest="${2:-Cargo.toml}"
if [ -z "$tag" ]; then
    echo "usage: check-release-tag.sh <tag> [path/to/Cargo.toml]" >&2
    exit 2
fi

# The first `version = "..."` inside [workspace.package].
version="$(awk '
    /^\[workspace\.package\]/ { inside = 1; next }
    /^\[/ { inside = 0 }
    inside && /^version[ \t]*=/ {
        sub(/^version[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit
    }' "$manifest")"
if [ -z "$version" ]; then
    echo "error: no [workspace.package] version in $manifest" >&2
    exit 1
fi

if ! printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "error: tag '$tag' is not of the form vMAJOR.MINOR.PATCH" >&2
    exit 1
fi
if [ "$tag" != "v$version" ]; then
    echo "error: tag $tag does not match the workspace version $version" >&2
    exit 1
fi
echo "tag $tag matches the workspace version"
