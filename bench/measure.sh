#!/bin/sh
# bench/measure.sh — time cold, warm, single-edit, and test runs for every
# build system described in bench/app. The app sources are identical for all
# systems; each system builds lib + bin + test from them.
#
# Usage: bench/measure.sh [app-dir]
#
# Tool selection, in order: $FORGE_BIN / $BUCK2_BIN / $BAZEL_BIN, else PATH.
# Missing tools are skipped, never fatal. Set CC to pick the C compiler
# (defaults to /usr/bin/gcc); CCACHE_DISABLE=1 is forced so caches can't
# mask real work.

set -u
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
APP="${1:-$SCRIPT_DIR/app}"
cd "$APP" || exit 1
APP="$(pwd)"

export CCACHE_DISABLE=1
export CC="${CC:-/usr/bin/gcc}"

FORGE_BIN="${FORGE_BIN:-$SCRIPT_DIR/../target/release/forge}"
BUCK2_BIN="${BUCK2_BIN:-$(command -v buck2 || true)}"
BAZEL_BIN="${BAZEL_BIN:-$(command -v bazelisk || command -v bazel || true)}"

now_ns() { date +%s%N; }
elapsed() { awk "BEGIN {printf \"%.2f\", ($2 - $1) / 1000000000}"; }

REPORT=""
record() { REPORT="$REPORT$1|$2|$3
"; }

run_case() {
	system="$1"; label="$2"; shift 2
	start="$(now_ns)"
	if "$@" >/tmp/opencode-bench-log 2>&1; then
		status="ok"
	else
		status="FAIL"
	fi
	record "$system" "$label" "$(elapsed "$start" "$(now_ns)") $status"
	if [ "$status" = "FAIL" ]; then
		echo "--- $system $label failed:"; tail -n 5 /tmp/opencode-bench-log
	fi
}

have() { command -v "$1" >/dev/null 2>&1; }

if [ -x "$FORGE_BIN" ]; then
	rm -rf forge-out
	run_case forge cold "$FORGE_BIN" build
	run_case forge warm "$FORGE_BIN" build
	touch lib/math.c
	run_case forge touch "$FORGE_BIN" build
	sed -i 's/return a + b + 0;/return a + b + 1;/' lib/math.c
	run_case forge edit "$FORGE_BIN" build
	sed -i 's/return a + b + 1;/return a + b + 0;/' lib/math.c
	"$FORGE_BIN" build >/dev/null 2>&1
	run_case forge test "$FORGE_BIN" test
else
	echo "skip forge: no binary at $FORGE_BIN (build it with: cargo build --release -p forge)"
fi

if have cmake && have ninja; then
	rm -rf build
	export CC
	cmake -G Ninja -DCMAKE_BUILD_TYPE=Debug -B build . >/dev/null 2>&1
	run_case cmake cold ninja -C build
	run_case cmake warm ninja -C build
	touch lib/math.c
	run_case cmake touch ninja -C build
	sed -i 's/return a + b + 0;/return a + b + 1;/' lib/math.c
	run_case cmake edit ninja -C build
	sed -i 's/return a + b + 1;/return a + b + 0;/' lib/math.c
	ninja -C build >/dev/null 2>&1
	run_case cmake test sh -c "cd build && ctest"
else
	echo "skip cmake: need cmake and ninja on PATH"
fi

if have meson && have ninja; then
	rm -rf builddir
	export CC
	meson setup --buildtype=debug builddir >/dev/null 2>&1
	run_case meson cold ninja -C builddir
	run_case meson warm ninja -C builddir
	touch lib/math.c
	run_case meson touch ninja -C builddir
	sed -i 's/return a + b + 0;/return a + b + 1;/' lib/math.c
	run_case meson edit ninja -C builddir
	sed -i 's/return a + b + 1;/return a + b + 0;/' lib/math.c
	ninja -C builddir >/dev/null 2>&1
	run_case meson test sh -c "cd builddir && meson test"
else
	echo "skip meson: need meson and ninja on PATH"
fi

if [ -n "$BUCK2_BIN" ]; then
	rm -rf buck-out
	"$BUCK2_BIN" kill >/dev/null 2>&1 || true
	run_case buck2 cold sh -c "cd '$APP' && '$BUCK2_BIN' build //:hello //:math_test"
	run_case buck2 warm sh -c "cd '$APP' && '$BUCK2_BIN' build //:hello //:math_test"
	touch lib/math.c
	run_case buck2 touch sh -c "cd '$APP' && '$BUCK2_BIN' build //:hello //:math_test"
	sed -i 's/return a + b + 0;/return a + b + 1;/' lib/math.c
	run_case buck2 edit sh -c "cd '$APP' && '$BUCK2_BIN' build //:hello //:math_test"
	sed -i 's/return a + b + 1;/return a + b + 0;/' lib/math.c
	sh -c "cd '$APP' && '$BUCK2_BIN' build //:hello //:math_test" >/dev/null 2>&1
	run_case buck2 test sh -c "cd '$APP' && '$BUCK2_BIN' test //:math_test"
else
	echo "skip buck2: set BUCK2_BIN or put buck2 on PATH"
fi

if [ -n "$BAZEL_BIN" ]; then
	sh -c "cd '$APP' && '$BAZEL_BIN' shutdown" >/dev/null 2>&1 || true
	run_case bazel cold sh -c "cd '$APP' && '$BAZEL_BIN' build //:hello //:math_test"
	run_case bazel warm sh -c "cd '$APP' && '$BAZEL_BIN' build //:hello //:math_test"
	touch lib/math.c
	run_case bazel touch sh -c "cd '$APP' && '$BAZEL_BIN' build //:hello //:math_test"
	sed -i 's/return a + b + 0;/return a + b + 1;/' lib/math.c
	run_case bazel edit sh -c "cd '$APP' && '$BAZEL_BIN' build //:hello //:math_test"
	sed -i 's/return a + b + 1;/return a + b + 0;/' lib/math.c
	sh -c "cd '$APP' && '$BAZEL_BIN' build //:hello //:math_test" >/dev/null 2>&1
	run_case bazel test sh -c "cd '$APP' && '$BAZEL_BIN' test //:math_test"
else
	echo "skip bazel: set BAZEL_BIN or put bazelisk/bazel on PATH"
fi

echo
echo "system | case | seconds result"
echo "$REPORT" | grep . | sort | awk -F'|' '{printf "%-6s | %-5s | %s\n", $1, $2, $3}'
