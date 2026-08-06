#!/usr/bin/env bash
# shellcheck disable=SC2317  ## unreachable-after-exit false positives in dispatchers

##	- Purpose: One-liner installer for a single-binary GitHub release. Detects the
##	  OS and CPU, works out which release asset that is, verifies its sha256
##	  against the release's checksums file, and installs it. Idempotent: states
##	  its plan, asks before touching anything, and does nothing when the
##	  installed binary is already current.
##	- Reusable: everything project-specific lives in the settings block below.
##	- Syntax:
##	  bash <(curl -fsSL https://raw.githubusercontent.com/jim-collier/silkterm/main/install.bash) [options]
##	- Options: --release stable|dev, --target user|system, --yes, --version, --help.
##	  The OS, the CPU architecture and the asset name are all detected.
##	- Needs: bash >= 3.2 (the macOS system bash), curl or wget, and one of
##	  sha256sum / shasum / openssl.
##	- History:
##	  - 20260723 JC: Created.
##	  - 20260806 JC: Made project-agnostic; dropped --arch for autodetection;
##	                 added --version; targets bash 3.2.

##	Copyright © 2026 Jim Collier (CryptogID: ѳ6ᴚ℈𐀘𐇦ɛ𐊁¥Mﾏb϶Δ𐌞)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


##	•••••••••••••••••••  Per-project settings - edit only these  ••••••••••••••••••

installerVersion="1.1.0"
ownerRepo="jim-collier/silkterm"
appName="SilkTerm"
exeName="silkterm"
appComment="Smooth-scrolling GPU terminal with split panes"

##	Release asset names. {exe} {version} {os} {arch} {ext} are substituted; {ext}
##	is ".exe" on Windows and empty elsewhere. {os} is linux/macos/freebsd/windows,
##	{arch} is x86_64/arm64 - match whatever the release actually publishes.
assetPattern="{exe}-{version}-{os}-{arch}{ext}"
sumsPattern="{exe}-{version}-sha256sums.txt"

##	Freedesktop launcher (Linux only). Set to 0 for a non-GUI program.
desktopEntry=1
desktopGenericName="Terminal"
desktopIcon="utilities-terminal"
desktopCategories="System;TerminalEmulator;"
desktopKeywords="terminal;shell;prompt;command;"

##	••••••••••••••••••••••••  End per-project settings  ••••••••••••••••••••••••••

apiBase="https://api.github.com/repos/${ownerRepo}"
dlBase="https://github.com/${ownerRepo}/releases/download"
rawBase="https://raw.githubusercontent.com/${ownerRepo}/main"


##	Output helpers

##	fFail "message" ["hint" ...] - one error line, then any hints, then exit.
function fFail() {
	local first="$1"; shift
	echo "Error: ${first}" >&2
	while [ "$#" -gt 0 ]; do echo "  ${1}" >&2; shift; done
	echo >&2
	exit 1
}

function fHelp() {
	cat <<EOF
${appName} installer ${installerVersion}

Downloads the newest ${appName} release from GitHub, checks its sha256, and
installs it. It prints what it is about to do and asks first, and it does
nothing at all when the installed copy is already current.

Usage:
  bash <(curl -fsSL ${rawBase}/install.bash) [options]

Options:
  --release stable|dev   stable (default): newest full release
                         dev:              newest release, pre-releases included
  --target  user|system  user (default):   \$HOME (no root needed)
                         system:           /usr/local (uses sudo)
  --yes, -y              skip the confirmation prompt
  --version              print this installer's version and exit
  --help, -h             this text

The operating system, the CPU architecture and the matching release asset are
all detected - there is nothing to pass for them.

EOF
}

##	Lowercase, the 3.2 way - \${x,,} is bash 4.
function fLower() { echo "$1" | tr '[:upper:]' '[:lower:]'; }


##	Network + hashing (curl preferred, wget accepted so minimal images work)

