# Shell integration

SilkTerm opens a new tab, split or window in the directory the current pane is in. To do that it has to know where that pane's shell is, and for most shells it can simply ask the operating system - it reads the shell process's own working directory, and nothing needs setting up.

Some shells never tell the operating system where they are. PowerShell is the one almost everybody meets: `Set-Location` moves PowerShell's own idea of where it is and leaves the process itself in the directory it was launched in, so there is nothing to read. The same applies to any session where the shell you are typing at is not the process SilkTerm started - an `ssh` session, a container, a REPL that keeps its own location.

The fix is the same one every terminal uses: have the shell say where it is, in a short escape sequence written at each prompt. SilkTerm listens for two spellings and takes either.

- **OSC 7**, a `file://` URL - what the unix shells emit, and what GNOME Terminal, WezTerm, kitty and others read.
- **OSC 9;9**, a plain path - the ConEmu spelling that Windows Terminal documents. A PowerShell profile already set up for Windows Terminal works here unchanged.

What the shell reports wins over what the operating system can see. A reported directory that no longer exists on this machine is ignored, and the operating system's answer stands instead - which is also what rejects a path reported from the far side of an `ssh`, since an OSC 7 URL naming another machine is never believed.

## PowerShell

Add this to your profile (`notepad $PROFILE`, creating it if it is not there):

```powershell
function prompt {
    $path = $ExecutionContext.SessionState.Path.CurrentLocation.ProviderPath
    $esc = [char]27
    Write-Host -NoNewline "$esc]9;9;`"$path`"$esc\"
    "PS $($ExecutionContext.SessionState.Path.CurrentLocation)$('>' * ($NestedPromptLevel + 1)) "
}
```

If you already have a `prompt` function, add the two `$esc` lines to it rather than replacing it. The sequence prints nothing on screen, and a terminal that does not understand it ignores it.

## bash

```bash
# ~/.bashrc
PROMPT_COMMAND='printf "\033]7;file://%s%s\033\\" "$HOSTNAME" "$PWD"'
```

Many distributions already do this for you - Debian and Fedora ship `/etc/profile.d/vte.sh`, which emits the same sequence. Check with `echo "$PROMPT_COMMAND"` before adding a second one.

Paths with characters that need escaping in a URL are supposed to be percent-encoded. SilkTerm reads them either way, so the one-liner above is enough here; encode if you also use a terminal that is stricter about it.

## zsh

```zsh
# ~/.zshrc
precmd() { printf "\033]7;file://%s%s\033\\" "$HOST" "$PWD" }
```

## fish

Nothing to do - fish emits OSC 7 on its own.

## cmd.exe

Nothing to do - `cd` moves the process itself, so the operating system can answer.

## Checking it works

`cd` somewhere, then split the pane (or open a new tab). The new pane should start in the same directory. If it starts where the old pane was *launched* instead, the sequence is not reaching us - check that the prompt function is actually running (`prompt` in PowerShell prints its own definition) and that nothing later in your profile overwrites it.
