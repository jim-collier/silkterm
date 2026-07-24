#!/usr/bin/env bash
# shellcheck disable=SC2317  ## unreachable-after-exit false positives in dispatchers

##	- Purpose: One-liner installer for SilkTerm on Linux (BSD/macOS builds are
##	  not published yet - it says so and points at building from source).
##	  Downloads the latest release binary from GitHub, verifies its sha256
##	  against the release's checksums file, and installs it. Idempotent: states
##	  its plan, asks before touching anything, and does nothing when the
##	  installed binary is already current.
##	- Syntax:
##	  bash <(curl -fsSL https://raw.githubusercontent.com/jim-collier/silkterm/main/install.bash) [--release stable|dev] [--target user|system] [--arch x64|amd64|arm64] [--yes]
##	- Options:
##	  --release  stable (default) = latest full release; dev = newest release
##	             including pre-releases. With no full release published yet,
##	             stable falls back to dev with a note.
##	  --target   user (default) = ~/.local/bin; system = /usr/local/bin (sudo)
##	  --arch     override the detected CPU architecture
##	  --yes      skip the confirmation prompt
##	- Needs: bash >= 3.2, curl, sha256sum or shasum.
##	- History:
##	  - 20260723 JC: Created.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


ownerRepo="jim-collier/silkterm"
exeName="silkterm"
apiBase="https://api.github.com/repos/${ownerRepo}"
dlBase="https://github.com/${ownerRepo}/releases/download"

function fFail() { echo "Error: $*" >&2; echo >&2; exit 1; }

function fConfirm() {
	local answer=""
	printf "%s [y/N]: " "$1"
	read -r answer </dev/tty || answer=""
	case "${answer}" in y|Y|yes|Yes|YES) return 0 ;; *) return 1 ;; esac
}

##	Pull the first "tag_name" out of a GitHub API JSON body (no jq dependency).
function fFirstTag() {
	sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

function fSha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | awk '{print $1}'
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | awk '{print $1}'
	else
		fFail "need sha256sum or shasum to verify the download"
	fi
}