##	fGet <url> <outfile> - quiet fetch. Returns non-zero on any HTTP or
##	transport error rather than writing a "404: not found" page to the file.
function fGet() {
	local url="$1" out="$2"
	if [ "${dlTool}" = "curl" ]; then
		curl -fsSL -o "${out}" "${url}"
	else
		wget -qO "${out}" "${url}"
	fi
}

##	fApi <url> - same, to stdout, carrying the optional token. Only the API is
##	rate-limited per IP; release downloads are not, so they stay anonymous.
function fApi() {
	local url="$1"
	if [ "${dlTool}" = "curl" ]; then
		if [ -n "${apiToken}" ]; then
			curl -fsSL -H "Authorization: Bearer ${apiToken}" "${url}"
		else
			curl -fsSL "${url}"
		fi
	else
		if [ -n "${apiToken}" ]; then
			wget -qO- --header="Authorization: Bearer ${apiToken}" "${url}"
		else
			wget -qO- "${url}"
		fi
	fi
}

##	Same, but shows progress - the release binary is the only big download.
function fGetShown() {
	local url="$1" out="$2"
	if [ "${dlTool}" = "curl" ]; then
		curl -fSL --progress-bar -o "${out}" "${url}"
	else
		wget -q --show-progress -O "${out}" "${url}"
	fi
}

function fSha256() {
	if [ -n "${shaTool}" ]; then
		case "${shaTool}" in
			sha256sum) sha256sum "$1" | awk '{print $1}' ;;
			shasum)    shasum -a 256 "$1" | awk '{print $1}' ;;
			openssl)   openssl dgst -sha256 "$1" | awk '{print $NF}' ;;
		esac
	fi
}

##	First "tag_name" in a GitHub API body, without depending on jq.
function fFirstTag() {
	sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

##	True when the deepest existing parent of $1 is writable by us.
function fCanWrite() {
	local dir="$1"
	while [ -n "${dir}" ] && [ "${dir}" != "/" ] && [ ! -e "${dir}" ]; do
		dir="$(dirname "${dir}")"
	done
	[ -w "${dir}" ]
}

##	Scratch space. Global on purpose: the EXIT trap fires after fMain has
##	returned, so a local would be out of scope by then (and `set -u` turns that
##	into a failed exit status on an otherwise perfect install).
tmpDir=""
function fCleanup() { [ -z "${tmpDir}" ] || rm -rf "${tmpDir}"; }

##	0 = yes, 1 = no, 2 = could not ask at all.
##	The terminal comes FIRST because the `curl ... | bash` form leaves the script
##	itself sitting on stdin - reading the answer from there would eat the script.
##	Falling back to stdin is what keeps a piped `echo y | ...` working.
function fConfirm() {
	local answer=""
	printf "%s [y/N]: " "$1"
	if { : </dev/tty; } 2>/dev/null; then
		read -r answer </dev/tty || { echo; return 2; }
	else
		read -r answer || { echo; return 2; }
	fi
	case "${answer}" in y|Y|yes|Yes|YES) return 0 ;; *) return 1 ;; esac
}


