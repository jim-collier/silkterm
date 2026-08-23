#!/usr/bin/env bash

#  shellcheck disable=2155  ## 'Declare and assign separately.' Cumbersome and unnecessary here.

##	Purpose:
##		Render the two browsable views of the shipped wallpaper pack.
##		  1. The README contact sheet: every image centre-cropped to one tile size and
##		     tiled into a single JPEG, so the README costs one request, not 113.
##		  2. The Pages gallery under docs/: a thumbnail grid whose tiles open the full
##		     image in place, with prev/next paging. Full images are NOT copied - the
##		     page fetches them from the pack in the repository, so nothing is duplicated.
##		Both are rendered artifacts: they go stale silently when images are added,
##		removed or replaced, so re-run this in the same commit that touches the pack.
##	Syntax:
##		wallpaper-gallery.bash [--cols N] [--tile WxH] [--src DIR] [--out FILE] [--quality N]
##		                       [--thumb WxH] [--docs DIR] [--raw-base URL]
##		                       [--sheet-only | --page-only]
##		  --cols N       contact-sheet tiles per row (default 9)
##		  --tile WxH     contact-sheet tile size in pixels (default 160x100)
##		  --src DIR      wallpaper folder (default: the pack under filesystem/)
##		  --out FILE     contact sheet (default: assets/wallpaper-gallery.jpg)
##		  --quality N    ffmpeg -q:v for the sheet, 2 best .. 31 worst (default 4)
##		  --thumb WxH    gallery thumbnail size in pixels (default 360x225)
##		  --docs DIR     Pages site root (default: docs)
##		  --raw-base URL where the page fetches full images from
##		  --sheet-only   render only the contact sheet
##		  --page-only    render only the Pages gallery
##	Needs: ffmpeg. Exit: 0 rendered, 2 skip (no ffmpeg / no images).
##	History: At bottom of script.

##	Copyright © 2026 Bubbles (ID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


set -Eeuo pipefail

meDir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repoDir="${meDir}/../.."                      ## this script lives in cicd/utility
src="${repoDir}/filesystem/home/.config/silkterm/wallpaper"
attrib="${repoDir}/filesystem/home/.config/silkterm/wallpaper-attribution.md"
out="${repoDir}/assets/wallpaper-gallery.jpg"
docs="${repoDir}/docs"
template="${meDir}/wallpaper-gallery.html"
rawBase="https://raw.githubusercontent.com/jim-collier/silkterm/main/filesystem/home/.config/silkterm/wallpaper/"
declare -i cols=9 quality=4 thumbQuality=5
declare -i doSheet=1 doPage=1
tile="160x100"
thumb="360x225"

while (($#)); do case "$1" in
	--cols)       cols="${2:-}"; shift 2 ;;
	--tile)       tile="${2:-}"; shift 2 ;;
	--src)        src="${2:-}"; shift 2 ;;
	--out)        out="${2:-}"; shift 2 ;;
	--quality)    quality="${2:-}"; shift 2 ;;
	--thumb)      thumb="${2:-}"; shift 2 ;;
	--docs)       docs="${2:-}"; shift 2 ;;
	--raw-base)   rawBase="${2:-}"; shift 2 ;;
	--sheet-only) doPage=0; shift ;;
	--page-only)  doSheet=0; shift ;;
	-h|--help)    grep -E '^##' "$0" | sed 's/^##\t\?//'; exit 0 ;;
	*) echo "wallpaper-gallery: unknown option: $1" >&2; exit 2 ;;
esac; done

fEcho()       { echo "[ $* ]"; }
fEcho_Clean() { echo "$*"; }
fSkip()       { echo "wallpaper-gallery: $1" >&2; exit 2; }   ## 2 = non-fatal skip, as elsewhere in cicd

command -v ffmpeg >/dev/null || fSkip "ffmpeg not found"
[[ -d "$src" ]] || fSkip "no wallpaper folder: $src"

