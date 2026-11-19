#!/usr/bin/env bash
set -euo pipefail

VERSION="0.1.0"
APP_BUNDLE="packaging/dist/RMPD-NextGen.app"

rustup target add x86_64-apple-darwin aarch64-apple-darwin
cargo build --release --workspace --target x86_64-apple-darwin
cargo build --release --workspace --target aarch64-apple-darwin

mkdir -p "${APP_BUNDLE}/Contents/MacOS"
lipo -create -output "${APP_BUNDLE}/Contents/MacOS/rmpd-launcher" \
  target/x86_64-apple-darwin/release/rmpd-launcher \
  target/aarch64-apple-darwin/release/rmpd-launcher
lipo -create -output "${APP_BUNDLE}/Contents/MacOS/rmpd-ui" \
  target/x86_64-apple-darwin/release/rmpd-ui \
  target/aarch64-apple-darwin/release/rmpd-ui

hdiutil create -volname "RMPD-NextGen" -srcfolder "${APP_BUNDLE}" -ov -format UDZO \
  "packaging/dist/rmpd-nextgen-v${VERSION}-macos-universal.dmg"
