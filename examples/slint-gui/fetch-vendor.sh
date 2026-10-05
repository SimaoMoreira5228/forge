#!/bin/sh
set -eu
cd "$(dirname "$0")"
VERSION=1.18.1
CORROSION=0.6.1
mkdir -p vendor
if [ ! -f vendor/slint/api/cpp/CMakeLists.txt ]; then
	curl -sL --fail -o vendor/slint.tar.gz "https://codeload.github.com/slint-ui/slint/tar.gz/refs/tags/v$VERSION"
	rm -rf vendor/slint
	mkdir -p vendor/slint
	tar xzf vendor/slint.tar.gz -C vendor/slint --strip-components=1
	rm vendor/slint.tar.gz
fi
if [ ! -f vendor/corrosion/CMakeLists.txt ]; then
	curl -sL --fail -o vendor/corrosion.tar.gz "https://codeload.github.com/corrosion-rs/corrosion/tar.gz/refs/tags/v$CORROSION"
	rm -rf vendor/corrosion
	mkdir -p vendor/corrosion
	tar xzf vendor/corrosion.tar.gz -C vendor/corrosion --strip-components=1
	rm vendor/corrosion.tar.gz
fi
test -f vendor/slint/api/cpp/CMakeLists.txt
test -f vendor/corrosion/CMakeLists.txt
echo "vendored Slint $VERSION and Corrosion $CORROSION under vendor/"
