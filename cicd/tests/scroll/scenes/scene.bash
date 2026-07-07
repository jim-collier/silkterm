#!/bin/dash

## Deterministic full-redraw scroll scene for the SILK_SCROLLDBG harness. Models the
## paint shape of a real full-screen app so the alt-screen slide can be measured
## without key injection - it self-scrolls on a timer. Shape ($1):
##   less   - content from the top, one static bottom status line (no top band)
##   vim    - content from the top, two static bottom rows (status + command line)
##   nano   - one static title bar on top, two static help rows at the bottom
##   muffer - two static header rows on top, one static footer row
## Uses explicit cursor positioning (CUP) and never a newline, so nothing scrolls the
## real grid - only the drawn content shifts, exactly the way curses/nano repaint.
## POSIX sh (dash): no backticks (dash would run them and leak the temp path).

shape="${1:-less}"
settle="${SILK_SCENE_SETTLE:-13}"   ## seconds to idle past the GL pipeline warmup
step="${SILK_SCENE_STEP:-0.15}"     ## seconds between repaints (one line/step)

printf '\033[?1049h\033[2J'                       ## enter alt screen, clear
trap 'printf "\033[?1049l"' EXIT INT TERM         ## restore on the way out
sleep "$settle"

case "$shape" in
	nano)   top=1; bot=2 ;;
	muffer) top=2; bot=1 ;;
	vim)    top=0; bot=2 ;;
	*)      top=0; bot=1 ;;   ## less
esac

n=0
while :; do
	sz=$(stty size 2>/dev/null) || sz=""
	rows=${sz% *}
	case "$rows" in ''|*[!0-9]*) rows=30 ;; esac
	[ "$rows" -ge 10 ] || rows=30

	## static top band (title bar / header): constant across frames
	r=1
	while [ "$r" -le "$top" ]; do
		printf '\033[%d;1H\033[7m  header line %d (static)  \033[0m\033[K' "$r" "$r"
		r=$((r + 1))
	done

	## scrolling middle region: the value at a fixed row grows by 1 each frame, so
	## the content moves up one line per step (a clean forward translate).
	r=$((top + 1))
	midbot=$((rows - bot))
	while [ "$r" -le "$midbot" ]; do
		printf '\033[%d;1H  line %06d  the quick brown fox jumps\033[K' "$r" "$((n + r))"
		r=$((r + 1))
	done

	## static bottom band (status / help): constant across frames
	r=$((rows - bot + 1))
	while [ "$r" -le "$rows" ]; do
		printf '\033[%d;1H\033[7m  status/help line (static)  \033[0m\033[K' "$r"
		r=$((r + 1))
	done

	printf '\033[%d;1H' "$rows"   ## park the cursor (harmless)
	n=$((n + 1))
	sleep "$step"
done
