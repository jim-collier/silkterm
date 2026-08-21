# Shell integration

SilkTerm opens a new tab, split or window in the directory the current pane is in. To do that it has to know where that pane's shell is, and for most shells it can simply ask the operating system - it reads the shell process's own working directory, and nothing needs setting up.

Some shells never tell the operating system where they are. PowerShell is the one almost everybody meets: `Set-Location` moves PowerShell's own idea of where it is and leaves the process itself in the directory it was launched in, so there is nothing to read. The same applies to any session where the shell you are typing at is not the process SilkTerm started - an `ssh` session, a container, a REPL that keeps its own location.

The fix is the same one every terminal uses: have the shell say where it is, in a short escape sequence. SilkTerm listens for two spellings and takes either.

- **OSC 7**, a `file://` URL - what the unix shells emit, and what GNOME Terminal, WezTerm, kitty and others read.
- **OSC 9;9**, a plain path - the ConEmu spelling that Windows Terminal documents. A PowerShell profile already set up for Windows Terminal works here unchanged.

What the shell reports wins over what the operating system can see. A reported directory that no longer exists on this machine is ignored, and the operating system's answer stands instead - which is also what rejects a path reported from the far side of an `ssh`, since an OSC 7 URL naming another machine is never believed.

## PowerShell: SilkTerm does this for you

A few seconds after launch, SilkTerm looks for the PowerShells you have installed and adds the block below to each one's profile. You do not have to do anything, and it happens once.

What it will not do:

- **Touch a profile that already reports.** Its own marker, or any other OSC 7 / OSC 9;9 already in the file - a Windows Terminal setup, oh-my-posh, anything - means somebody has this in hand, and the file is left exactly as it is.
- **Rewrite what is there.** The block is appended, after a copy of the profile is saved beside it as `Microsoft.PowerShell_profile.ps1.silkterm-backup`.
- **Put it back.** Deleting the block is how you switch it off. Nothing restores it.
- **Change your prompt.** On PowerShell 6+ the prompt is not touched at all; on Windows PowerShell 5.1, which has no other hook, whatever prompt is in place is wrapped rather than replaced.

- **Write a file the shell would refuse to read.** If PowerShell's execution policy blocks script files, the block would only turn every launch into a red execution-policy error, so the profile is left alone and a line says which shell and why. Windows PowerShell 5.1 is commonly in that state; `Get-ExecutionPolicy` shows it, and `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` is the usual fix - your call to make, not SilkTerm's.

To switch the whole thing off before it ever runs, set `shell.integration: false` in the config, or clear "PowerShell integration" on the Shell tab of Settings.

If you would rather relax the policy than change it, the Tabs menu offers **Windows PowerShell 5 (relaxed)** - the same shell launched with `-ExecutionPolicy RemoteSigned`, which applies to that session and writes nothing anywhere. It ships switched off; enable it on the Shell tab of Settings. Note that the flag is inherited by anything that session starts, so it relaxes the policy for the whole pane, not just the profile.

This is the block, if you would rather paste it in yourself (`notepad $PROFILE`, creating the file if it is not there):

```powershell
# >>> SilkTerm shell integration >>>
# Reports this shell's directory to the terminal, so a new tab, pane or window
# opens where this shell is. PowerShell keeps its location to itself, so there
# is nothing for a terminal to read unless the shell says so. Nothing is drawn
# on screen, and a terminal that does not understand the sequence ignores it.
# Delete this block to switch it off - SilkTerm will not put it back.
if ($Host.Name -eq 'ConsoleHost' -and -not [Console]::IsOutputRedirected) {
	function global:__SilkTermReportDir {
		$dir = $ExecutionContext.SessionState.Path.CurrentLocation.ProviderPath
		if ($dir) { Write-Host -NoNewline ("{0}]9;9;`"{1}`"{0}\" -f [char]27, $dir) }
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
	}
	else {
		# Windows PowerShell 5.1 has no such hook, so wrap whatever prompt is in
		# place rather than replacing it.
		$global:__SilkTermPrevPrompt = $function:prompt
		function global:prompt {
			__SilkTermReportDir
			if ($global:__SilkTermPrevPrompt) { & $global:__SilkTermPrevPrompt }
			else { "PS $($ExecutionContext.SessionState.Path.CurrentLocation)$('>' * ($NestedPromptLevel + 1)) " }
		}
	}
	__SilkTermReportDir
}
# <<< SilkTerm shell integration <<<
```

The guard on the first line is what keeps it out of your way: it reports only in an interactive console session whose output is not redirected, so `pwsh -File build.ps1 > log.txt` gets a clean log rather than escape sequences in it.

## bash

Nothing to set up - bash moves its own process, so SilkTerm can ask the operating system. Add this only if you want a pane behind `ssh`, `sudo -i` or a container to report as well:

```bash
# ~/.bashrc
PROMPT_COMMAND='printf "\033]7;file://%s%s\033\\" "$HOSTNAME" "$PWD"'
```

Many distributions already do this for you - Debian and Fedora ship `/etc/profile.d/vte.sh`, which emits the same sequence. Check with `echo "$PROMPT_COMMAND"` before adding a second one.

Paths with characters that need escaping in a URL are supposed to be percent-encoded. SilkTerm reads them either way, so the one-liner above is enough here; encode if you also use a terminal that is stricter about it.

## zsh

Same story as bash - nothing needed locally:

```zsh
# ~/.zshrc
precmd() { printf "\033]7;file://%s%s\033\\" "$HOST" "$PWD" }
```

## fish

Nothing to do - fish emits OSC 7 on its own.

## cmd.exe

Nothing to do - `cd` moves the process itself, so the operating system can answer.

## Checking it works

`cd` somewhere, then split the pane (or open a new tab). The new pane should start in the same directory. If it starts where the old pane was *launched* instead, the sequence is not reaching us - check that the block is in the profile that shell actually loads (`$PROFILE` prints its path), and that nothing later in the file replaces the prompt on 5.1.
