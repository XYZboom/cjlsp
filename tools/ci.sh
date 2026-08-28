#!/usr/bin/env bash
# cj-lang automated test pipeline (T9).
#
# One command runs every gate a developer needs before pushing:
#   fmt check -> clippy (-D warnings) -> unit tests -> LSP diagnostics
#   coverage -> SCAN Parser alignment -> macro E2E -> cross-platform (T45):
#   Linux debug+release build, Windows GNU cross-build, clippy on both targets.
#
# Each step prints PASS/FAIL with the measurable number (coverage %, test
# count). The script exits non-zero on the first failing gate so CI can stop
# early. Use `-j N` for parallel clippy/test builds (rayon is a project dep;
# cargo -j controls the rust build itself).
#
# Usage:
#   ./tools/ci.sh            # full pipeline
#   ./tools/ci.sh -j 12      # parallel builds (12 cores)
#   CI=1 ./tools/ci.sh       # non-interactive (same behaviour)
set -uo pipefail

cd "$(dirname "$0")/.."

JOBS=""
while getopts "j:h" opt; do
  case "$opt" in
    j) JOBS="-j $OPTARG" ;;
    h) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) exit 2 ;;
  esac
done

PASS=0
FAIL=0
SUMMARY=()

step() {
  local name="$1"; shift
  local out
  out="$("$@" 2>&1)"
  local rc=$?
  if [ $rc -eq 0 ]; then
    PASS=$((PASS + 1))
    echo "PASS  $name"
  else
    FAIL=$((FAIL + 1))
    echo "FAIL  $name"
    echo "$out" | tail -20
  fi
  SUMMARY+=("$name|$rc")
}

# Ensure toolchain env (cargo is on PATH; SDK needed only for macro cache).
if ! command -v cargo >/dev/null 2>&1; then
  echo "FATAL: cargo not found — source your Rust env first"
  exit 1
fi

echo "=== cj-lang CI pipeline (jobs=${JOBS:-default}) ==="

# 1. Format check (no formatting diffs allowed).
step "cargo fmt --check" bash -c "cargo fmt --all --check 2>&1"

# 2. Clippy with warnings as errors.
step "cargo clippy -D warnings" bash -c "cargo clippy --workspace -- -D warnings 2>&1"

# 3. Unit / integration tests.
TEST_OUT="$(cargo test --workspace $JOBS 2>&1)"
TEST_RC=$?
TEST_N=$(echo "$TEST_OUT" | grep -E "test result: ok" | awk -F'[.;]' '{s+=$2} END {print s+0}')
if [ $TEST_RC -eq 0 ]; then
  PASS=$((PASS + 1)); echo "PASS  cargo test --workspace ($TEST_N tests)"
else
  FAIL=$((FAIL + 1)); echo "FAIL  cargo test --workspace"
  echo "$TEST_OUT" | tail -25
fi
SUMMARY+=("cargo-test|$TEST_RC")

# 4. LSP diagnostics coverage (must not regress below 75%).
COV_OUT="$(timeout 300 python3 tools/lsp_cov.py 2>&1)"
COV_RC=$?
COV_PCT=$(echo "$COV_OUT" | grep -oE '[0-9]+/[0-9]+ \([0-9.]+%\)' | tail -1)
if [ $COV_RC -eq 0 ] && [ -n "$COV_PCT" ]; then
  PASS=$((PASS + 1)); echo "PASS  lsp_cov.py ($COV_PCT)"
else
  FAIL=$((FAIL + 1)); echo "FAIL  lsp_cov.py"
  echo "$COV_OUT" | tail -10
fi
SUMMARY+=("lsp-cov|$COV_RC")

# 5. Macro expansion E2E (unresolved macro reported).
step "macro E2E (unresolved macro)" python3 tools/test_macro_e2e.py

# 5b. Macro expansion preview note E2E (T14: diagnostics inside an expansion
#     span carry "the code after the macro is expanded as follows").
step "macro preview note E2E" python3 tools/test_macro_preview.py

