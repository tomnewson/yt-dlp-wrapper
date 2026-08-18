#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
project="$repository_root/src/dotnet/YtDlpWrapper.App/YtDlpWrapper.App.csproj"
target_triple=aarch64-apple-darwin
runtime=osx-arm64
dist_root="$repository_root/dist/macos-arm64"
publish_output="$dist_root/publish"
app_bundle="$dist_root/YT-DLP Wrapper.app"
app_macos="$app_bundle/Contents/MacOS"
app_resources="$app_bundle/Contents/Resources"

if [[ $(uname -s) != Darwin || $(uname -m) != arm64 ]]; then
    echo "This build script must run natively on an Apple Silicon Mac." >&2
    exit 1
fi

build_version=${YT_DLP_WRAPPER_VERSION:-}
if [[ -z "$build_version" ]]; then
    version_tag=$(git -C "$repository_root" describe --tags --abbrev=0 --match 'v[0-9]*')
    build_version=${version_tag#v}
fi
if [[ ! "$build_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.+][0-9A-Za-z.-]+)?$ ]]; then
    echo "Build version is not a supported semantic version: $build_version" >&2
    exit 1
fi
bundle_version=${build_version%%[-+]*}

rm -rf "$dist_root"
mkdir -p "$publish_output" "$app_macos" "$app_resources"

export YT_DLP_WRAPPER_VERSION=$build_version
cargo build \
    --manifest-path "$repository_root/Cargo.toml" \
    --release \
    --target "$target_triple" \
    --locked
dotnet restore "$project" --runtime "$runtime"
dotnet publish "$project" \
    --configuration Release \
    --runtime "$runtime" \
    --self-contained true \
    --no-restore \
    -p:PublishTrimmed=false \
    -p:DebugType=None \
    -p:Version="$build_version" \
    --output "$publish_output"

cp -R "$publish_output/." "$app_macos/"
cp "$repository_root/target/$target_triple/release/yt-dlp-wrapper-backend" "$app_macos/"
chmod 755 "$app_macos/yt-dlp-wrapper" "$app_macos/yt-dlp-wrapper-backend"
find "$app_macos" -name '*.pdb' -delete

sed \
    -e "s/@VERSION@/$build_version/g" \
    -e "s/@BUNDLE_VERSION@/$bundle_version/g" \
    "$repository_root/assets/macos/Info.plist" >"$app_bundle/Contents/Info.plist"
cp "$repository_root/COPYING" "$repository_root/README.md" "$repository_root/THIRD_PARTY.md" "$app_resources/"

codesign --force --deep --sign - "$app_bundle"
rm -f "$dist_root/YT-DLP-Wrapper-macOS-arm64.zip"
ditto -c -k --sequesterRsrc --keepParent \
    "$app_bundle" \
    "$dist_root/YT-DLP-Wrapper-macOS-arm64.zip"

echo "Built $app_bundle"
echo "Built $dist_root/YT-DLP-Wrapper-macOS-arm64.zip"
