#!/bin/dash
#  shellcheck disable=2086  ## $BENCH_ARGS is a multi-argument string and has to split.
#                              $LABEL must NOT: it can contain a space, and splitting it
#                              silently killed every run of the first campaign.
# Runs inside the terminal under test.
#
# While it waits for the go file it reports its own grid, which is what lets one
# fitter size any terminal to the same grid without knowing that terminal's geometry
# options or cell metrics.

cd "$REPO_DIR" || exit 1

while [ ! -f "$GO_FILE" ]; do
	stty size > "$SIZE_FILE.tmp" 2>/dev/null && mv "$SIZE_FILE.tmp" "$SIZE_FILE"
	sleep 0.2
done

# stdout has to stay on the tty: the tool stops its clock on the terminal's own reply,
# so redirecting it would measure a pipe instead. The report goes out via --out.
python3 utility/termbench.py $BENCH_ARGS --label "$LABEL" --out "$OUT_FILE" 2>"$OUT_FILE.err"
echo "exit=$?" > "$OUT_FILE.done"
