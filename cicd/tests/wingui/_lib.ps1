##	Helpers for a scenario running inside the console session. Dot-sourced by
##	_run.ps1, which has already checked that there is a desktop to draw on.

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace Silk -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
[DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint f, bool i, uint a);
[DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr h);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool GetUserObjectInformation(IntPtr h, int i, System.Text.StringBuilder p, int n, out uint need);
[DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
public struct RECT { public int Left, Top, Right, Bottom; }
'@

##	MainWindowHandle answers with whatever the process registered first, which for
##	this app is a 16x16 helper window - so the real one has to be picked out by
##	size from everything the process owns.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class SilkEnum {
	delegate bool EnumProc(IntPtr h, IntPtr p);
	[DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
	[DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
	[DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
	[DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out R r);
	struct R { public int L, T, Rt, B; }
	public static IntPtr Largest(uint want) {
		IntPtr best = IntPtr.Zero; long area = 0;
		EnumWindows((h, _) => {
			uint pid; GetWindowThreadProcessId(h, out pid);
			if (pid != want || !IsWindowVisible(h)) return true;
			R r; if (!GetWindowRect(h, out r)) return true;
			long a = (long)(r.Rt - r.L) * (r.B - r.T);
			if (a > area) { area = a; best = h; }
			return true;
		}, IntPtr.Zero);
		return best;
	}
}
'@

$script:checks   = @()
$script:failures = 0
$script:shotDir  = $null

function fCheck($what, $ok) {
	if ($ok) { $script:checks += "  ok   $what" }
	else     { $script:checks += "  FAIL $what"; $script:failures++ }
	$ok
}

function fNote($text) { $script:checks += "  note $text" }

##	Whether anyone could actually see or type into this desktop. A locked session
##	still runs windows and still answers PrintWindow, but screen grabs come back
##	black and injected input goes to the lock screen - so a scenario that needs
##	either must stop rather than quietly measure nothing.
function fSessionUsable {
	if (Get-Process LogonUI -ErrorAction SilentlyContinue) { return $false }
	$d = [Silk.Win]::OpenInputDesktop(0, $false, 0x0100)
	if ($d -eq [IntPtr]::Zero) { return $false }
	$sb = New-Object System.Text.StringBuilder 256
	$n = 0
	$got = [Silk.Win]::GetUserObjectInformation($d, 2, $sb, 256, [ref]$n)
	[void][Silk.Win]::CloseDesktop($d)
	$got -and $sb.ToString() -eq "Default"
}

function fStartSilk($exe, $silkArgs, $envVars) {
	foreach ($k in $envVars.Keys) { [Environment]::SetEnvironmentVariable($k, $envVars[$k]) }
	$p = Start-Process $exe -ArgumentList $silkArgs -PassThru
	foreach ($k in $envVars.Keys) { [Environment]::SetEnvironmentVariable($k, $null) }
	$p
}

function fWaitWindow($p, $seconds = 30) {
	for ($i = 0; $i -lt ($seconds * 4); $i++) {
		if ($p.HasExited) { return [IntPtr]::Zero }
		$h = [SilkEnum]::Largest([uint32]$p.Id)
		if ($h -ne [IntPtr]::Zero) {
			$r = fRect $h
			if ($r.w -gt 200 -and $r.h -gt 100) { return $h }
		}
		Start-Sleep -Milliseconds 250
	}
	[IntPtr]::Zero
}

function fFocus($h) {
	[void][Silk.Win]::SetForegroundWindow($h)
	Start-Sleep -Milliseconds 350
	[Silk.Win]::GetForegroundWindow() -eq $h
}

function fType($text) { [System.Windows.Forms.SendKeys]::SendWait($text); Start-Sleep -Milliseconds 250 }

function fRect($h) {
	$r = New-Object Silk.Win+RECT
	[void][Silk.Win]::GetWindowRect($h, [ref]$r)
	@{ x = $r.Left; y = $r.Top; w = $r.Right - $r.Left; h = $r.Bottom - $r.Top }
}

##	A screen grab is the faithful picture - it is what the compositor put up, and
##	it is the only one that sees a window drawn without a redirection bitmap (the
##	transparent path). PrintWindow is the fallback for a session nobody can see.
function fShot($h, $name) {
	$r = fRect $h
	if ($r.w -le 0 -or $r.h -le 0) { return $null }
	$bmp = New-Object System.Drawing.Bitmap $r.w, $r.h
	$g = [System.Drawing.Graphics]::FromImage($bmp)
	$how = "screen"
	if (fSessionUsable) {
		$g.CopyFromScreen($r.x, $r.y, 0, 0, (New-Object System.Drawing.Size $r.w, $r.h))
	} else {
		$dc = $g.GetHdc()
		[void][Silk.Win]::PrintWindow($h, $dc, 2)
		$g.ReleaseHdc($dc)
		$how = "printwindow"
	}
	if ($script:shotDir) {
		New-Item -ItemType Directory -Force -Path $script:shotDir | Out-Null
		$bmp.Save((Join-Path $script:shotDir "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
	}
	$bmp | Add-Member -NotePropertyName How -NotePropertyValue $how -PassThru
}

##	Fraction of sampled pixels carrying any light. A window that came up but never
##	drew reads near zero, which is the difference between a real capture and the
##	black rectangle a locked session hands back.
function fInk($bmp, $step = 4) {
	if (-not $bmp) { return 0.0 }
	$lit = 0; $seen = 0
	for ($y = 0; $y -lt $bmp.Height; $y += $step) {
		for ($x = 0; $x -lt $bmp.Width; $x += $step) {
			$seen++
			$px = $bmp.GetPixel($x, $y)
			if ($px.R + $px.G + $px.B -gt 40) { $lit++ }
		}
	}
	if ($seen -eq 0) { 0.0 } else { [math]::Round($lit / $seen, 4) }
}

##	How much of the picture moved. Ink saturates on a wallpaper, so 'did anything
##	happen' has to be asked as a difference rather than a brightness.
function fDiff($a, $b, $step = 3) {
	if (-not $a -or -not $b) { return 1.0 }
	if ($a.Width -ne $b.Width -or $a.Height -ne $b.Height) { return 1.0 }
	$moved = 0; $seen = 0
	for ($y = 0; $y -lt $a.Height; $y += $step) {
		for ($x = 0; $x -lt $a.Width; $x += $step) {
			$seen++
			$p1 = $a.GetPixel($x, $y); $p2 = $b.GetPixel($x, $y)
			if ([math]::Abs($p1.R - $p2.R) + [math]::Abs($p1.G - $p2.G) + [math]::Abs($p1.B - $p2.B) -gt 24) { $moved++ }
		}
	}
	if ($seen -eq 0) { 0.0 } else { [math]::Round($moved / $seen, 4) }
}

function fStop($p) {
	if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
}
