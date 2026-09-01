# >>> SilkTerm shell integration >>>
# Reports this shell's directory to the terminal, so a new tab, pane or window
# opens where this shell is. PowerShell keeps its location to itself, so there
# is nothing for a terminal to read unless the shell says so. Nothing is drawn
# on screen, and a terminal that does not understand the sequence ignores it.
# It also sets a git-aware prompt - but only when the prompt is still the stock
# one, so your own prompt is never replaced.
# Delete this block to switch it off - SilkTerm will not put it back. It does
# keep the block itself up to date, so an edit made INSIDE the two markers is
# replaced on a later launch - copy it out below them to make it yours.
if ($Host.Name -eq 'ConsoleHost' -and -not [Console]::IsOutputRedirected) {
	function global:__SilkTermReportDir {
		$dir = $ExecutionContext.SessionState.Path.CurrentLocation.ProviderPath
		if ($dir) { Write-Host -NoNewline ("{0}]9;9;`"{1}`"{0}\" -f [char]27, $dir) }
	}
	# Is this still the prompt PowerShell ships? Its own help link is the marker:
	# a prompt anybody has written, or that oh-my-posh or starship installed,
	# will not carry it. Only the stock one is replaced.
	$__SilkTermStock = (-not $function:prompt) -or ($function:prompt.ToString() -match 'LinkID=225750')

	# Everything below builds the prompt. Worked out once, at load: none of it
	# changes while the session runs, and the prompt is drawn after every command.
	$global:__SilkTermHasGit = [bool](Get-Command git -CommandType Application -ErrorAction SilentlyContinue)
	$global:__SilkTermRemotes = @{}
	# The console goes to UTF-8 so that a branch name with a non-ASCII character
	# in it decodes, and so nothing downstream has to guess. The prompt itself
	# is written as wide characters and reaches the screen either way.
	try { [Console]::OutputEncoding = New-Object Text.UTF8Encoding $false } catch { }
	# Code points rather than literal glyphs, because 5.1 reads a file with no
	# byte-order mark as ANSI.
	$global:__SilkTermGlyphs = @{ Yes = [string][char]0x2713; No = [string][char]0x2718 }
	# Root gets a different decorator, the way a unix prompt does.
	$global:__SilkTermAdmin = $false
	try {
		if ($PSVersionTable.PSVersion.Major -lt 6 -or $IsWindows) {
			$id = [Security.Principal.WindowsIdentity]::GetCurrent()
			$global:__SilkTermAdmin = (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
		}
		else { $global:__SilkTermAdmin = [Environment]::UserName -eq 'root' }
	}
	catch { }
	# A machine you are logged into by mistake should look wrong immediately, so
	# the host name is colored per machine. Add your own; anything not listed
	# gets the default.
	$global:__SilkTermHostColor = switch ([Environment]::MachineName.ToLowerInvariant()) {
		'b12' { '1;32' }
		'b15' { '1;34' }
		'b16' { '1;31' }
		'b17' { '1;35' }
		'b23' { '1;31' }
		'vm925w' { '1;36' }
		'xub2004a' { '1;32' }
		't2nsn' { '1;35' }
		default { '1;37' }
	}

	function global:__SilkTermPaint {
		param([string]$Code, [string]$Text)
		"$([char]27)[${Code}m$Text$([char]27)[0m"
	}

	# The working tree this directory is in, or $null. Walking up for a .git
	# entry costs a few file tests; asking git costs a process, on every prompt,
	# in every directory that is not a repository.
	function global:__SilkTermRepoRoot {
		param([string]$Start)
		$dir = $Start
		while ($dir) {
			if (Test-Path -LiteralPath (Join-Path $dir '.git')) { return $dir }
			$parent = Split-Path -Parent $dir
			if (-not $parent -or $parent -eq $dir) { return $null }
			$dir = $parent
		}
		return $null
	}

	# Branch, clean, and in step with the upstream, from one call. The v2 format
	# answers all three: the branch header lines start with #, and any other line
	# is a change - a modified file, or an untracked one.
	function global:__SilkTermGitState {
		param([string]$Root)
		$prev = $ErrorActionPreference
		$ErrorActionPreference = 'Continue'
		$lines = & git status --porcelain=v2 --branch 2>$null
		$ErrorActionPreference = $prev
		if (-not $lines) { return $null }
		$branch = ''
		$ahead = $null
		$clean = $true
		foreach ($line in $lines) {
			if ($line.StartsWith('# branch.head ')) { $branch = ($line -split ' ', 3)[2] }
			elseif ($line.StartsWith('# branch.ab ')) { $ahead = ($line -split ' ', 3)[2] }
			elseif (-not $line.StartsWith('#')) { $clean = $false }
		}
		if (-not $global:__SilkTermRemotes.ContainsKey($Root)) {
			$prev = $ErrorActionPreference
			$ErrorActionPreference = 'Continue'
			$url = (& git config --get remote.origin.url 2>$null) -as [string]
			$ErrorActionPreference = $prev
			if ($url) {
				# ssh remotes read git@host:owner/repo - the part before the @ is
				# the same on every line and says nothing.
				$at = $url.IndexOf('@')
				if ($at -ge 0) { $url = $url.Substring($at + 1) }
			}
			$global:__SilkTermRemotes[$Root] = $url
		}
		@{ Branch = $branch; Clean = $clean; Synced = ($ahead -eq '+0 -0'); Remote = $global:__SilkTermRemotes[$Root] }
	}

	function global:__SilkTermPrompt {
		$dec = if ($global:__SilkTermAdmin) { '#' } else { '>' }
		$dec = $dec * ($NestedPromptLevel + 1)
		$dir = "$($ExecutionContext.SessionState.Path.CurrentLocation)"
		# An ordinary prompt, for anyone who wants one back for a session.
		if ($env:X9PS1_STANDARD -eq '1') { return "PS $dir$dec " }
		$home_ = $HOME
		if ($home_ -and $dir.StartsWith($home_, [StringComparison]::OrdinalIgnoreCase)) {
			$dir = '~' + $dir.Substring($home_.Length)
		}
		$v = $PSVersionTable.PSVersion
		# The version, because two PowerShells look alike at a prompt.
		$out = __SilkTermPaint '2;37' "[PS $($v.Major).$($v.Minor)]"
		$out += ' ' + (__SilkTermPaint '2;36' (Get-Date -Format 'HH:mm:ss'))
		$out += ' ' + (__SilkTermPaint '0;32' ([Environment]::UserName))
		$out += (__SilkTermPaint '2;37' '@')
		$out += (__SilkTermPaint $global:__SilkTermHostColor ([Environment]::MachineName))
		$out += (__SilkTermPaint '2;37' ':')
		$out += (__SilkTermPaint '0;37' $dir)
		$git = $null
		if ($global:__SilkTermHasGit -and $PWD.Provider.Name -eq 'FileSystem') {
			$root = __SilkTermRepoRoot $PWD.ProviderPath
			if ($root) { $git = __SilkTermGitState $root }
		}
		if ($git) {
			$mark = { param($ok) __SilkTermPaint $(if ($ok) { '7;32' } else { '7;31' }) $(if ($ok) { $global:__SilkTermGlyphs.Yes } else { $global:__SilkTermGlyphs.No }) }
			$out += ' ' + (__SilkTermPaint '2;37' '[') + ' '
			if ($git.Remote) { $out += (__SilkTermPaint '0;35' $git.Remote) + (__SilkTermPaint '2;37' ':') }
			$out += (__SilkTermPaint '1;36' $git.Branch) + ' '
			# Two marks: everything committed, and level with the upstream.
			$out += (& $mark $git.Clean) + (& $mark $git.Synced)
			$out += ' ' + (__SilkTermPaint '2;37' ']')
			# The first line is long in a repository, so the typing starts on its own.
			$out += "`n"
			$sep = ''
		}
		else { $sep = ' ' }
		$out + $sep + (__SilkTermPaint '2;37' $dec) + ' '
	}
	if ($null -ne $ExecutionContext.SessionState.InvokeCommand.PSObject.Properties['LocationChangedAction']) {
		# PowerShell 6+ can be told about the location itself, which leaves the
		# prompt alone - oh-my-posh, starship and a hand-written prompt all keep
		# working, and anything already using this hook is called first.
		$global:__SilkTermPrevLocation = $ExecutionContext.SessionState.InvokeCommand.LocationChangedAction
		$ExecutionContext.SessionState.InvokeCommand.LocationChangedAction = {
			if ($global:__SilkTermPrevLocation) { & $global:__SilkTermPrevLocation @args }
			__SilkTermReportDir
		}
		if ($__SilkTermStock) { function global:prompt { __SilkTermPrompt } }
	}
	else {
		# Windows PowerShell 5.1 has no such hook, so wrap whatever prompt is in
		# place rather than replacing it.
		$global:__SilkTermPrevPrompt = if ($__SilkTermStock) { $null } else { $function:prompt }
		function global:prompt {
			__SilkTermReportDir
			if ($global:__SilkTermPrevPrompt) { & $global:__SilkTermPrevPrompt }
			else { __SilkTermPrompt }
		}
	}
	__SilkTermReportDir
}
# <<< SilkTerm shell integration <<<