function fMain() {

	echo

	##	Detect the platform first so --help can describe it, but defer any
	##	"unsupported" failure until after --help/--version have had their say.
	local osName osToken="" archToken="" exeExt="" osProblem=""
	osName="$(uname -s 2>/dev/null || echo unknown)"
	case "$(fLower "${osName}")" in
		linux)                       osToken="linux" ;;
		darwin)                      osToken="macos" ;;
		freebsd)                     osToken="freebsd" ;;
		openbsd)                     osToken="openbsd" ;;
		netbsd)                      osToken="netbsd" ;;
		dragonfly)                   osToken="dragonfly" ;;
		mingw*|msys*|cygwin*)        osToken="windows"; exeExt=".exe" ;;
		*)                           osProblem="unrecognized operating system: ${osName}" ;;
	esac
	case "$(fLower "$(uname -m 2>/dev/null || echo unknown)")" in
		x86_64|amd64|x64)            archToken="x86_64" ;;
		aarch64|arm64)               archToken="arm64" ;;
		i386|i486|i586|i686)         osProblem="32-bit x86 is not supported" ;;
		arm*)                        osProblem="32-bit ARM is not supported" ;;
		*)                           osProblem="unrecognized CPU architecture: $(uname -m 2>/dev/null)" ;;
	esac

	##	Parse arguments
	local release="stable" target="user" assumeYes=0
	while [ "$#" -gt 0 ]; do case "$1" in
		--release)   release="${2:-}"; shift 2 ;;
		--release=*) release="${1#*=}"; shift ;;
		--target)    target="${2:-}"; shift 2 ;;
		--target=*)  target="${1#*=}"; shift ;;
		--yes|-y)    assumeYes=1; shift ;;
		--version)   echo "${appName} installer ${installerVersion}"; echo; exit 0 ;;
		-h|--help)   fHelp; exit 0 ;;
		*)           fFail "unknown option: $1" "Run with --help to see the options." ;;
	esac; done
	case "${release}" in stable|dev) : ;; *) fFail "--release must be stable or dev (got '${release}')" ;; esac
	case "${target}" in user|system) : ;; *) fFail "--target must be user or system (got '${target}')" ;; esac

	[ -z "${osProblem}" ] || fFail "${osProblem}" \
		"No ${appName} build is published for this platform." \
		"Building from source: https://github.com/${ownerRepo}#build-it-yourself"
	if [ "${osToken}" = "windows" ]; then
		fFail "this is the Windows shell environment (${osName})" \
			"Use the PowerShell installer instead - it also sets up the Start Menu entry and PATH:" \
			"  irm ${rawBase}/install.ps1 | iex"
	fi

	##	Tools. curl/wget only need to exist for one of them; a hash tool is
	##	non-negotiable, since an unverified binary must never be installed.
	local dlTool="" shaTool=""
	if command -v curl >/dev/null 2>&1; then dlTool="curl"
	elif command -v wget >/dev/null 2>&1; then dlTool="wget"
	else fFail "neither curl nor wget is installed" "Install one of them and re-run (for example: sudo apt install curl)."
	fi
	if command -v sha256sum >/dev/null 2>&1; then shaTool="sha256sum"
	elif command -v shasum >/dev/null 2>&1; then shaTool="shasum"
	elif command -v openssl >/dev/null 2>&1; then shaTool="openssl"
	else fFail "no sha256 tool found (looked for sha256sum, shasum, openssl)" "The download can't be verified without one, so nothing will be installed."
	fi

	##	An API token is optional, and only lifts the unauthenticated rate limit.
	local apiToken="${GITHUB_TOKEN:-}"

	##	Resolve the release tag. "latest" deliberately EXCLUDES pre-releases, so
	##	stable asks for it first and only then falls back to the newest of any
	##	kind - which is also what makes a project with only betas installable.
	echo "Looking up the newest ${release} release of ${appName} ..."
	local tag="" apiBody=""
	if [ "${release}" = "stable" ]; then
		apiBody="$(fApi "${apiBase}/releases/latest" 2>/dev/null)" || apiBody=""
		tag="$(echo "${apiBody}" | fFirstTag)"
		if [ -z "${tag}" ]; then
			echo "No full release published yet; using the newest pre-release instead."
			release="dev"
		fi
	fi
	if [ "${release}" = "dev" ] && [ -z "${tag}" ]; then
		apiBody="$(fApi "${apiBase}/releases?per_page=10" 2>/dev/null)" || apiBody=""
		tag="$(echo "${apiBody}" | fFirstTag)"
	fi
	if [ -z "${tag}" ]; then
		case "${apiBody}" in
			*"rate limit"*) fFail "GitHub's API rate limit is exhausted for this IP" \
				"Wait an hour, or set GITHUB_TOKEN to a personal access token and re-run." ;;
			*) fFail "could not find a release at github.com/${ownerRepo}" \
				"Either none is published yet (build it from source - see the README)," \
				"or github.com is unreachable from here (check your network or HTTPS_PROXY)." ;;
		esac
	fi
	local version="${tag#v}"

	##	Work out the asset name for this platform
	local asset sums
	asset="${assetPattern}"
	##	'|' as the delimiter, so a value carrying a '/' cannot break the script.
	asset="$(echo "${asset}" | sed -e "s|{exe}|${exeName}|g" -e "s|{version}|${version}|g" \
		-e "s|{os}|${osToken}|g" -e "s|{arch}|${archToken}|g" -e "s|{ext}|${exeExt}|g")"
	sums="$(echo "${sumsPattern}" | sed -e "s|{exe}|${exeName}|g" -e "s|{version}|${version}|g")"

	##	Pull the checksums first: it is small, it says which platforms this
	##	release actually carries, and its hash lets an already-current install
	##	finish without downloading the binary at all.
	tmpDir="$(mktemp -d 2>/dev/null)" || fFail "could not create a temporary directory"
	trap fCleanup EXIT
	fGet "${dlBase}/${tag}/${sums}" "${tmpDir}/${sums}" 2>/dev/null \
		|| fFail "release ${tag} has no checksums file (${sums})" \
			"Nothing can be verified without it, so nothing will be installed." \
			"Release page: https://github.com/${ownerRepo}/releases/tag/${tag}"

	local wantSha
	wantSha="$(awk -v want="${asset}" '{ name = $2; sub(/^\*/, "", name); if (name == want) { print $1; exit } }' "${tmpDir}/${sums}")"
	if [ -z "${wantSha}" ]; then
		echo >&2
		echo "Error: release ${tag} has no build for ${osToken}-${archToken}." >&2
		echo "  Expected asset: ${asset}" >&2
		echo "  What it does carry:" >&2
		awk '{ name = $2; sub(/^\*/, "", name); print "    " name }' "${tmpDir}/${sums}" >&2
		echo "  Building from source: https://github.com/${ownerRepo}#build-it-yourself" >&2
		echo >&2
		exit 1
	fi

	##	Destination
	local destDir destFile appDir="" sudoCmd=""
	if [ "${target}" = "user" ]; then
		destDir="${HOME}/.local/bin"
		appDir="${HOME}/.local/share/applications"
	else
		destDir="/usr/local/bin"
		appDir="/usr/local/share/applications"
		if [ "$(id -u)" != "0" ]; then
			command -v sudo >/dev/null 2>&1 \
				|| fFail "a system install needs root, and sudo is not installed" \
					"Re-run as root, or use --target user to install under \$HOME instead."
			sudoCmd="sudo"
		fi
	fi
	[ "${osToken}" = "linux" ] && [ "${desktopEntry}" = "1" ] || appDir=""
	destFile="${destDir}/${exeName}"

	##	Already current? Then say so and stop - no prompt, no download.
	if [ -f "${destFile}" ] && [ "$(fSha256 "${destFile}")" = "${wantSha}" ]; then
		echo
		echo "Already up to date: ${destFile} is ${tag}. Nothing to do."
		echo
		exit 0
	fi

	##	Catch a permission problem now, rather than after a 10MB download.
	if [ -z "${sudoCmd}" ] && ! fCanWrite "${destDir}"; then
		if [ "${target}" = "user" ]; then
			fFail "no permission to write to ${destDir}" \
				"Check who owns it: ls -ld ${destDir}"
		else
			fFail "no permission to write to ${destDir}" \
				"Re-run under sudo, or use --target user to install under \$HOME instead."
		fi
	fi

	##	The plan
	echo
	echo "Plan:"
	echo "  Program:  ${appName} ${tag} (${release})"
	echo "  Platform: ${osToken}-${archToken}"
	echo "  Download: ${dlBase}/${tag}/${asset}"
	echo "  Verify:   sha256 against ${sums}"
	echo "  Install:  ${destFile}"
	[ -z "${appDir}" ] || echo "  Launcher: ${appDir}/${exeName}.desktop"
	[ -z "${sudoCmd}" ] || echo "  Elevation: the install steps run under sudo"
	echo
	if [ "${assumeYes}" != "1" ]; then
		local answered=0
		fConfirm "Proceed?" || answered=$?
		if [ "${answered}" = "2" ]; then
			fFail "there is no terminal here to ask for confirmation" \
				"Re-run with --yes to install without being asked."
		fi
		if [ "${answered}" != "0" ]; then
			echo "Aborted - nothing was touched."; echo; exit 0
		fi
		echo
	fi

	##	Download + verify
	echo "Downloading ${asset} ..."
	fGetShown "${dlBase}/${tag}/${asset}" "${tmpDir}/${asset}" \
		|| fFail "download failed" \
			"The release lists this asset, so this is most likely a network problem." \
			"URL: ${dlBase}/${tag}/${asset}"
	local haveSha; haveSha="$(fSha256 "${tmpDir}/${asset}")"
	if [ "${haveSha}" != "${wantSha}" ]; then
		fFail "checksum mismatch - NOT installing" \
			"expected ${wantSha}" \
			"got      ${haveSha}" \
			"The download was corrupted or tampered with. Try again; if it repeats, report it."
	fi
	echo "Checksum OK."

	##	Install. Land beside the target and rename into place: a rename is atomic,
	##	and it replaces a binary that is currently RUNNING (a straight copy over
	##	one fails with ETXTBSY).
	echo
	echo "Installing ..."
	local staged="${destDir}/.${exeName}.new.$$"
	${sudoCmd} mkdir -p "${destDir}" || fFail "could not create ${destDir}"
	${sudoCmd} cp "${tmpDir}/${asset}" "${staged}" || fFail "could not write to ${destDir}"
	${sudoCmd} chmod 0755 "${staged}"
	${sudoCmd} mv -f "${staged}" "${destFile}" || {
		${sudoCmd} rm -f "${staged}"
		fFail "could not replace ${destFile}"
	}

	##	Desktop launcher
	if [ -n "${appDir}" ]; then
		{
			echo "[Desktop Entry]"
			echo "Type=Application"
			echo "Name=${appName}"
			echo "GenericName=${desktopGenericName}"
			echo "Comment=${appComment}"
			echo "Exec=${destFile}"
			echo "Icon=${desktopIcon}"
			echo "Terminal=false"
			echo "Categories=${desktopCategories}"
			echo "Keywords=${desktopKeywords}"
			echo "StartupNotify=true"
		} > "${tmpDir}/${exeName}.desktop"
		if ${sudoCmd} mkdir -p "${appDir}" 2>/dev/null \
			&& ${sudoCmd} cp "${tmpDir}/${exeName}.desktop" "${appDir}/${exeName}.desktop" 2>/dev/null; then
			${sudoCmd} chmod 0644 "${appDir}/${exeName}.desktop"
		else
			echo "Note: could not write the desktop launcher to ${appDir} (the program itself installed fine)."
		fi
	fi

	echo "Installed ${appName} ${tag} to ${destFile}"
	case ":${PATH}:" in
		*":${destDir}:"*) : ;;
		*)
			echo
			echo "Note: ${destDir} is not on your PATH, so '${exeName}' won't be found by name yet."
			echo "  Add it with:  echo 'export PATH=\"${destDir}:\$PATH\"' >> ~/.profile"
			echo "  Until then, run it in full:  ${destFile}"
			;;
	esac
	echo
}


##	Script entry point
set -u -e -E -o pipefail
shopt -s inherit_errexit 2>/dev/null || true
if [ "${BASH_SOURCE[0]:-}" = "${0}" ] || [ -z "${BASH_SOURCE[0]:-}" ]; then
	##	"$@" with no arguments is an unbound-variable error under `set -u` on
	##	bash 3.2 (the macOS system bash), so only pass it when there is one.
	if [ "$#" -gt 0 ]; then fMain "$@"; else fMain; fi
fi
