$ErrorActionPreference = 'Stop'
$Dir = 'C:\probe-std'
New-Item -ItemType Directory -Force $Dir | Out-Null
Copy-Item -Path (Join-Path $PSScriptRoot '*') -Destination $Dir -Force
& icacls.exe $Dir /grant '*S-1-5-32-545:(OI)(CI)M' | Out-Null

$Name = 'probestd'
$chars = [char[]]'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
$Password = (-join (1..20 | ForEach-Object { $chars | Get-Random })) + 'Aa1!'
Write-Host "::add-mask::$Password"
& net.exe user $Name $Password /add | Out-Null
if ($LASTEXITCODE -ne 0) { throw "net user /add failed with $LASTEXITCODE" }
Write-Host "Created local user $Name. Groups:"
& net.exe user $Name | Select-String 'Local Group'

$secure = ConvertTo-SecureString $Password -AsPlainText -Force
$cred = [pscredential]::new("$env:COMPUTERNAME\$Name", $secure)
$pwsh = (Get-Process -Id $PID).Path
$argList = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', "$Dir\probe.ps1", '-Label', 'standard-user', '-SummaryPath', "$Dir\summary.md")
$p = Start-Process -FilePath $pwsh -ArgumentList $argList -Credential $cred -LoadUserProfile -WorkingDirectory $Dir -RedirectStandardOutput "$Dir\out.txt" -RedirectStandardError "$Dir\err.txt" -PassThru
if (-not $p.WaitForExit(35 * 60 * 1000)) {
    Write-Host 'Standard-user probe timed out; killing it'
    $p.Kill($true)
}
Write-Host "Standard-user probe exit code: $($p.ExitCode)"
Write-Host '----- stdout -----'
if (Test-Path "$Dir\out.txt") { Get-Content "$Dir\out.txt" }
Write-Host '----- stderr -----'
if (Test-Path "$Dir\err.txt") { Get-Content "$Dir\err.txt" }
if ((Test-Path "$Dir\summary.md") -and $env:GITHUB_STEP_SUMMARY) {
    Get-Content "$Dir\summary.md" | Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY
}
