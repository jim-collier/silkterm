#!/usr/bin/env bash

#  shellcheck disable=2155  ## 'Declare and assign separately.' Cumbersome and unnecessary here.

##	Purpose:
##		Render the README's wallpaper contact sheet from the shipped wallpaper pack.
##		Every image is centre-cropped to one tile size and tiled into a single JPEG,
##		so the gallery costs the README one request instead of one per wallpaper.
##		This is a rendered artifact: it goes stale silently when images are added,
##		removed or replaced, so re-run it in the same commit that touches the pack.
##	Syntax:
##		wallpaper-gallery.bash [--cols N] [--tile WxH] [--src DIR] [--out FILE] [--quality N]
##		  --cols N      tiles per row (default 9)
##		  --tile WxH    tile size in pixels (default 160x100)
##		  --src DIR     wallpaper folder (default: the pack under filesystem/)
##		  --out FILE    output image (default: assets/wallpaper-gallery.jpg)
##		  --quality N   ffmpeg -q:v for the sheet, 2 best .. 31 worst (default 4)
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
out="${repoDir}/assets/wallpaper-gallery.jpg"
declare -i cols=9 quality=4
tile="160x100"

while (($#)); do case "$1" in
	--cols)     cols="${2:-}"; shift 2 ;;
	--tile)     tile="${2:-}"; shift 2 ;;
	--src)      src="${2:-}"; shift 2 ;;
	--out)      out="${2:-}"; shift 2 ;;
	--quality)  quality="${2:-}"; shift 2 ;;
	-h|--help)  grep -E '^##' "$0" | sed 's/^##\t\?//'; exit 0 ;;
	*) echo "wallpaper-gallery: unknown option: $1" >&2; exit 2 ;;
esac; done

fEcho()       { echo "[ $* ]"; }
fEcho_Clean() { echo "$*"; }
fSkip()       { echo "wallpaper-gallery: $1" >&2; exit 2; }   ## 2 = non-fatal skip, as elsewhere in cicd

command -v ffmpeg >/dev/null || fSkip "ffmpeg not found"
[[ -d "$src" ]] || fSkip "no wallpaper folder: $src"

declare -i tileW="${tile%%x*}" tileH="${tile##*x}"
((tileW > 0 && tileH > 0 && cols > 0)) || fSkip "bad --tile/--cols: ${tile}, ${cols}"

##	ffmpeg on Windows is a native binary and cannot read an MSYS path, so hand it a
##	native one where cygpath exists. A no-op everywhere else.
fNativePath() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

##	Collect the pack in a stable order - the filenames lead with their source, so
##	alphabetical keeps each collection together on the sheet.
declare -a images=()
while IFS= read -r f; do images+=("$f"); done < <(find "$src" -maxdepth 1 -type f \
	\( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' \) | LC_ALL=C sort)
declare -i count="${#images[@]}"
((count)) || fSkip "no images in $src"

declare -i rows=$(( (count + cols - 1) / cols ))

fEcho_Clean
fEcho "Wallpaper gallery"
fEcho_Clean "  Source   ${src}"
fEcho_Clean "  Images   ${count} in ${cols}x${rows} tiles of ${tileW}x${tileH}"
fEcho_Clean "  Output   ${out}"
fEcho_Clean

tmpDir="$(mktemp -d)"
trap 'rm -rf "$tmpDir"' EXIT

##	Pass 1: one tile per image, filled and centre-cropped. The pack's own XMP anchors
##	are all 50%,50%, so a centre crop is what each image asks for anyway.
declare -i i=0
for f in "${images[@]}"; do
	i+=1
	ffmpeg -y -v error -i "$(fNativePath "$f")" \
		-vf "scale=${tileW}:${tileH}:force_original_aspect_ratio=increase,crop=${tileW}:${tileH}" \
		-q:v 2 "${tmpDir}/$(printf '%03d' "$i").jpg" \
		|| fSkip "could not render a tile for: $(basename "$f")"
done
fEcho "Rendered ${i} tiles"

##	Pass 2: tile them. The last row is short whenever the count does not divide
##	evenly; those cells come out as the padding colour.
ffmpeg -y -v error -framerate 1 -i "$(fNativePath "${tmpDir}")/%03d.jpg" \
	-filter_complex "tile=${cols}x${rows}:margin=6:padding=6:color=#14141a" \
	-frames:v 1 -q:v "$quality" "$(fNativePath "$out")"

fEcho "Wrote $(du -h "$out" | cut -f1) to ${out}"
fEcho_Clean


##	Script history:
##		- 20260819: Created.
