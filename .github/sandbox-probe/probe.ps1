#requires -Version 7.2
param(
    [string]$SummaryPath = $env:GITHUB_STEP_SUMMARY,
    [string]$Label = 'runner-user'
)

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
$Source = $PSScriptRoot
Add-Type -Path (Join-Path $Source 'Sandbox.cs')
[Sandbox]::QuietErrors()

$Facts = [ordered]@{}
$Results = [ordered]@{}
$Matrix = [ordered]@{}

function Section([string]$Title) {
    Write-Host ''
    Write-Host ('=' * 100)
    Write-Host "== $Title"
    Write-Host ('=' * 100)
}

function Show([string]$Text, [int]$Max = 6, [string]$Filter = '') {
    if (-not $Text) { return }
    $lines = @($Text -split "`r?`n" | Where-Object { $_.Trim() -ne '' })
    if ($Filter) { $lines = @($lines | Where-Object { $_ -match $Filter }) }
    foreach ($line in ($lines | Select-Object -First $Max)) { Write-Host "         | $line" }
    if ($lines.Count -gt $Max) { Write-Host "         | ... ($($lines.Count - $Max) more lines)" }
}

function Fact([string]$Key, $Value) {
    $script:Facts[$Key] = "$Value"
    Write-Host "FACT  $Key = $Value"
}

function Ace-State([string]$Path, [string]$SidValue) {
    try {
        $acl = Get-Acl -LiteralPath $Path -ErrorAction Stop
        $rules = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | Where-Object { $_.IdentityReference.Value -eq $SidValue })
        if ($rules.Count -eq 0) { return "no ACE for SID (protected=$($acl.AreAccessRulesProtected))" }
        return (($rules | ForEach-Object { "{0} {1} {2}" -f $_.AccessControlType, $_.FileSystemRights, $(if ($_.IsInherited) { 'inherited' } else { 'explicit' }) }) -join '; ')
    }
    catch { return "cannot read ACL: $($_.Exception.Message)" }
}

$RealProfile = [Environment]::GetFolderPath('UserProfile')
if ($env:USERPROFILE -ne $RealProfile) {
    Write-Host "Environment belongs to another user ($env:USERPROFILE); switching it to $RealProfile"
    $env:USERPROFILE = $RealProfile
    $env:HOMEDRIVE = $RealProfile.Substring(0, 2)
    $env:HOMEPATH = $RealProfile.Substring(2)
    $env:APPDATA = [Environment]::GetFolderPath('ApplicationData')
    $env:LOCALAPPDATA = [Environment]::GetFolderPath('LocalApplicationData')
    $env:USERNAME = [Environment]::UserName
    $env:TEMP = Join-Path $env:LOCALAPPDATA 'Temp'
    $env:TMP = $env:TEMP
    New-Item -ItemType Directory -Force $env:TEMP | Out-Null
}

Section "0. Environment ($Label)"
$os = Get-CimInstance Win32_OperatingSystem
Write-Host "OS: $($os.Caption) $($os.Version)"
Write-Host "PowerShell: $($PSVersionTable.PSVersion)"
$UserSid = [Sandbox]::UserSid()
$LogonSid = [Sandbox]::LogonSid()
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
$IsAdmin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$lua = (Get-ItemProperty HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System -ErrorAction SilentlyContinue).EnableLUA
Fact 'User' "$env:USERDOMAIN\$env:USERNAME ($UserSid)"
Fact 'Logon SID' $LogonSid
Fact 'Parent token is elevated admin' "$IsAdmin (UAC EnableLUA=$lua)"
Write-Host "USERPROFILE=$env:USERPROFILE APPDATA=$env:APPDATA TEMP=$env:TEMP"
Show (whoami.exe /groups | Out-String) 40
Write-Host 'Parent privileges before removal:'
[Sandbox]::Privileges() | ForEach-Object { Write-Host "   $_" }
foreach ($name in 'SeAssignPrimaryTokenPrivilege', 'SeIncreaseQuotaPrivilege') {
    Write-Host "   $([Sandbox]::RemovePrivilege($name))"
}
$privAfter = [Sandbox]::Privileges()
Write-Host 'Parent privileges after removal (every sandboxed launch below happens with these):'
$privAfter | ForEach-Object { Write-Host "   $_" }
Fact 'Parent holds SeAssignPrimaryTokenPrivilege at launch' ([bool]($privAfter -match 'SeAssignPrimaryToken'))
Fact 'Parent holds SeIncreaseQuotaPrivilege at launch' ([bool]($privAfter -match 'SeIncreaseQuota'))