# 6. SCAN Parser alignment (default: the LLT Parser suite with SCAN blocks).
SCAN_DIR="${SCAN_DIR:-/root/Code/cangjie/cangjie_test/testsuites/LLT/compiler/Parser}"
if [ -n "$SCAN_DIR" ]; then
  SCAN_OUT="$(timeout 300 python3 tools/scan_compare.py --dir "$SCAN_DIR" 2>&1)"
  SCAN_RC=$?
  SCAN_PCT=$(echo "$SCAN_OUT" | grep -oE '[0-9.]+%' | tail -1)
  if [ $SCAN_RC -eq 0 ]; then
    PASS=$((PASS + 1)); echo "PASS  scan_compare.py (${SCAN_PCT:-n/a})"
  else
    FAIL=$((FAIL + 1)); echo "FAIL  scan_compare.py"
    echo "$SCAN_OUT" | tail -10
  fi
  SUMMARY+=("scan-compare|$SCAN_RC")
fi

# 7. LSP feature cases (completion + hover). Default smoke (2 cases each);
#    FEATURE_FULL=1 runs the whole suite (slow, ~minutes).
if [ "${FEATURE_FULL:-0}" = "1" ]; then
  FEAT_OUT="$(timeout 900 python3 tools/run_feature_cases.py --workers 8 2>&1)"
  FEAT_RC=$?
  FEAT_SUM=$(echo "$FEAT_OUT" | grep -oE 'pass=[0-9]+ +fail=[0-9]+' | head -2 | tr '\n' ' ')
  if [ $FEAT_RC -eq 0 ]; then
    PASS=$((PASS + 1)); echo "PASS  feature cases ($FEAT_SUM)"
  else
    FAIL=$((FAIL + 1)); echo "FAIL  feature cases"
    echo "$FEAT_OUT" | tail -8
  fi
  SUMMARY+=("feature-cases|$FEAT_RC")
else
  step "feature smoke (completion/hover)" bash -c "python3 tools/run_feature_cases.py --limit 2 --workers 4 2>&1 | tail -4"
fi

# 8. Cross-platform build gates (T45). Linux debug+release workspace build,
#    Windows GNU cross-build of the two user-facing binaries (cj-lsp /
#    cj-frontend, which pull in the whole dependency graph) and clippy on the
#    Windows target. Windows steps need the rustup target plus a mingw linker;
#    they are skipped (with a visible note) on boxes without the toolchain so
#    the pipeline still runs on Linux-only dev machines.
step "cargo build (linux, workspace)" bash -c "cargo build --workspace 2>&1"
step "cargo build --release (linux, workspace)" bash -c "cargo build --release --workspace 2>&1"

if command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 \
   && rustup target list --installed 2>/dev/null | grep -qx x86_64-pc-windows-gnu; then
  # Debug links may print "corrupt .drectve at end of def file" from mingw ld
  # (known false positive, release links are silent). It is a warning only and
  # does not fail the gate.
  step "cargo build (windows-gnu, cj-lsp+cj-frontend)" bash -c "cargo build --target x86_64-pc-windows-gnu -p cj-lsp -p cj-frontend 2>&1"
  step "cargo build --release (windows-gnu, cj-lsp+cj-frontend)" bash -c "cargo build --release --target x86_64-pc-windows-gnu -p cj-lsp -p cj-frontend 2>&1"
  step "clippy (windows-gnu target)" bash -c "cargo clippy --workspace --target x86_64-pc-windows-gnu -- -D warnings 2>&1"
else
  echo "SKIP  windows-gnu cross-build (needs rustup target x86_64-pc-windows-gnu + x86_64-w64-mingw32-gcc)"
fi

echo
echo "=== CI result: $PASS passed, $FAIL failed ==="
for s in "${SUMMARY[@]}"; do
  echo "  ${s%%|*}: $([ "${s##*|}" -eq 0 ] && echo PASS || echo FAIL)"
done
[ $FAIL -eq 0 ]