#!/usr/bin/env bash

##	- Purpose:
##		A release may only publish artifacts built from the source being tagged.
##		The checksums only say the artifacts match each other, which let a stale
##		build go out under a new tag with everything reporting green.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier (CryptogID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	SPDX-License-Identifier: GPL-2.0-or-later

set -euo pipefail
meDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${meDir}/../.." && pwd)"
# shellcheck source=cicd/utility/built-from.bash
source "${root}/utility/built-from.bash"

failures=0
fCheck(){ local -r what="${1}"; shift; if "${@}"; then echo "  ok   ${what}"; else echo "  FAIL ${what}"; failures=$((failures + 1)); fi; }

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
cd "${work}"
git init -q .
git -c user.name=t -c user.email=t@t commit -q --allow-empty -m first
mkdir art

fNot(){ ! fCheckBuiltFrom art >/dev/null; }
fYes(){ fCheckBuiltFrom art >/dev/null; }

fCheck "no note at all is refused" fNot

fWriteBuiltFrom art
fCheck "a note written here and now is accepted" fYes

## More commits, then the release is cut: the artifacts are now stale.
echo change > file.txt
git add file.txt
git -c user.name=t -c user.email=t@t commit -q -m second
fCheck "a stale artifact directory is refused" fNot

## A merge keeps the tree, so a release cut from main after 'dev -> main --no-ff'
## still matches what the pipeline built on dev.
fWriteBuiltFrom art
git -c user.name=t -c user.email=t@t checkout -q -b rel
git -c user.name=t -c user.email=t@t commit -q --allow-empty -m "merge (same tree)"
fCheck "the same tree on another branch is accepted" fYes

## A dirty tree at build time says nothing about what was built.
echo more >> file.txt
fWriteBuiltFrom art
fCheck "a build from a dirty tree is refused" fNot

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