Section '1. Choose the synthetic SID'
$AclEdit = Join-Path $Source 'acledit.ps1'
Write-Host 'icacls.exe cannot take an unmapped SID (it fails with error 1332), so ACEs for the synthetic SID go through .NET FileSystemAclExtensions.SetAccessControl, which calls SetNamedSecurityInfo.'
$Scratch = Join-Path $env:TEMP 'probe-sid-scratch'
New-Item -ItemType Directory -Force $Scratch | Out-Null
$Sid = $null
foreach ($candidate in 'S-1-5-21-0-0-0-4242', 'S-1-5-21-4242-4242-4242-4242', 'S-1-9-4242') {
    $parse = [Sandbox]::ParseSid($candidate)
    $out = & $AclEdit grant $Scratch $candidate | Out-String
    $code = $LASTEXITCODE
    Write-Host "   $candidate : $parse ; SetNamedSecurityInfo grant exit=$code"
    Show $out 2
    if (-not $Sid -and $code -eq 0 -and $parse.StartsWith('valid')) { $Sid = $candidate }
}
if (-not $Sid) { Write-Host 'No candidate SID worked; stopping.'; exit 0 }
Fact 'Synthetic SID used' $Sid

$UserHome = $env:USERPROFILE
$Data = Join-Path $env:APPDATA 'dotori-probe'
$DataRenamed = "$Data-renamed"
$Work = Join-Path $Data 'work'
$Vault = Join-Path $Data 'vault.key'
$ConfigFile = Join-Path $Data 'config.toml'
$HomeTest = Join-Path $UserHome 'probe-home-test'
$Tools = Join-Path $UserHome 'probe-tools'
$SshDir = Join-Path $UserHome '.ssh'
$SshKey = Join-Path $SshDir 'probe_ed25519'
$EscapeFile = Join-Path $HomeTest 'escape.txt'
$VaultText = 'VAULT-SECRET-7f3a9c'
$CredTarget = 'dotori-probe-cred'
$CredSecret = 's3cret-probe-91'
$GitExe = (Get-Command git.exe -ErrorAction SilentlyContinue).Source
$GitRoot = if ($GitExe) { Split-Path (Split-Path $GitExe) } else { 'C:\Program Files\Git' }
$Bash = Join-Path $GitRoot 'bin\bash.exe'
$GitSsh = Join-Path $GitRoot 'usr\bin\ssh.exe'
$WinSsh = Join-Path $env:SystemRoot 'System32\OpenSSH\ssh.exe'
$WinSshKeygen = Join-Path $env:SystemRoot 'System32\OpenSSH\ssh-keygen.exe'
$Pwsh = (Get-Process -Id $PID).Path
$BareUrl = 'file:///' + ($HomeTest -replace '\\', '/') + '/bare.git'
$Variant = 'D'
Write-Host "git=$GitExe bash=$Bash gitssh=$GitSsh winssh=$WinSsh pwsh=$Pwsh"
Write-Host "first ssh on PATH: $((Get-Command ssh.exe -ErrorAction SilentlyContinue).Source)"
Write-Host "cargo on PATH: $((Get-Command cargo.exe -ErrorAction SilentlyContinue).Source)"
Write-Host "npm on PATH: $((Get-Command npm.cmd -ErrorAction SilentlyContinue).Source)"

function NewCase {
    param(
        [string]$Id, [string]$Name, [string]$Expect, [string]$Cmd,
        [string]$Group = 'other', [scriptblock]$Check = $null, [int]$Timeout = 120,
        [string]$Cwd = $HomeTest, [scriptblock]$Before = $null, [scriptblock]$After = $null,
        [int]$ShowMax = 6, [string]$ShowFilter = ''
    )
    [pscustomobject]@{
        Id = $Id; Name = $Name; Expect = $Expect; Cmd = $Cmd; Group = $Group; Check = $Check; Timeout = $Timeout
        Cwd = $Cwd; Before = $Before; After = $After; ShowMax = $ShowMax; ShowFilter = $ShowFilter
    }
}

