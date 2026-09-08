##	The Settings dialog is a second window with its own GPU context, and winit's
##	parenting on Windows makes it a child rather than an owned window - so nothing
##	about it follows from the main window working.

if (-not (fSessionUsable)) { fSkip "console session is locked - the dialog cannot be grabbed" }

$cfg = Join-Path $OutDir "dlg-config.shcl"
fFreshConfig $cfg

$p = fStartSilk $Exe @("--config=$cfg", "--columns", "110", "--rows", "32") @{}
$h = fWaitWindow $p 40
if (-not (fCheck "the terminal came up" ($h -ne [IntPtr]::Zero))) { fStop $p; return }
[void](fCheck "the terminal takes the foreground" (fFocus $h))
Start-Sleep -Seconds 4

fPress "ctrl+,"
$d = fWaitOther $p $h 25
if (-not (fCheck "ctrl+comma opens the dialog" ($d -ne [IntPtr]::Zero))) { [void](fShot $h "dlg-none"); fStop $p; return }

Start-Sleep -Seconds 2
$r = fRect $d
fNote "dialog $($r.w)x$($r.h) at $($r.x),$($r.y)"
$first = fShot $d "dlg-tab-first"
fNote "capture via $($first.How), ink $(fInk $first)"
[void](fCheck "the dialog drew something" ((fInk $first) -gt 0.05))

##	A child window is clipped to its parent. The dialog is taller than it is wide,
##	so a clipped one shows up as a short window rather than a missing one.
[void](fCheck "the dialog is not clipped to its parent" ($r.h -gt $r.w))

[void](fFocus $d)
fPress "ctrl+tab"
Start-Sleep -Seconds 1
$second = fShot $d "dlg-tab-second"
fNote "the panel changed across a tab by $(fDiff $first $second)"
[void](fCheck "ctrl+tab moves to another tab" ((fDiff $first $second) -gt 0.02))

fPress "escape"
Start-Sleep -Seconds 2
[void](fCheck "escape closes the dialog" (-not ([SilkEnum]::All([uint32]$p.Id) -contains $d)))
fStop $p
