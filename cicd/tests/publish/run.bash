#!/usr/bin/env bash

##	- Purpose:
##		The publish script commits and pushes, so nothing the caller's environment
##		holds may be executed inside it. It used to eval one of its own variables.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier [ID: 2უNაɘ«҂թȹɤξπ๙¿ձϖ]
##	SPDX-License-Identifier: GPL-2.0-or-later

set -euo pipefail
meDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${meDir}/../.." && pwd)"
publish="${root}/utility/n8git_backup-and-publish"
config="${root}/config.bash"

failures=0
fCheck(){ local -r what="${1}"; shift; if "${@}"; then echo "  ok   ${what}"; else echo "  FAIL ${what}"; failures=$((failures + 1)); fi; }

fCheck "nothing in the publish script is eval'd" \
	bash -c '! grep -Eq "(^|[^[:alnum:]_])eval[[:space:]]" "$1"' _ "${publish}"

## The exclude list is read line by line now. Replay that here with a value that
## would have run a command under the old eval, and check nothing ran.
canary="$(mktemp -u "${TMPDIR:-/tmp}/silk-publish-canary.XXXXXX")"
GIT_BACKUP_AND_PUBLISH_RAR_EXCLUDES="*/ok
\$(touch '${canary}')
\`touch '${canary}'\`"
declare -a args=()
while IFS= read -r pattern; do
	[[ -n "${pattern}" ]] && args+=("-x${pattern}")
done <<<"${GIT_BACKUP_AND_PUBLISH_RAR_EXCLUDES}"

fCheck "every line becomes one argument" test "${#args[@]}" -eq 3
fCheck "and the first is the pattern it was given" test "${args[0]}" = "-x*/ok"
fCheck "nothing in the value ran" test ! -e "${canary}"
rm -f "${canary}"

## The shipped list and the reader still agree.
# shellcheck disable=SC1090
source "${config}"
declare -a shipped=()
while IFS= read -r pattern; do
	[[ -n "${pattern}" ]] && shipped+=("-x${pattern}")
done <<<"${GIT_BACKUP_AND_PUBLISH_RAR_EXCLUDES}"
fCheck "the shipped excludes are plain patterns" \
	bash -c 'for a in "$@"; do case "$a" in *\'"'"'*|*\"*) exit 1 ;; esac; done' _ "${shipped[@]}"
fCheck "and cicd/artifacts is among them" \
	bash -c 'for a in "$@"; do [[ "$a" == "-x*/cicd/artifacts" ]] && exit 0; done; exit 1' _ "${shipped[@]}"

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
