#!/usr/bin/env bash
# Use YT_DLP_WRAPPER_DATA_ROOT="$(mktemp -d)" ./scripts/run-dev.sh to preview first-run states.
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
project="$repository_root/src/dotnet/YtDlpWrapper.App/YtDlpWrapper.App.csproj"
backend="$repository_root/target/debug/yt-dlp-wrapper-backend"
app_output="$repository_root/src/dotnet/YtDlpWrapper.App/bin/Debug/net10.0"

if [[ $(uname -s) != Darwin || $(uname -m) != arm64 ]]; then
    echo "This development script is for Apple Silicon macOS." >&2
    exit 1
fi

build_version=${YT_DLP_WRAPPER_VERSION:-}
if [[ -z "$build_version" ]]; then
    version_tag=$(git -C "$repository_root" describe --tags --abbrev=0 --match 'v[0-9]*')
    build_version=${version_tag#v}
fi

export YT_DLP_WRAPPER_VERSION=$build_version
cargo build --manifest-path "$repository_root/Cargo.toml" --locked
dotnet build "$project" -p:Version="$build_version"
cp "$backend" "$app_output/yt-dlp-wrapper-backend"
chmod 755 "$app_output/yt-dlp-wrapper-backend"
dotnet run --project "$project" --no-build -p:Version="$build_version"
