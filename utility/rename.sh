#!/usr/bin/env bash
# Rename the project everywhere (development-time helper).
#
#   utility/rename.sh <NewDisplayName>
#
# <NewDisplayName> is the human-facing name (e.g. "SilkTerm"). The lowercase
# identifier used for the cargo package, the binary, and the config directory
# is derived from it (e.g. "silkterm").
#
# Replaces the display name (SilkTerm) and the id (silkterm) across Cargo.toml,
# all Rust sources, and the Markdown docs. Cargo.lock is left for `cargo build`
# to regenerate. Review `git diff` afterwards.
set -euo pipefail

new_display="${1:-}"
if [[ -z "$new_display" ]]; then
	echo "usage: utility/rename.sh <NewDisplayName>" >&2
	exit 1
fi
case "$new_display" in
	*/* | *'&'* | *'\'*)
		echo "error: name must not contain / & or backslash" >&2
		exit 1
		;;
esac

new_id="$(printf '%s' "$new_display" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9_-')"
if [[ -z "$new_id" ]]; then
	echo "error: '$new_display' yields no usable lowercase identifier" >&2
	exit 1
fi

old_display="SilkTerm"
old_id="silkterm"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Tracked text files, plus any matching ones present in the working tree
# (covers files mid-move where the index and worktree disagree).
mapfile -t files < <(
	{ git ls-files; git ls-files --others --exclude-standard; } \
		| grep -E '(^Cargo\.toml$|\.rs$|\.md$)' | grep -vx 'utility/rename.sh' | sort -u
)

for f in "${files[@]}"; do
	[[ -f "$f" ]] || continue
	sed -i "s/${old_display}/${new_display}/g; s/${old_id}/${new_id}/g" "$f"
done

echo "Renamed across ${#files[@]} files:"
echo "  display : ${old_display} -> ${new_display}"
echo "  id      : ${old_id} -> ${new_id}"
echo
echo "Next steps:"
echo "  - review 'git diff'"
echo "  - 'cargo build' (regenerates Cargo.lock with the new package name)"
echo "  - if you rename the GitHub repo too, update the 'git remote' URL"
