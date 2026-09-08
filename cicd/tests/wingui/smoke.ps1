##	Proves the rig itself: a window comes up on the real adapter, it draws, it takes
##	the foreground, and what is typed reaches the shell inside it.

if (-not (fSessionUsable)) { fSkip "console session is locked - nothing can be typed or grabbed" }

$cfg = Join-Path $OutDir "smoke-config.shcl"
Remove-Item $cfg -ErrorAction SilentlyContinue

$p = fStartSilk $Exe @("--config=$cfg", "--columns", "100", "--rows", "30") @{}
$h = fWaitWindow $p 40
if (-not (fCheck "a window came up" ($h -ne [IntPtr]::Zero))) { fStop $p; return }

Start-Sleep -Seconds 3
$r = fRect $h
fNote "window $($r.w)x$($r.h) at $($r.x),$($r.y)"
$idle = fShot $h "smoke-idle"
fNote "capture via $($idle.How), ink $(fInk $idle)"
[void](fCheck "the window is a real size" ($r.w -gt 400 -and $r.h -gt 200))
[void](fCheck "the window actually drew" ((fInk $idle) -gt 0.02))

[void](fCheck "it takes the foreground" (fFocus $h))

##	Type something the shell echoes, then look for the picture to have changed.
$before = fShot $h "smoke-before"
fType "echo silkrig-was-here{ENTER}"
Start-Sleep -Seconds 2
$after = fShot $h "smoke-after"
$moved = fDiff $before $after
fNote "picture moved $moved"
[void](fCheck "typing reached the shell" ($moved -gt 0.005))

fStop $p
