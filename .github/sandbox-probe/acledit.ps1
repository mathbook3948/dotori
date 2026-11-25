param([string]$Action, [string]$Path, [string]$SidValue = '')
$ErrorActionPreference = 'Stop'
try {
    $info = if ([IO.Directory]::Exists($Path)) { [IO.DirectoryInfo]::new($Path) } else { [IO.FileInfo]::new($Path) }
    $sections = [Security.AccessControl.AccessControlSections]::Access
    $acl = [IO.FileSystemAclExtensions]::GetAccessControl($info, $sections)
    $sid = if ($SidValue) { [Security.Principal.SecurityIdentifier]::new($SidValue) } else { $null }
    $inherit = if ($info -is [IO.DirectoryInfo]) { [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit' } else { [Security.AccessControl.InheritanceFlags]::None }
    $full = [Security.AccessControl.FileSystemRights]::FullControl
    $none = [Security.AccessControl.PropagationFlags]::None
    switch ($Action) {
        'grant' { $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, $full, $inherit, $none, 'Allow')) }
        'deny' { $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, $full, $inherit, $none, 'Deny')) }
        'purge' { $acl.PurgeAccessRules($sid) }
        'owner-rights' { $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new('S-1-3-4'), [Security.AccessControl.FileSystemRights]::ReadPermissions, $inherit, $none, 'Allow')) }
        'protect-purge' {
            $acl.SetAccessRuleProtection($true, $true)
            [IO.FileSystemAclExtensions]::SetAccessControl($info, $acl)
            $acl = [IO.FileSystemAclExtensions]::GetAccessControl($info, $sections)
            $acl.PurgeAccessRules($sid)
        }
        default { throw "unknown action $Action" }
    }
    [IO.FileSystemAclExtensions]::SetAccessControl($info, $acl)
    Write-Output "OK $Action $Path $SidValue"
    exit 0
}
catch {
    Write-Output "ERROR $Action $Path $SidValue : $($_.Exception.Message)"
    exit 1
}
