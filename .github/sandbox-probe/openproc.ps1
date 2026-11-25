param([int]$TargetPid, [uint32]$Access)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ProbeOpen
{
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    public static string Try(int pid, uint access)
    {
        IntPtr h = OpenProcess(access, false, pid);
        if (h == IntPtr.Zero) return "ERROR OpenProcess(pid " + pid + ", access 0x" + access.ToString("X") + ") failed: error " + Marshal.GetLastWin32Error();
        return "OpenProcess(pid " + pid + ", access 0x" + access.ToString("X") + ") succeeded";
    }
}
'@
$r = [ProbeOpen]::Try($TargetPid, $Access)
Write-Output $r
if ($r.StartsWith('ERROR')) { exit 1 }
exit 0
