#!/usr/bin/env bash

##	- Purpose: Cut a release locally from main. No hosted CI: the tag, the
##	  artifacts, and the optional GitHub Release upload all happen on this box.
##	- Flow (run AFTER merging dev into main --no-ff):
##	   1. verify: on main, clean tree, version bumped, README badge matches
##	   2. run the full pipeline if the release artifacts are missing/stale
##	   3. tag the merge: v<version>, where <version> comes from source/Cargo.toml
##	      alone (the build stamps from it too, so they can never disagree)
##	   4. --push: push main + the tag
##	   5. --publish: also attach cicd/artifacts/release/* to a GitHub Release
##	      as plain uploads (gh CLI; no Actions)
##	- Syntax:
##	  cicd/utility/release.bash [--push] [--publish] [-y]
##	  With no flags it tags only and prints the remaining steps.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


set -Eeuo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${here}/../.." && pwd)"
cd "${root}"
source "${here}/../config.bash"

do_push=0; do_publish=0; assume_yes=0
while (($#)); do case "$1" in
	--push)    do_push=1; shift ;;
	--publish) do_push=1; do_publish=1; shift ;;
	-y|--yes)  assume_yes=1; shift ;;
	-h|--help) sed -n '/^##	- Purpose:/,/^##	Copyright/p' "${BASH_SOURCE[0]}" | sed '$d; s/^##	\{0,1\}//'; exit 0 ;;
	*) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
esac; done

die(){ echo "FAILED: $*" >&2; exit 1; }

## 1. Preconditions: releases only cut from a clean main, with the version
## already bumped on dev (so no commit ever lands directly on main here).
branch="$(git rev-parse --abbrev-ref HEAD)"
[[ "$branch" == "main" ]] || die "not on main (on ${branch}); merge dev --no-ff into main first"
git diff --quiet && git diff --cached --quiet || die "working tree not clean"

ver="$(sed -n 's/^version *= *"\(.*\)".*/\1/p' "${VERSION_MANIFEST}" | head -1)"
[[ -n "$ver" ]] || die "no version in ${VERSION_MANIFEST}"
tag="v${ver}"
git rev-parse -q --verify "refs/tags/${tag}" >/dev/null && die "tag ${tag} already exists - bump the version on dev first"

## The README release badge is static; it must be bumped on dev with the version
## (shields.io escapes '-' as '--'), never patched here on main.
badge_ver="${ver//-/--}"
grep -q "Release-${badge_ver}-" README.md || die "README release badge does not say ${ver} - update it on dev before the release merge"

## 2. Release artifacts must exist and carry this version (full cicd run makes them).
art_dir="${RELEASE_ARTIFACT_DIR}"
sums="${art_dir}/${EXE_NAME}-${ver}-sha256sums.txt"
[[ -s "$sums" ]] || die "no ${sums} - run cicd/cicd.bash (full, not --quick) first"
( cd "${art_dir}" && sha256sum -c "${EXE_NAME}-${ver}-sha256sums.txt" >/dev/null ) || die "artifact checksums do not verify"

echo ""
echo "Release ${tag} from $(git rev-parse --short HEAD) on main"
echo "Artifacts:"; ls -1 "${art_dir}/${EXE_NAME}-${ver}-"* | sed 's/^/  /'
echo "Push: ${do_push}  Publish (gh): ${do_publish}"
if ((! assume_yes)); then read -r -p "Proceed? [y/N] " a; [[ "$a" == [yY]* ]] || exit 1; fi

## 3. Tag the merge.
git tag -a "${tag}" -m "${tag}"
echo "tagged ${tag}"

## 4/5. Push and publish.
if ((do_push)); then
	git push origin main
	git push origin "${tag}"
	echo "pushed main + ${tag}"
else
	echo "next: git push origin main && git push origin ${tag}"
fi
if ((do_publish)); then
	command -v gh >/dev/null 2>&1 || die "gh CLI not found"
	gh release create "${tag}" --title "${APP_NAME} ${ver}" --notes "See the README for details." \
		"${art_dir}/${EXE_NAME}-${ver}-"*
	echo "GitHub Release ${tag} created with artifacts"
elif ((do_push)); then
	echo "next (optional): gh release create ${tag} ${art_dir}/${EXE_NAME}-${ver}-*"
fi
