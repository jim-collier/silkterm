#!/usr/bin/env bash

##	Purpose:
##		- Non-interactive GIT_EDITOR for the cicd publish stage.
##	 	  git invokes it as `git-auto-msg.bash <msgfile>`. If the message is empty
##		  (a plain `git commit` with no -m), fill it from $GIT_AUTO_MESSAGE; if git already pre-filled one
##		  (e.g. a `pull --no-ff` merge message), leave it. Either way, never block.

set -euo pipefail

file="$1"

## Anything left after dropping comment + blank lines means git pre-filled a message.
if [[ -z "$(grep -vE '^[[:space:]]*#' "$file" | tr -d '[:space:]')" ]]; then
	printf '%s\n' "${GIT_AUTO_MESSAGE:-CI/CD automated commit}" >"$file"
fi
