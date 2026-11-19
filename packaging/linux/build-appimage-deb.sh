#!/usr/bin/env bash
set -euo pipefail

VERSION="0.1.0"
ARCH="x86_64"
APPDIR="packaging/dist/AppDir"

cargo build --release --workspace
rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin" "packaging/dist"
cp target/release/rmpd-launcher target/release/rmpd-ui "${APPDIR}/usr/bin/"

cat > "${APPDIR}/rmpd-nextgen.desktop" <<'EOF'
[Desktop Entry]
Name=RMPD-NextGen
Exec=rmpd-launcher
Icon=rmpd-nextgen
Type=Application
Categories=AudioVideo;Audio;Player;
MimeType=audio/flac;audio/mpeg;audio/ogg;audio/wav;
EOF

echo "AppDir staged at ${APPDIR} (ready for appimagetool / dpkg-deb)."