$ReadVault = { param($r) $r.ExitCode -eq 0 -and $r.Output -match $VaultText }
$SshCheck = {
    param($r)
    $r.Output -match 'Permission denied \(publickey\)' -and $r.Output -match 'Offering public key' -and $r.Output -notmatch 'UNPROTECTED PRIVATE KEY|[Bb]ad permissions'
}
$SshFilter = 'Offering public key|UNPROTECTED|[Bb]ad permissions|Permission denied|Load key|Authenticated to|no such identity|Could not|known_hosts|denied|Connection'
$EscapeCheck = {
    param($r)
    if ($r.ExitCode -ne 0) { return $false }
    $deadline = (Get-Date).AddSeconds(25)
    while ((Get-Date) -lt $deadline) {
        if ((Test-Path -LiteralPath $EscapeFile) -and ((Get-Content -LiteralPath $EscapeFile -Raw) -match $VaultText)) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}
$EscapeClean = {
    & schtasks.exe /delete /tn DotoriProbeEscape /f 2>&1 | Out-Null
    & schtasks.exe /delete /tn DotoriProbeEscapeS4U /f 2>&1 | Out-Null
    Remove-Item -LiteralPath $EscapeFile -Force -ErrorAction SilentlyContinue
}
$TaskAction = "cmd /c type $Vault > $EscapeFile"
$DataAclBefore = $null

$Cases = @(
    NewCase '01a' 'cmd /c echo hi' 'works' 'cmd.exe /d /c echo hi' -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match 'hi' }
    NewCase '01b' 'whoami' 'works' 'whoami.exe'
    NewCase '01c' 'whoami /groups (info)' 'record' 'whoami.exe /groups' -ShowMax 30
    NewCase '01d' 'whoami /priv (info)' 'record' 'whoami.exe /priv' -ShowMax 12
    NewCase '02a' 'write file in %USERPROFILE%\probe-home-test' 'works' "cmd.exe /d /c echo child-wrote> `"$HomeTest\child.txt`""
    NewCase '02b' 'read that file back' 'works' "cmd.exe /d /c type `"$HomeTest\child.txt`"" -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match 'child-wrote' }
    NewCase '02c' 'read a file the parent made in home test' 'works' "cmd.exe /d /c type `"$HomeTest\seed.txt`""
    NewCase '03a' 'read dotori-probe\vault.key' 'fails' "cmd.exe /d /c type `"$Vault`"" -Group acl -Check $ReadVault
    NewCase '03b' 'list dotori-probe' 'fails' "cmd.exe /d /c dir /b `"$Data`"" -Group acl -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match 'vault' }
    NewCase '03c' 'read dotori-probe\config.toml' 'fails' "cmd.exe /d /c type `"$ConfigFile`"" -Group acl -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match 'probe' }
    NewCase '04a' 'write dotori-probe\work\x.txt' 'works' "cmd.exe /d /c echo work-ok> `"$Work\x.txt`"" -Group acl
    NewCase '04b' 'read dotori-probe\work\x.txt' 'works' "cmd.exe /d /c type `"$Work\x.txt`"" -Group acl -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match 'work-ok' }
    NewCase '04c' 'list dotori-probe\work' 'works' "cmd.exe /d /c dir /b `"$Work`"" -Group acl
    NewCase '05a' 'read data folder ACL (icacls, READ_CONTROL)' 'fails' "icacls.exe `"$Data`"" -Group acl -Before { $script:DataAclBefore = (& icacls.exe $Data 2>&1 | Out-String) }
    NewCase '05b' 'icacls /grant Everyone:(OI)(CI)F on data folder' 'fails' "icacls.exe `"$Data`" /grant *S-1-1-0:(OI)(CI)F" -Group acl
    NewCase '05c' 'remove the SID ACEs on data folder (like /remove:d)' 'fails' "`"$Pwsh`" -NoProfile -NonInteractive -File `"$Tools\acledit.ps1`" purge `"$Data`" $Sid" -Group acl -Timeout 90
    NewCase '05c2' 'grant the SID full control on data folder' 'fails' "`"$Pwsh`" -NoProfile -NonInteractive -File `"$Tools\acledit.ps1`" grant `"$Data`" $Sid" -Group acl -Timeout 90
    NewCase '05d' 'icacls /reset on data folder' 'fails' "icacls.exe `"$Data`" /reset" -Group acl
    NewCase '05e' 'takeown /f data folder' 'fails' "takeown.exe /f `"$Data`"" -Group acl
    NewCase '05f' 'read vault.key again after 05b-05e' 'fails' "cmd.exe /d /c type `"$Vault`"" -Group acl -Check $ReadVault -After {
        param($r)
        $now = (& icacls.exe $Data 2>&1 | Out-String)
        $changed = $now -ne $script:DataAclBefore
        Write-Host "         > parent view: data folder ACL changed during 05b-05e: $changed"
        Show $now 12
        $script:Facts["Data ACL changed by 05b-05e [$($script:CurrentMode)]"] = "$changed"
    }
    NewCase '06a' 'create planted.txt next to vault.key' 'fails' "cmd.exe /d /c echo planted> `"$Data\planted.txt`"" -Group acl -Check { param($r) Test-Path -LiteralPath (Join-Path $Data 'planted.txt') }
    NewCase '06b' 'delete vault.key' 'fails' "cmd.exe /d /c del /f /q `"$Vault`"" -Group acl -Check { param($r) -not (Test-Path -LiteralPath $Vault) }
    NewCase '06c' 'rename data folder' 'record' "cmd.exe /d /c ren `"$Data`" dotori-probe-renamed" -Group acl -Check { param($r) Test-Path -LiteralPath $DataRenamed } -After {
        param($r)
        if (Test-Path -LiteralPath $DataRenamed) {
            Write-Host '         > parent: data folder WAS renamed by the child; renaming it back'
            Rename-Item -LiteralPath $DataRenamed -NewName 'dotori-probe'
        }
    }
    NewCase '06d' 'rd /s /q data folder' 'record' "cmd.exe /d /c rd /s /q `"$Data`"" -Group acl -Check { param($r) -not (Test-Path -LiteralPath $Data) } -After {
        param($r)
        Write-Host ("         > parent: after rd, data folder exists={0} vault.key exists={1} work exists={2} work\x.txt exists={3}" -f (Test-Path -LiteralPath $Data), (Test-Path -LiteralPath $Vault), (Test-Path -LiteralPath $Work), (Test-Path -LiteralPath "$Work\x.txt"))
        New-DataFolder $script:Variant -Quiet
    }
    NewCase '07a' 'git --version' 'works' 'git.exe --version'
    NewCase '07b' 'git init + add + commit' 'works' 'cmd.exe /d /c git init -q repo && cd repo && echo hi> f.txt && git add f.txt && git -c user.name=probe -c user.email=probe@example.invalid commit -q -m init && git log --oneline'
    NewCase '07c' 'git clone over HTTPS (octocat/Hello-World)' 'works' 'git.exe clone -q --depth 1 https://github.com/octocat/Hello-World.git hw' -Timeout 180
    NewCase '07d' 'git ls-remote over HTTPS' 'works' 'git.exe ls-remote https://github.com/octocat/Hello-World.git HEAD'
    NewCase '07e' 'ssh -V (Windows OpenSSH)' 'works' "`"$WinSsh`" -V"
    NewCase '07f' 'ssh -V (Git for Windows ssh)' 'works' "`"$GitSsh`" -V"
    NewCase '07g' 'ssh -T git@github.com with key (Windows OpenSSH)' 'works' "`"$WinSsh`" -v -i `"$SshKey`" -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=accept-new -T git@github.com" -Group ssh -Check $SshCheck -ShowFilter $SshFilter -Timeout 60
    NewCase '07h' 'ssh -T git@github.com with key (Git ssh)' 'works' "`"$GitSsh`" -v -i `"$SshKey`" -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=accept-new -T git@github.com" -Group ssh -Check $SshCheck -ShowFilter $SshFilter -Timeout 60
    NewCase '07i' 'git push to local bare repo over file://' 'works' "cmd.exe /d /c git init -q --bare bare.git && cd repo && git push -q $BareUrl HEAD:refs/heads/main && git ls-remote $BareUrl"
    NewCase '08a' 'npm --version' 'works' 'cmd.exe /d /c npm --version'
    NewCase '08b' 'npm init -y' 'works' 'cmd.exe /d /c mkdir npmproj && cd npmproj && npm init -y'
    NewCase '08c' 'npm install is-number (network)' 'works' 'cmd.exe /d /c cd npmproj && npm install is-number --no-audit --no-fund' -Timeout 240
    NewCase '09a' 'cargo --version' 'works' 'cmd.exe /d /c cargo --version'
    NewCase '09b' 'cargo new hello' 'works' 'cmd.exe /d /c cargo new hello'
    NewCase '09c' 'cargo build hello' 'works' 'cmd.exe /d /c cd hello && cargo build' -Timeout 300 -ShowMax 10
    NewCase '10a' 'cmdkey /list shows the probe credential' 'works' 'cmdkey.exe /list' -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match $CredTarget }
    NewCase '10b' 'CredRead of the probe credential' 'works' "`"$Pwsh`" -NoProfile -NonInteractive -File `"$Tools\credread.ps1`" $CredTarget" -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match $CredSecret } -Timeout 90
    NewCase '11a' 'reg query HKCU\Software\DotoriProbe' 'works' 'reg.exe query HKCU\Software\DotoriProbe /v seed'
    NewCase '11b' 'reg add HKCU\Software\DotoriProbe' 'works' 'reg.exe add HKCU\Software\DotoriProbe /v child /d 1 /f'
    NewCase '11c' 'reg query HKCU\...\Explorer (existing key)' 'works' 'reg.exe query HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer'
    NewCase '12a' 'Git Bash: echo hi && ls ~' 'works' "`"$Bash`" -c `"echo hi && ls ~ | head -n 5`"" -Check { param($r) $r.ExitCode -eq 0 -and $r.Output -match 'hi' }
    NewCase '13a' 'pwsh starts (extra)' 'works' "`"$Pwsh`" -NoProfile -NonInteractive -Command `"'pwsh-ok'`"" -Timeout 90
    NewCase '13b' 'OpenProcess(parent probe, VM_WRITE+CREATE_THREAD) (extra)' 'fails' "`"$Pwsh`" -NoProfile -NonInteractive -File `"$Tools\openproc.ps1`" $PID 42" -Timeout 90
    NewCase '13c' 'escape: schtasks task (interactive) reads vault (extra)' 'fails' "cmd.exe /d /c schtasks /create /tn DotoriProbeEscape /sc once /st 23:59 /f /tr `"$TaskAction`" && schtasks /run /tn DotoriProbeEscape" -Check $EscapeCheck -Before $EscapeClean -After { param($r) & $EscapeClean }
    NewCase '13d' 'escape: schtasks task (S4U /np) reads vault (extra)' 'fails' "cmd.exe /d /c schtasks /create /tn DotoriProbeEscapeS4U /ru $env:USERDOMAIN\$env:USERNAME /np /sc once /st 23:59 /f /tr `"$TaskAction`" && schtasks /run /tn DotoriProbeEscapeS4U" -Check $EscapeCheck -Before $EscapeClean -After { param($r) & $EscapeClean }
)