declare -i tileW="${tile%%x*}" tileH="${tile##*x}"
declare -i thumbW="${thumb%%x*}" thumbH="${thumb##*x}"
((tileW > 0 && tileH > 0 && cols > 0)) || fSkip "bad --tile/--cols: ${tile}, ${cols}"
((thumbW > 0 && thumbH > 0))           || fSkip "bad --thumb: ${thumb}"
if ((doPage)) && [[ ! -r "$template" ]]; then fSkip "no page template: ${template}"; fi

##	ffmpeg on Windows is a native binary and cannot read an MSYS path, so hand it a
##	native one where cygpath exists. A no-op everywhere else.
fNativePath() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

##	One image, filled and centre-cropped to a fixed box. The pack's own XMP anchors
##	are all 50%,50%, so a centre crop is what each image asks for anyway.
fCrop() {
	local -r from="$1" to="$2"; local -ri w="$3" h="$4" q="$5"
	ffmpeg -y -v error -i "$(fNativePath "$from")" \
		-vf "scale=${w}:${h}:force_original_aspect_ratio=increase,crop=${w}:${h}" \
		-q:v "$q" "$(fNativePath "$to")" \
		|| fSkip "could not render a tile for: $(basename "$from")"
}

##	Collect the pack in a stable order - the filenames lead with their source, so
##	alphabetical keeps each collection together on the sheet and in the grid.
declare -a images=()
while IFS= read -r f; do images+=("$f"); done < <(find "$src" -maxdepth 1 -type f \
	\( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' \) | LC_ALL=C sort)
declare -i count="${#images[@]}"
((count)) || fSkip "no images in $src"

declare -i rows=$(( (count + cols - 1) / cols ))

fEcho_Clean
fEcho "Wallpaper gallery"
fEcho_Clean "  Source   ${src}"
fEcho_Clean "  Images   ${count}"
((doSheet)) && fEcho_Clean "  Sheet    ${cols}x${rows} tiles of ${tileW}x${tileH} -> ${out}"
((doPage))  && fEcho_Clean "  Page     thumbnails of ${thumbW}x${thumbH} -> ${docs}/wallpapers/"
fEcho_Clean

tmpDir="$(mktemp -d)"
trap 'rm -rf "$tmpDir"' EXIT


##	The README contact sheet

if ((doSheet)); then
	declare -i i=0
	for f in "${images[@]}"; do
		i+=1
		fCrop "$f" "${tmpDir}/$(printf '%03d' "$i").jpg" "$tileW" "$tileH" 2
	done
	fEcho "Rendered ${i} sheet tiles"

	##	The last row is short whenever the count does not divide evenly; those cells
	##	come out as the padding colour.
	ffmpeg -y -v error -framerate 1 -i "$(fNativePath "${tmpDir}")/%03d.jpg" \
		-filter_complex "tile=${cols}x${rows}:margin=6:padding=6:color=#14141a" \
		-frames:v 1 -q:v "$quality" "$(fNativePath "$out")"

	fEcho "Wrote $(du -h "$out" | cut -f1) to ${out}"
	fEcho_Clean
fi


##	The Pages gallery

if ((doPage)); then
	pageDir="${docs}/wallpapers"
	mkdir -p "${pageDir}/thumb"
	rm -f "${pageDir}/thumb/"*.jpg

	declare -i i=0
	for f in "${images[@]}"; do
		i+=1
		fCrop "$f" "${pageDir}/thumb/$(printf '%03d' "$i").jpg" "$thumbW" "$thumbH" "$thumbQuality"
	done
	fEcho "Rendered ${i} thumbnails ($(du -sh --apparent-size "${pageDir}/thumb" | cut -f1))"

	##	Provenance, joined on the file name. The table's columns are, in order:
	##	confidence, stars, file name, original name, original date, source URL,
	##	copyright, licence - and a leading empty field, since the row starts with '|'.
	if [[ -r "$attrib" ]]; then
		awk -F'|' 'NF>=14 && $4 ~ /\.(jpg|jpeg|png)[[:space:]\r]*$/ {
			n=$4; u=$7; c=$8; l=$9
			gsub(/^[ \t]+|[ \t\r]+$/,"",n); gsub(/^[ \t]+|[ \t\r]+$/,"",u)
			gsub(/^[ \t]+|[ \t\r]+$/,"",c); gsub(/^[ \t]+|[ \t\r]+$/,"",l)
			sub(/^</,"",u); sub(/>$/,"",u)
			if (u == "-") u = ""
			if (c == "-") c = ""
			if (l == "-") l = ""
			print n "\t" c "\t" l "\t" u
		}' "$attrib" > "${tmpDir}/attrib.tsv"
	else
		: > "${tmpDir}/attrib.tsv"
		fEcho "No attribution table at ${attrib} - the page will carry names only"
	fi

	printf '%s\n' "${images[@]}" | sed 's:.*/::' > "${tmpDir}/names.txt"

	##	One JSON record per image: file name, title, slug (which is what a permalink
	##	names, so it survives the pack being reordered), credit, licence, source URL.
	##	Only " and \ need escaping; the text travels as UTF-8 either way. An image
	##	with no attribution row is REPORTED - that is how the table goes stale.
	awk -F'\t' '
		function esc(s) { gsub(/\\/,"\\\\",s); gsub(/"/,"\\\"",s); return s }
		NR==FNR { cr[$1]=$2; lic[$1]=$3; url[$1]=$4; next }
		{
			n=$0; sub(/\r$/,"",n)
			t=n; sub(/\.[^.]*$/,"",t)
			s=tolower(t); gsub(/[^a-z0-9]+/,"-",s); gsub(/^-+|-+$/,"",s)
			if (s in seen) { seen[s]++; s=s "-" seen[s] } else seen[s]=1
			if (!(n in cr)) missing = missing "\n    " n
			printf "%s\t\t{\"f\":\"%s\",\"t\":\"%s\",\"s\":\"%s\",\"c\":\"%s\",\"l\":\"%s\",\"u\":\"%s\"}",
				(FNR>1 ? ",\n" : ""), esc(n), esc(t), esc(s), esc(cr[n]), esc(lic[n]), esc(url[n])
		}
		END {
			print ""
			if (missing != "") print "wallpaper-gallery: no attribution row for:" missing > "/dev/stderr"
		}' "${tmpDir}/attrib.tsv" "${tmpDir}/names.txt" > "${tmpDir}/data.json"

	##	Splice the records into the template. Line-oriented rather than a sed
	##	substitution, so nothing in a file name can be read as a replacement.
	awk -v raw="$rawBase" -v data="${tmpDir}/data.json" '
		BEGIN { gsub(/&/, "\\&", raw) }   ## & is the matched text in a sub() replacement
		{ sub(/@@RAW@@/, raw) }
		/^[[:space:]]*\/\/@@DATA@@[[:space:]]*$/ { while ((getline line < data) > 0) print line; next }
		{ print }' "$template" > "${pageDir}/index.html"

	##	Jekyll would process the site otherwise; there is nothing here for it to do.
	: > "${docs}/.nojekyll"

	##	The site root redirects, so whatever else lands under Pages later can have it.
	{
		echo '<!doctype html>'
		echo '<meta charset="utf-8">'
		echo '<title>SilkTerm</title>'
		echo '<meta http-equiv="refresh" content="0; url=wallpapers/">'
		echo '<link rel="canonical" href="wallpapers/">'
		echo '<p><a href="wallpapers/">SilkTerm wallpaper pack</a></p>'
	} > "${docs}/index.html"

	fEcho "Wrote ${pageDir}/index.html - ${count} images"
	fEcho_Clean
fi


##	Script history:
##		- 20260819: Created.
##		- 20260819: Added the Pages gallery - thumbnail grid, in-place viewer, prev/next.
