#!/usr/bin/env bash

##	- Purpose:
##		A release may only publish artifacts built from the source being tagged.
##		The checksums only say the artifacts match each other, which let a stale
##		build go out under a new tag with everything reporting green.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier [ID: 2უNაɘ«҂թȹɤξπ๙¿ձϖ]
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

## Signing. The checksums file says the download was not corrupted; the signature
## is what says it came from here. Driven with a throwaway key: signed the way
## release.bash signs, and checked by each installer's OWN verify function, so a
## retyped command cannot pass where the installer fails.
if command -v ssh-keygen >/dev/null 2>&1; then
	keyDir="${work}/key"
	mkdir -p "${keyDir}"
	ssh-keygen -q -t ed25519 -N "" -C releases@silkterm -f "${keyDir}/id" </dev/null
	namespace="silkterm-release"
	tag="v0.0.0-test"
	mkdir -p "${work}/dl/${tag}" "${work}/inst"
	printf 'checksum line\n' > "${work}/dl/${tag}/sums.txt"
	ssh-keygen -Y sign -f "${keyDir}/id" -n "${namespace}" "${work}/dl/${tag}/sums.txt" >/dev/null 2>&1

	## install.bash, sourced: its entry block is guarded, the release page is a
	## folder, and fGet copies from it. fFail exits, so each run is a subshell.
	fBashInstaller(){
		cp "${1}" "${work}/inst/sums.txt"
		## the settings and fGet are read by the sourced function, not by us
		# shellcheck disable=SC2034,SC2329
		(
			# shellcheck source=/dev/null
			source "${root}/../install.bash"
			releaseSignPubkey="$(cat "${keyDir}/id.pub")"
			dlBase="${work}/dl"
			fGet(){ cp "${1}" "${2}"; }
			fVerifySignature "${work}/inst" sums.txt "${tag}"
		) >/dev/null 2>&1
	}
	fCheck "install.bash accepts a signed checksums file" fBashInstaller "${work}/dl/${tag}/sums.txt"

	printf 'checksum line tampered\n' > "${work}/tampered.txt"
	if fBashInstaller "${work}/tampered.txt"; then
		echo "  FAIL install.bash accepted a changed checksums file"; failures=$((failures + 1))
	else
		echo "  ok   install.bash refuses a changed one"
	fi

	## and a signature by a different key is not the release key's
	ssh-keygen -q -t ed25519 -N "" -C other -f "${keyDir}/other" </dev/null
	rm -f "${work}/dl/${tag}/sums.txt.sig"   ## or ssh-keygen stops to ask about overwriting
	ssh-keygen -Y sign -f "${keyDir}/other" -n "${namespace}" "${work}/dl/${tag}/sums.txt" >/dev/null 2>&1
	if fBashInstaller "${work}/dl/${tag}/sums.txt"; then
		echo "  FAIL install.bash accepted another key's signature"; failures=$((failures + 1))
	else
		echo "  ok   install.bash refuses another key's signature"
	fi

	## install.ps1 the same way, wherever pwsh is. The driver makes its own key.
	if command -v pwsh >/dev/null 2>&1; then
		pwsh -NoProfile -File "${meDir}/verify-sign.ps1" -Installer "${root}/../install.ps1" || failures=$((failures + 1))
	else
		echo "  skip install.ps1 signing (no pwsh)"
	fi
else
	echo "  skip signing (no ssh-keygen)"
fi

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
##		- 20260917 JC: The installers' own verify functions, and install.ps1 through verify-sign.ps1.