$MatrixCases = @(
    NewCase 'm1' 'echo' 'works' 'cmd.exe /d /c echo hi' -Timeout 30
    NewCase 'm2' 'whoami' 'works' 'whoami.exe' -Timeout 30
    NewCase 'm3' 'home write+read' 'works' "cmd.exe /d /c echo m> `"$HomeTest\m.txt`" && type `"$HomeTest\m.txt`"" -Timeout 30
    NewCase 'm4' 'HKCU read' 'works' 'reg.exe query HKCU\Software\DotoriProbe /v seed' -Timeout 30
    NewCase 'm5' 'git --version' 'works' 'git.exe --version' -Timeout 30
    NewCase 'm6' 'pwsh start' 'works' "`"$Pwsh`" -NoProfile -NonInteractive -Command `"'pwsh-ok'`"" -Timeout 60
    NewCase 'm7' 'vault read' 'fails' "cmd.exe /d /c type `"$Vault`"" -Check $ReadVault -Timeout 30
    NewCase 'm8' 'data ACL read' 'fails' "icacls.exe `"$Data`"" -Timeout 30
)

$CurrentMode = ''
function Invoke-Case([string]$Mode, [IntPtr]$Token, $Case, [switch]$NoStore) {
    $script:CurrentMode = $Mode
    if ($Case.Before) { & $Case.Before | Out-Null }
    $r = [Sandbox]::Run($Token, $Case.Cmd, $Case.Cwd, $Case.Timeout * 1000)
    if ($r.LaunchError) { $got = 'fails' }
    else {
        $ok = if ($Case.Check) { [bool](& $Case.Check $r) } else { $r.ExitCode -eq 0 }
        $got = if ($ok -and -not $r.TimedOut) { 'works' } else { 'fails' }
    }
    $exp = $Case.Expect
    if (($Mode -like 'baseline*' -or $Mode -eq 'pregrant') -and $exp -ne 'record') { $exp = 'works' }
    $verdict = if ($exp -eq 'record') { 'INFO' } elseif ($got -eq $exp) { 'PASS' } else { 'FAIL' }
    $why = ''
    if ($r.LaunchError) { $why = 'launch error' }
    elseif ($r.TimedOut) { $why = 'timeout' }
    elseif ($got -eq 'fails' -and $r.Output -match '[Dd]enied') { $why = 'access denied' }
    $code = if ($r.LaunchError) { '-' } elseif ($r.ExitCode -lt 0) { '0x{0:X8}' -f $r.ExitCode } else { "$($r.ExitCode)" }
    $line = '{0,-4} [{1,-10}] {2,-4} {3,-58} expect={4,-6} got={5,-5} exit={6,-10} {7,5:N1}s {8}' -f $verdict, $Mode, $Case.Id, $Case.Name, $exp, $got, $code, $r.Seconds, $why
    Write-Host $line
    if ($r.LaunchError) { Show $r.LaunchError } else { Show $r.Output $Case.ShowMax $Case.ShowFilter }
    if ($Case.After) { & $Case.After $r | Out-Null }
    $cell = $got
    if ($why) { $cell += " ($why)" }
    if ($verdict -eq 'FAIL') { $cell += ' **FAIL**' }
    if (-not $NoStore) {
        if (-not $script:Results.Contains($Case.Id)) { $script:Results[$Case.Id] = [ordered]@{ Name = $Case.Name; Expect = $Case.Expect; Modes = @{} } }
        $script:Results[$Case.Id].Modes[$Mode] = $cell
    }
    return [pscustomobject]@{ Got = $got; Verdict = $verdict; Cell = $cell }
}

