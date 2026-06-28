# shellcheck shell=bash
# Reusable GFS (grandfather-father-son) file rotation. Source this and call:
#
#   gfs_rotate <dir> <prefix> <ext>
#
# Keeps a bounded, time-spread set of files and prunes the rest: the newest of
# each recent hour/day/week/month/year, plus the last N most recent ("frequent"),
# plus the very first (kept forever) - about 30 files total.
#
# Period roles are RETROSPECTIVE: a file is tagged hour/day/week/month/year only
# once that period has ended and it is the last file in it; until then it stays
# "frequent". Kept files are renamed to a canonical, naturally-sorting name:
#
#   <prefix>_<YYYYmmDD-HHMMSS>_<role>.<ext>
#
# The constant <prefix> is first and the sortable timestamp second, so a plain
# directory listing is chronological. Pre-existing files that don't follow the
# convention are conformed: the timestamp is parsed from the name, else taken
# from the file mtime. Re-running is idempotent (already-canonical files are left
# alone until their role actually changes).
#
# Retention is tunable via env (defaults sum to ~30): GFS_KEEP_FREQUENT,
# GFS_KEEP_HOURLY, GFS_KEEP_DAILY, GFS_KEEP_WEEKLY, GFS_KEEP_MONTHLY,
# GFS_KEEP_YEARLY. GFS_NOW (epoch seconds) overrides "now" for testing.

# Echo "<epoch> <YYYYmmDD-HHMMSS>" for a file, from its name if it carries a
# date, else from its mtime.
_gfs_ts(){
	local base d="" t="000000" epoch=""
	base="$(basename "$1")"
	if [[ "$base" =~ (19|20)([0-9]{2})([0-9]{2})([0-9]{2})[-_]?([0-9]{2})([0-9]{2})([0-9]{2}) ]]; then
		d="${BASH_REMATCH[1]}${BASH_REMATCH[2]}${BASH_REMATCH[3]}${BASH_REMATCH[4]}"
		t="${BASH_REMATCH[5]}${BASH_REMATCH[6]}${BASH_REMATCH[7]}"
	elif [[ "$base" =~ (19|20)([0-9]{2})([0-9]{2})([0-9]{2}) ]]; then
		d="${BASH_REMATCH[1]}${BASH_REMATCH[2]}${BASH_REMATCH[3]}${BASH_REMATCH[4]}"
	fi
	[[ -n "$d" ]] && epoch="$(date -d "${d:0:4}-${d:4:2}-${d:6:2} ${t:0:2}:${t:2:2}:${t:4:2}" +%s 2>/dev/null)"
	[[ -n "$epoch" ]] || epoch="$(stat -c %Y "$1" 2>/dev/null)"
	printf '%s %s' "${epoch}" "$(date -d "@${epoch}" +%Y%m%d-%H%M%S)"
}

gfs_rotate(){
	local dir="$1" prefix="$2" ext="$3"
	local now="${GFS_NOW:-$(date +%s)}"
	local kFreq="${GFS_KEEP_FREQUENT:-10}" kHour="${GFS_KEEP_HOURLY:-4}" \
	      kDay="${GFS_KEEP_DAILY:-5}" kWeek="${GFS_KEEP_WEEKLY:-4}" \
	      kMonth="${GFS_KEEP_MONTHLY:-4}" kYear="${GFS_KEEP_YEARLY:-2}"

	shopt -s nullglob; local cands=("$dir/${prefix}"_*."$ext"); shopt -u nullglob
	((${#cands[@]})) || return 0

	# "epoch<TAB>canon<TAB>path", oldest first.
	local -a items=(); local f e ct
	for f in "${cands[@]}"; do
		read -r e ct < <(_gfs_ts "$f")
		[[ -n "$e" ]] && items+=("${e}"$'\t'"${ct}"$'\t'"${f}")
	done
	((${#items[@]})) || return 0
	mapfile -t items < <(printf '%s\n' "${items[@]}" | sort -n)

	# Latest file in each *completed* period (the still-open current one is skipped
	# so it can't be tagged yet - that is what makes the roles retrospective).
	local curH curD curW curM curY
	curH="$(date -d "@$now" +%Y%m%d%H)"; curD="$(date -d "@$now" +%Y%m%d)"
	curW="$(date -d "@$now" +%G%V)";     curM="$(date -d "@$now" +%Y%m)"; curY="$(date -d "@$now" +%Y)"
	local -A pH pD pW pM pY; local it kH kD kW kM kY
	# shellcheck disable=SC2034  # pH..pY are populated here, read later through the namerefs
	for it in "${items[@]}"; do
		e="${it%%$'\t'*}"
		kH="$(date -d "@$e" +%Y%m%d%H)"; kD="$(date -d "@$e" +%Y%m%d)"
		kW="$(date -d "@$e" +%G%V)";     kM="$(date -d "@$e" +%Y%m)"; kY="$(date -d "@$e" +%Y)"
		[[ "$kH" != "$curH" ]] && pH["$kH"]="$it"
		[[ "$kD" != "$curD" ]] && pD["$kD"]="$it"
		[[ "$kW" != "$curW" ]] && pW["$kW"]="$it"
		[[ "$kM" != "$curM" ]] && pM["$kM"]="$it"
		[[ "$kY" != "$curY" ]] && pY["$kY"]="$it"
	done

	# Assign the coarsest role to each kept file: first > year > month > week >
	# day > hour > frequent (first-set wins, so process coarsest first).
	local -A role; role["${items[0]}"]="first"
	local spec rn cnt nk i
	for spec in "year pY $kYear" "month pM $kMonth" "week pW $kWeek" "day pD $kDay" "hour pH $kHour"; do
		# shellcheck disable=SC2086
		set -- $spec; rn="$1"; cnt="$3"; local -n arr="$2"
		local -a keys=("${!arr[@]}")
		if ((${#keys[@]})); then
			mapfile -t keys < <(printf '%s\n' "${keys[@]}" | sort)
			nk=${#keys[@]}
			for ((i = nk>cnt ? nk-cnt : 0; i<nk; i++)); do
				it="${arr[${keys[i]}]}"; [[ -z "${role[$it]:-}" ]] && role["$it"]="$rn"
			done
		fi
		unset -n arr
	done

	# Frequent: most recent kFreq not already claimed by a coarser role.
	local ni=${#items[@]}
	for ((i = ni>kFreq ? ni-kFreq : 0; i<ni; i++)); do
		[[ -z "${role[${items[i]}]:-}" ]] && role["${items[i]}"]="frequent"
	done

	# Prune the unrole'd; rename the kept to canonical (no-op if already canonical).
	local rest r want
	for it in "${items[@]}"; do
		rest="${it#*$'\t'}"; ct="${rest%%$'\t'*}"; f="${rest#*$'\t'}"
		r="${role[$it]:-}"
		if [[ -z "$r" ]]; then
			rm -f "$f"; printf '  rotate: pruned %s\n' "$(basename "$f")"
		else
			want="$dir/${prefix}_${ct}_${r}.${ext}"
			if [[ "$f" != "$want" ]]; then
				[[ -e "$want" ]] && continue   # never clobber a same-name collision
				mv -f "$f" "$want"; printf '  rotate: %s -> %s\n' "$(basename "$f")" "$(basename "$want")"
			fi
		fi
	done
}
