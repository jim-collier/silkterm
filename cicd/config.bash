# shellcheck shell=bash
# shellcheck disable=SC2034  ## these settings are consumed by cicd.bash, which sources this file
# Project-specific CI/CD settings. To reuse this pipeline in another project,
# copy the whole cicd/ directory and edit THIS file (cicd.bash stays generic).
# All command arrays run from the repo root. The engine prepends ~/.cargo/bin to
# PATH so the rustup toolchain (cross targets, edition 2024) wins over system rust.

# Identity
APP_NAME="SilkTerm"
EXE_NAME="silkterm"

# Stage 1: debug build (fast compile sanity)
DEBUG_BUILD_CMD=(cargo build)

# Stage 2: regression tests
TEST_CMD=(cargo test)

# Stage 4: native release build + its artifact (this is what gets dogfooded)
RELEASE_NATIVE_CMD=(cargo build --release)
RELEASE_NATIVE_BIN="target/release/${EXE_NAME}"

# Stage 4: cross-release targets. One per line: "label|artifact|command...".
# Set BUILD_CROSS=0 to skip them for a quick local run.
BUILD_CROSS=1
CROSS_TARGETS=(
	"Windows x86_64 (mingw)|target/x86_64-pc-windows-gnu/release/${EXE_NAME}.exe|cargo build --release --target x86_64-pc-windows-gnu"
	"Linux ARM64 (zig)|target/aarch64-unknown-linux-gnu/release/${EXE_NAME}|cargo zigbuild --release --target aarch64-unknown-linux-gnu"
	"Windows ARM64 (zig)|target/aarch64-pc-windows-gnullvm/release/${EXE_NAME}.exe|cargo zigbuild --release --target aarch64-pc-windows-gnullvm"
)

# Stage 3: profiler (non-gating artifact, not a pass/fail test). Builds an
# optimized+symbols binary (cargo --profile $PROFILE_PROFILE --features
# $PROFILE_FEATURE), runs the real app under an in-process sampler against a heavy
# workload for $PROFILE_SECS, and writes a flamegraph SVG. See cicd.bash for the
# skip-vs-abort failure policy ($PROFILE_STRICT to force abort on any failure).
PROFILE_ENABLE=1
PROFILE_SECS=8
PROFILE_FEATURE="profiling"
PROFILE_PROFILE="profiling"
PROFILE_BIN="target/profiling/${EXE_NAME}"
PROFILE_WORKLOAD_SCRIPT="cicd/utility/n8output-random-unicode.py"
PROFILE_WORKLOAD_ARGS="600 0"          # <duration_s> <delay_s>; duration >> PROFILE_SECS, no delay = max output
PROFILE_OUT_DIR="../private/profiling"  # relative to repo root; created if missing (kept out of git)
PROFILE_KEEP_FREQUENT=15                # GFS retention: also keep first + newest-per-day/month/year
PROFILE_STRICT=0                        # 1 = any profiler failure aborts the pipeline

# Stage 5: dogfood the native release here (first existing dir wins)
DOGFOOD_DESTS=(
	"${HOME}/synced/0-0/common/exec/util/linux/bin"
	"/usr/local/sbin"
)

# Stage 6: backup + publish to git (runs from repo root).
GIT_PUBLISH=(cicd/utility/n8git_backup-and-publish)
# Set a non-empty commit message to publish hands-off (suppresses the script's
# prompt and supplies the message so `git commit` won't open an editor). Left
# empty, publish is interactive unless -m/--message or -y is given (see cicd.bash).
PUBLISH_AUTO_MESSAGE=""