function Remove-DataTree([string]$Path) {
    if (Test-Path -LiteralPath $Path) {
        & icacls.exe $Path /reset /t /c /q 2>&1 | Out-Null
        Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $Path) {
            & takeown.exe /f $Path /r /d y 2>&1 | Out-Null
            & icacls.exe $Path /reset /t /c /q 2>&1 | Out-Null
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

function New-DataFolder([string]$Which, [switch]$Quiet) {
    Remove-DataTree $Data
    Remove-DataTree $DataRenamed
    New-Item -ItemType Directory -Force $Work | Out-Null
    Set-Content -LiteralPath $Vault -Value $VaultText -NoNewline
    Set-Content -LiteralPath $ConfigFile -Value 'name = "probe"'
    $log = @()
    $log += & icacls.exe $Data /setowner "*$UserSid" /t /c /q 2>&1
    if ($Which -eq 'D') {
        $log += & $AclEdit deny $Data $Sid
    }
    else {
        $log += & $AclEdit protect-purge $Data $Sid
    }
    $log += & $AclEdit grant $Work $Sid
    if (-not $Quiet) {
        Write-Host "Data folder variant $Which built. icacls messages:"
        Show ($log | Out-String) 10
        foreach ($p in $Data, $Vault, $Work) {
            Write-Host "   ACL of ${p}:"
            Show (& icacls.exe $p 2>&1 | Out-String) 12
        }
        Write-Host "   owner of data folder: $((Get-Acl -LiteralPath $Data).Owner)"
    }
}

function Reset-HomeTest {
    if (Test-Path -LiteralPath $HomeTest) { Remove-Item -LiteralPath $HomeTest -Recurse -Force -ErrorAction SilentlyContinue }
    New-Item -ItemType Directory -Force $HomeTest | Out-Null
    Set-Content -LiteralPath (Join-Path $HomeTest 'seed.txt') -Value 'seed-from-parent'
}

Section '2. Setup as the normal user'
& $EscapeClean
Remove-DataTree $Data
Remove-DataTree $DataRenamed
Reset-HomeTest
New-Item -ItemType Directory -Force $Tools | Out-Null
Copy-Item -LiteralPath (Join-Path $Source 'credread.ps1'), (Join-Path $Source 'openproc.ps1'), (Join-Path $Source 'acledit.ps1') -Destination $Tools -Force
$Deep = Join-Path $HomeTest 'deep\a\b\c'
New-Item -ItemType Directory -Force $Deep | Out-Null
Set-Content -LiteralPath (Join-Path $Deep 'old.txt') -Value 'made before the home grant'
New-Item -ItemType Directory -Force $SshDir | Out-Null
if (-not (Test-Path -LiteralPath $SshKey)) {
    $kg = [Sandbox]::Run([IntPtr]::Zero, "`"$WinSshKeygen`" -t ed25519 -N `"`" -C probe -q -f `"$SshKey`"", $UserHome, 60000)
    Write-Host "ssh-keygen exit=$($kg.ExitCode) $($kg.LaunchError)"
    Show $kg.Output
}
Write-Host "ACL of the private key before the home grant:"
Show (& icacls.exe $SshKey 2>&1 | Out-String) 8

New-Item -Path HKCU:\Software\DotoriProbe -Force | Out-Null
Set-ItemProperty -Path HKCU:\Software\DotoriProbe -Name seed -Value 'parent'
$ck = & cmdkey.exe "/generic:$CredTarget" /user:probe "/pass:$CredSecret" 2>&1 | Out-String
Write-Host "cmdkey add (parent, normal user): exit=$LASTEXITCODE"
Show $ck

Write-Host ''
Write-Host 'Baseline ssh checks BEFORE the home grant (parent token, no sandbox):'
foreach ($c in $Cases | Where-Object Group -eq 'ssh') { Invoke-Case 'pregrant' ([IntPtr]::Zero) $c | Out-Null }

Write-Host ''
Write-Host 'Profile size:'
$eo = [IO.EnumerationOptions]::new()
$eo.RecurseSubdirectories = $true
$eo.IgnoreInaccessible = $true
$eo.AttributesToSkip = [IO.FileAttributes]::ReparsePoint
$sw = [Diagnostics.Stopwatch]::StartNew()
$DirList = [IO.Directory]::GetDirectories($UserHome, '*', $eo)
$fileCount = 0
foreach ($f in [IO.Directory]::EnumerateFiles($UserHome, '*', $eo)) { $fileCount++ }
Fact 'Profile size' ("{0} folders, {1} files (counted in {2:N1}s)" -f $DirList.Count, $fileCount, $sw.Elapsed.TotalSeconds)

Write-Host ''
Write-Host "Applying the home grant: SetNamedSecurityInfo on `"$UserHome`" adding Allow $Sid FullControl (OI)(CI)"
$script:GrantOut = ''
$grantTime = Measure-Command { $script:GrantOut = & $AclEdit grant $UserHome $Sid | Out-String; $script:GrantCode = $LASTEXITCODE }
Show $script:GrantOut 10
Fact 'Home grant (one SetNamedSecurityInfo call on the profile root) time' ("{0:N1}s, exit={1}" -f $grantTime.TotalSeconds, $script:GrantCode)

$probePaths = @(
    (Join-Path $Deep 'old.txt'), $SshKey, $env:APPDATA, $env:LOCALAPPDATA, $env:TEMP,
    (Join-Path $env:APPDATA 'Microsoft\Protect'), (Join-Path $env:APPDATA 'Microsoft\Credentials'),
    (Join-Path $env:LOCALAPPDATA 'Microsoft\Credentials'), (Join-Path $UserHome '.cargo\bin\cargo.exe'),
    (Join-Path $UserHome '.rustup'), (Join-Path $UserHome '.gitconfig'), (Join-Path $UserHome 'NTUSER.DAT')
)
Write-Host 'Did the inheritable ACE reach existing children?'
foreach ($p in $probePaths) {
    if (Test-Path -LiteralPath $p -ErrorAction SilentlyContinue) { Write-Host ("   {0,-70} {1}" -f $p, (Ace-State $p $Sid)) }
    else { Write-Host ("   {0,-70} (absent)" -f $p) }
}
$oldState = Ace-State (Join-Path $Deep 'old.txt') $Sid
Fact 'Pre-existing deep file got the ACE' $oldState
if ($oldState -notmatch 'Allow') {
    Write-Host 'Root grant did not propagate to the pre-existing file.'
}

Write-Host ''
Write-Host 'Scanning every folder in the profile for the SID ACE:'
$sw = [Diagnostics.Stopwatch]::StartNew()
$withAce = 0; $missing = [Collections.Generic.List[string]]::new(); $errors = 0; $protectedCount = 0
$sidObj = [Security.Principal.SecurityIdentifier]::new($Sid)
foreach ($d in $DirList) {
    if ($sw.Elapsed.TotalSeconds -gt 240) { Write-Host '   (scan stopped after 240s)'; break }
    try {
        $acl = [IO.FileSystemAclExtensions]::GetAccessControl([IO.DirectoryInfo]::new($d), [Security.AccessControl.AccessControlSections]::Access)
        if ($acl.AreAccessRulesProtected) { $protectedCount++ }
        $hit = $false
        foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
            if ($rule.IdentityReference -eq $sidObj -and $rule.AccessControlType -eq 'Allow') { $hit = $true; break }
        }
        if ($hit) { $withAce++ } else { $missing.Add($d) }
    }
    catch { $errors++ }
}
Fact 'Profile folders with the SID allow ACE after grant' ("{0} of {1}; missing {2}; protected DACLs {3}; unreadable {4}; scan {5:N1}s" -f $withAce, $DirList.Count, $missing.Count, $protectedCount, $errors, $sw.Elapsed.TotalSeconds)
foreach ($m in ($missing | Select-Object -First 25)) { Write-Host "   missing: $m" }

Write-Host ''
Write-Host 'ACL of the private key after the home grant:'
Show (& icacls.exe $SshKey 2>&1 | Out-String) 8

New-DataFolder 'D'

Section '3. Restricting-SID matrix (quick tests per token shape)'
$Base = @($Sid, 'S-1-1-0', 'S-1-5-32-545', 'S-1-5-11')
$BaseLogon = if ($LogonSid) { $Base + @($LogonSid) } else { $Base }
$DaclSids = @($UserSid, 'S-1-5-18', $Sid)
$Configs = [ordered]@{
    'A' = @{ Desc = 'synthetic SID only'; Sids = @($Sid); Def = $false; Tok = $false }
    'B' = @{ Desc = 'synthetic + Everyone + Users + Authenticated Users + logon SID'; Sids = $BaseLogon; Def = $false; Tok = $false }
    'C' = @{ Desc = 'B + default DACL (user, SYSTEM, synthetic)'; Sids = $BaseLogon; Def = $true; Tok = $false }
    'D' = @{ Desc = 'C + token object DACL (user, SYSTEM, synthetic)'; Sids = $BaseLogon; Def = $true; Tok = $true }
    'E' = @{ Desc = 'D without the logon SID'; Sids = $Base; Def = $true; Tok = $true }
    'F' = @{ Desc = 'D + RESTRICTED (S-1-5-12)'; Sids = $BaseLogon + @('S-1-5-12'); Def = $true; Tok = $true }
}
foreach ($key in $Configs.Keys) {
    $cfg = $Configs[$key]
    Write-Host ''
    Write-Host "--- config $key : $($cfg.Desc)"
    try {
        $tok = [Sandbox]::MakeToken([string[]]$cfg.Sids, $true, $(if ($cfg.Def) { [string[]]$DaclSids } else { $null }), $(if ($cfg.Tok) { [string[]]$DaclSids } else { $null }))
        $cfg.Token = $tok
        Write-Host "    restricting SIDs in token: $([Sandbox]::RestrictedSidsOf($tok) -join ', ')"
    }
    catch {
        Write-Host "    token creation failed: $($_.Exception.Message)"
        $cfg.Token = [IntPtr]::Zero
        $Matrix[$key] = [ordered]@{ Error = $_.Exception.Message }
        continue
    }
    $row = [ordered]@{}
    $score = 0
    foreach ($mc in $MatrixCases) {
        $res = Invoke-Case "matrix-$key" $tok $mc -NoStore
        $row[$mc.Id] = $res.Cell
        if ($res.Verdict -eq 'PASS') { $score++ }
    }
    $cfg.Score = $score
    $Matrix[$key] = $row
}
$Chosen = $null
foreach ($key in 'D', 'F', 'C', 'B', 'E', 'A') {
    $cfg = $Configs[$key]
    if ($cfg.Token -eq [IntPtr]::Zero -or $null -eq $cfg.Score) { continue }
    if ($null -eq $Chosen -or $cfg.Score -gt $Configs[$Chosen].Score) { $Chosen = $key }
}
Fact 'Token shape used for the full sandbox run' "$Chosen ($($Configs[$Chosen].Desc)), matrix score $($Configs[$Chosen].Score)/$($MatrixCases.Count)"
$MainToken = $Configs[$Chosen].Token

Section "4. Full list in the SANDBOX, data folder variant D (explicit inheritable DENY for the SID)"
$Variant = 'D'
New-DataFolder 'D' -Quiet
Reset-HomeTest
foreach ($c in $Cases) { Invoke-Case 'sandbox-D' $MainToken $c | Out-Null }

Section "5. ACL tests in the SANDBOX, data folder variant P (inheritance cut, SID removed, no deny)"
$Variant = 'P'
New-DataFolder 'P'
Reset-HomeTest
foreach ($c in $Cases | Where-Object Group -eq 'acl') { Invoke-Case 'sandbox-P' $MainToken $c | Out-Null }

Section "6. Full list WITHOUT the sandbox (baseline, same parent, CreateProcess)"
$Variant = 'D'
New-DataFolder 'D' -Quiet
Reset-HomeTest
foreach ($c in $Cases) { Invoke-Case 'baseline' ([IntPtr]::Zero) $c | Out-Null }

Section '7. Summary'
& $EscapeClean
$md = [Text.StringBuilder]::new()
[void]$md.AppendLine("## Sandbox probe: $Label")
[void]$md.AppendLine('')
foreach ($k in $Facts.Keys) { [void]$md.AppendLine("- **$k**: $($Facts[$k])") }
[void]$md.AppendLine('')
[void]$md.AppendLine('### Restricting-SID matrix')
[void]$md.AppendLine('')
$hdr = '| Config | ' + (($MatrixCases | ForEach-Object { "$($_.Id) $($_.Name) (expect $($_.Expect))" }) -join ' | ') + ' |'
[void]$md.AppendLine($hdr)
[void]$md.AppendLine('|' + ('---|' * ($MatrixCases.Count + 1)))
foreach ($key in $Matrix.Keys) {
    $row = $Matrix[$key]
    if ($row.Contains('Error')) { [void]$md.AppendLine("| $key $($Configs[$key].Desc) | token error: $($row.Error) |"); continue }
    [void]$md.AppendLine("| $key $($Configs[$key].Desc) | " + (($MatrixCases | ForEach-Object { $row[$_.Id] }) -join ' | ') + ' |')
}
[void]$md.AppendLine('')
[void]$md.AppendLine("### Results (sandbox token shape $Chosen)")
[void]$md.AppendLine('')
[void]$md.AppendLine('| Id | Test | Expected in sandbox | Sandbox, DENY ACE (D) | Sandbox, no inheritance (P) | Baseline, no sandbox | Baseline before home grant |')
[void]$md.AppendLine('|---|---|---|---|---|---|---|')
foreach ($id in $Results.Keys) {
    $e = $Results[$id]
    $cells = foreach ($m in 'sandbox-D', 'sandbox-P', 'baseline', 'pregrant') { if ($e.Modes.ContainsKey($m)) { $e.Modes[$m] } else { '' } }
    [void]$md.AppendLine("| $id | $($e.Name) | $($e.Expect) | " + ($cells -join ' | ') + ' |')
}
$text = $md.ToString()
Write-Host $text
if ($SummaryPath) { Add-Content -LiteralPath $SummaryPath -Value $text }
exit 0
