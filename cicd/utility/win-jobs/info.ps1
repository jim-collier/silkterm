$ErrorActionPreference = "Continue"
. "$PSScriptRoot\_env.ps1"

"host        = $env:COMPUTERNAME"
"windows     = " + [System.Environment]::OSVersion.Version.ToString()
"cores       = $env:NUMBER_OF_PROCESSORS"
$c = Get-CimInstance Win32_ComputerSystem
"memory_gb   = " + [math]::Round($c.TotalPhysicalMemory / 1GB)
foreach ($g in Get-CimInstance Win32_VideoController) {
	"gpu         = $($g.Name) [$($g.Status)] drv $($g.DriverVersion)"
}
"sandbox     = " + (Get-WindowsOptionalFeature -Online -FeatureName Containers-DisposableClientVM).State
foreach ($t in "git","cargo","rustc","pwsh","makensis") {
	$cmd = Get-Command $t -ErrorAction SilentlyContinue
	if ($cmd) { "tool $t".PadRight(12) + "= " + $cmd.Source } else { "tool $t".PadRight(12) + "= absent" }
}
"repo        = " + (git -C $RepoDir rev-parse --short HEAD) + " on " + (git -C $RepoDir rev-parse --abbrev-ref HEAD)
""
(query session 2>&1 | Out-String).TrimEnd()
