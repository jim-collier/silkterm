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

## Runs against a real repository from here on: a bare remote and a clone of it,
## with git's own config kept out so nothing on this box decides the result.
work="$(mktemp -d "${TMPDIR:-/tmp}/silk-publish.XXXXXX")"
trap 'rm -rf "${work}"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset GIT_CONFIG_COUNT GIT_DIR GIT_WORK_TREE GIT_AUTO_MESSAGE GIT_BACKUP_AND_PUBLISH_MESSAGE
export GIT_BACKUP_AND_PUBLISH_NOBACKUP=1 GIT_EDITOR=false

fRepo(){  ## fRepo <name>: a remote, a clone of it at <name>/proj with one commit
	local -r dir="${work}/${1}"
	mkdir -p "${dir}"
	git init -q --bare -b main "${dir}/remote.git"
	git clone -q "${dir}/remote.git" "${dir}/proj" 2>/dev/null
	git -C "${dir}/proj" config user.name t; git -C "${dir}/proj" config user.email t@t
	printf 'one\ntwo\nthree\nfour\n' >"${dir}/proj/file.txt"
	git -C "${dir}/proj" add file.txt; git -C "${dir}/proj" commit -qm first; git -C "${dir}/proj" push -q origin main 2>/dev/null
}
fPublish(){  ## fPublish <name> <args...>: run the publisher in that clone; sets rc and out
	local -r dir="${work}/${1}/proj"; shift
	rc=0; out="$(cd "${dir}" && bash "${publish}" -q "${@}" 2>&1)" || rc=$?
}
fUpstream(){  ## fUpstream <name> <line1>: another clone pushes a change to line one
	local -r dir="${work}/${1}"
	git clone -q "${dir}/remote.git" "${dir}/other" 2>/dev/null
	git -C "${dir}/other" config user.name t; git -C "${dir}/other" config user.email t@t
	printf '%s\ntwo\nthree\nfour\n' "${2}" >"${dir}/other/file.txt"
	git -C "${dir}/other" commit -qam upstream; git -C "${dir}/other" push -q origin main 2>/dev/null
}
fLastMsg(){ git -C "${work}/${1}/remote.git" log -1 --format=%B main; }

## A failed pull leaves the work where it was, tracked and untracked, with no stash.
fRepo gone
printf 'one\ntwo\nthree\nmine\n' >"${work}/gone/proj/file.txt"; echo new >"${work}/gone/proj/untracked.txt"
mv "${work}/gone/remote.git" "${work}/gone/moved.git"
fPublish gone -m "should not go"
fCheck "an unreachable remote fails the run" test "${rc}" -ne 0
fCheck "and says nothing was pushed" grep -q "Nothing was committed or pushed" <<<"${out}"
fCheck "the tracked change is back in the tree" grep -qx mine "${work}/gone/proj/file.txt"
fCheck "the untracked file is back" test -f "${work}/gone/proj/untracked.txt"
fCheck "and nothing is left stashed" test -z "$(git -C "${work}/gone/proj" stash list)"

## A reachable remote still stashes, pulls and pops.
fRepo fine
printf 'one\ntwo\nthree\nmine\n' >"${work}/fine/proj/file.txt"
fUpstream fine theirs
fPublish fine -m "both"
fCheck "a reachable remote publishes" test "${rc}" -eq 0
fCheck "with the pulled change and the local one" test "$(git -C "${work}/fine/remote.git" show main:file.txt)" = "theirs
two
three
mine"

## A pop that conflicts still stops and says what to do.
fRepo clash
printf 'mine\ntwo\nthree\nfour\n' >"${work}/clash/proj/file.txt"
fUpstream clash theirs
fPublish clash -m "clash"
fCheck "a conflicting pop fails the run" test "${rc}" -ne 0
fCheck "and says how to finish" grep -q "then 'git stash drop'" <<<"${out}"

## A message is committed byte for byte, and a flag inside it is only text.
msg="don't \"quote\" -v and -h"
fRepo words
echo more >>"${work}/words/proj/file.txt"
fPublish words -m "${msg}"
fCheck "a message with quotes and flags publishes" test "${rc}" -eq 0
fCheck "and is committed as given" test "$(fLastMsg words)" = "${msg}"
fRepo inline
echo more >>"${work}/inline/proj/file.txt"
fPublish inline "--msg=it's -v"
fCheck "an inline message is committed as given" test "$(fLastMsg inline)" = "it's -v"
fRepo ver
echo more >>"${work}/ver/proj/file.txt"
fPublish ver -v
fCheck "a bare -v still only shows the version" test "$(fLastMsg ver)" = "first"

## cicd.bash: a blank answer at the publish prompt takes the message its plan
## names, and the publisher commits that message.
# shellcheck disable=SC2034  ## read by the sourced function
APP_NAME=Silk stamp=20260101-000000
# shellcheck disable=SC1090
source <(sed -n '/^auto_msg=/,/^}/p' "${root}/cicd.bash")
fCheck "the plan names the automatic message" grep -qF '(will prompt for message; blank = \"${auto_msg}\")' "${root}/cicd.bash"
fCheck "the prompt does too" grep -qF 'Publish commit message (blank = \"${auto_msg}\"' "${root}/cicd.bash"
fCheck "and its answer goes through the same choice" grep -qF 'publish_msg="$(fPublishMessage "" "" "$m")"' "${root}/cicd.bash"
fCheck "a blank answer takes the automatic message" test "$(fPublishMessage "" "" "")" = "Silk CI/CD 20260101-000000"
fCheck "a typed answer is used as typed" test "$(fPublishMessage "" "" "typed")" = "typed"
fCheck "--message still wins" test "$(fPublishMessage "cli" "cfg" "")" = "cli"
fRepo blank
echo more >>"${work}/blank/proj/file.txt"
rc=0; out="$(cd "${work}/blank/proj" && GIT_BACKUP_AND_PUBLISH_QUIET=1 GIT_AUTO_MESSAGE="$(fPublishMessage "" "" "")" bash "${publish}" --quiet 2>&1)" || rc=$?
fCheck "and that is the message committed" test "$(fLastMsg blank)" = "Silk CI/CD 20260101-000000"

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
##		- 20260917 JC: A failed pull, a message as given, and the blank prompt answer.