function fMain() {

	echo

	## Parse arguments
	local release="stable" target="user" archArg="" assumeYes=0
	while (($#)); do case "$1" in
		--release) release="${2:-}"; shift 2 ;;
		--release=*) release="${1#*=}"; shift ;;
		--target) target="${2:-}"; shift 2 ;;
		--target=*) target="${1#*=}"; shift ;;
		--arch) archArg="${2:-}"; shift 2 ;;
		--arch=*) archArg="${1#*=}"; shift ;;
		--yes|-y) assumeYes=1; shift ;;
		-h|--help) sed -n '/^##	- Purpose:/,/^##	Copyright/p' "${BASH_SOURCE[0]:-/dev/null}" 2>/dev/null | sed '$d; s/^##	\{0,1\}//'; exit 0 ;;
		*) fFail "unknown option: $1 (try --help)" ;;
	esac; done
	case "${release}" in stable|dev) : ;; *) fFail "--release must be stable or dev" ;; esac
	case "${target}" in user|system) : ;; *) fFail "--target must be user or system" ;; esac

	command -v curl >/dev/null 2>&1 || fFail "curl is required"

	## Detect OS
	local os; os="$(uname -s)"
	case "${os}" in
		Linux) : ;;
		Darwin) fFail "macOS builds are not published yet - please build from source: https://github.com/${ownerRepo}#building-from-source" ;;
		*BSD|DragonFly) fFail "BSD builds are not published yet - please build from source: https://github.com/${ownerRepo}#building-from-source" ;;
		*) fFail "unsupported OS: ${os}" ;;
	esac

	## Detect/normalize architecture
	local arch="${archArg}"
	[[ -n "${arch}" ]] || arch="$(uname -m)"
	case "$(echo "${arch}" | tr '[:upper:]' '[:lower:]')" in
		x64|amd64|x86_64) arch="x86_64" ;;
		arm64|aarch64) arch="arm64" ;;
		*) fFail "unsupported architecture: ${arch} (use --arch x64|arm64)" ;;
	esac
	local osArch="linux-${arch}"

	## Resolve the release tag
	echo "Looking up the latest ${release} release of SilkTerm ..."
	local tag=""
	if [[ "${release}" == "stable" ]]; then
		tag="$(curl -fsSL "${apiBase}/releases/latest" 2>/dev/null | fFirstTag)" || true
		if [[ -z "${tag}" ]]; then
			echo "No full release published yet; using the newest pre-release instead."
			release="dev"
		fi
	fi
	if [[ "${release}" == "dev" && -z "${tag}" ]]; then
		tag="$(curl -fsSL "${apiBase}/releases?per_page=5" 2>/dev/null | fFirstTag)" || true
	fi
	[[ -n "${tag}" ]] || fFail "no releases found at github.com/${ownerRepo} (releases may not be published yet - see the README for building from source)"
	local version="${tag#v}"

	## Work out names, paths, and the plan
	local asset="${exeName}-${version}-${osArch}"
	local sums="${exeName}-${version}-sha256sums.txt"
	local destDir destFile appDir sudoCmd=""
	if [[ "${target}" == "user" ]]; then
		destDir="${HOME}/.local/bin"
		appDir="${HOME}/.local/share/applications"
	else
		destDir="/usr/local/bin"
		appDir="/usr/local/share/applications"
		[[ "$(id -u)" == "0" ]] || sudoCmd="sudo"
	fi
	destFile="${destDir}/${exeName}"

	echo
	echo "Plan:"
	echo "  Release:  ${tag} (${release})"
	echo "  Download: ${dlBase}/${tag}/${asset}"
	echo "  Verify:   sha256 against ${sums}"
	echo "  Install:  ${destFile}"
	echo "  Launcher: ${appDir}/${exeName}.desktop"
	[[ -n "${sudoCmd}" ]] && echo "  (system target: install steps run under sudo)"
	echo
	if ((!assumeYes)); then
		fConfirm "Proceed?" || { echo "Aborted - nothing was touched."; echo; exit 0; }
		echo
	fi

	## Download + verify
	local tmpDir; tmpDir="$(mktemp -d)" || fFail "mktemp failed"
	trap 'rm -rf "${tmpDir}"' EXIT
	echo "Downloading ${asset} ..."
	curl -fSL --progress-bar -o "${tmpDir}/${asset}" "${dlBase}/${tag}/${asset}" || fFail "download failed (no ${osArch} build in release ${tag}?)"
	curl -fsSL -o "${tmpDir}/${sums}" "${dlBase}/${tag}/${sums}" || fFail "checksums file missing from release ${tag}"
	local wantSha haveSha
	wantSha="$(awk -v f="${asset}" '$2 == f || $2 == "*"f {print $1}' "${tmpDir}/${sums}" | head -n 1)"
	[[ -n "${wantSha}" ]] || fFail "no checksum entry for ${asset} in ${sums}"
	haveSha="$(fSha256 "${tmpDir}/${asset}")"
	[[ "${haveSha}" == "${wantSha}" ]] || fFail "checksum mismatch (expected ${wantSha}, got ${haveSha}) - not installing"
	echo "Checksum OK."

	## Idempotence: already current?
	if [[ -f "${destFile}" ]] && [[ "$(fSha256 "${destFile}")" == "${wantSha}" ]]; then
		echo
		echo "Already up to date: ${destFile} is ${tag}. Nothing to do."
		echo
		exit 0
	fi

	## Install
	echo
	echo "Installing ..."
	${sudoCmd} mkdir -p "${destDir}" "${appDir}"
	${sudoCmd} install -m 0755 "${tmpDir}/${asset}" "${destFile}"
	{
		echo "[Desktop Entry]"
		echo "Type=Application"
		echo "Name=SilkTerm"
		echo "GenericName=Terminal"
		echo "Comment=Smooth-scrolling GPU terminal with split panes"
		echo "Exec=${destFile}"
		echo "Icon=utilities-terminal"
		echo "Terminal=false"
		echo "Categories=System;TerminalEmulator;"
		echo "Keywords=terminal;shell;prompt;command;"
		echo "StartupNotify=true"
	} > "${tmpDir}/${exeName}.desktop"
	${sudoCmd} install -m 0644 "${tmpDir}/${exeName}.desktop" "${appDir}/${exeName}.desktop"

	echo "Installed ${tag} to ${destFile}"
	case ":${PATH}:" in
		*":${destDir}:"*) : ;;
		*) echo "Note: ${destDir} is not on your PATH - add it, or launch via the desktop menu." ;;
	esac
	echo
}


##	Script entry point
set -u -e -E -o pipefail
shopt -s inherit_errexit 2>/dev/null || true
if [[ "${BASH_SOURCE[0]:-}" == "${0}" || -z "${BASH_SOURCE[0]:-}" ]]; then
	fMain "$@"
fi
