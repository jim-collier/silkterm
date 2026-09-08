#!/usr/bin/env bash

##	- Purpose:
##		Tie a set of release artifacts to the source they were built from, so a
##		stale artifact directory cannot be published under a new tag. Sourced by
##		cicd.bash (which writes the note) and release.bash (which checks it).
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier (CryptogID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	SPDX-License-Identifier: GPL-2.0-or-later

##	The TREE, not the commit. A release is cut from main after 'dev -> main
##	--no-ff', so HEAD is a different commit from the one the pipeline built -
##	but the merge keeps the tree, and the tree is what the binaries came from.
##	The two branches have diverged in ancestry before now without diverging in
##	content, which is exactly the case a commit comparison gets wrong.

BUILT_FROM_FILE="built-from.txt"

## fWriteBuiltFrom <artifact-dir>
fWriteBuiltFrom(){
	local -r dir="${1}"
	[[ -d "${dir}" ]] || return 0
	local dirty="no"
	git diff --quiet && git diff --cached --quiet || dirty="yes"
	{
		echo "tree $(git rev-parse "HEAD^{tree}")"
		echo "commit $(git rev-parse HEAD)"
		echo "dirty ${dirty}"
	} > "${dir}/${BUILT_FROM_FILE}"
}

## fCheckBuiltFrom <artifact-dir>
## Echoes why on a mismatch and returns non-zero.
fCheckBuiltFrom(){
	local -r dir="${1}"
	local -r note="${dir}/${BUILT_FROM_FILE}"
	if [[ ! -s "${note}" ]]; then
		echo "no ${note} - the artifacts do not say what they were built from; re-run the pipeline"
		return 1
	fi
	local builtTree="" builtCommit="" dirty=""
	builtTree="$(  sed -n 's/^tree //p'   "${note}")"
	builtCommit="$(sed -n 's/^commit //p' "${note}")"
	dirty="$(      sed -n 's/^dirty //p'  "${note}")"
	if [[ "${dirty}" == "yes" ]]; then
		echo "the artifacts were built from a dirty working tree; re-run the pipeline"
		return 1
	fi
	local -r here="$(git rev-parse "HEAD^{tree}")"
	if [[ "${builtTree}" != "${here}" ]]; then
		echo "the artifacts were built from ${builtCommit:0:12} (tree ${builtTree:0:12}), not from what is checked out (tree ${here:0:12}); re-run the pipeline"
		return 1
	fi
	return 0
}

##	History:
##		- 20260908 JC: Created - the release could publish a stale build under a
##		  new tag with everything reporting green.
