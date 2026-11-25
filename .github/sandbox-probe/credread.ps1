param([string]$Target)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class ProbeCred
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct CREDENTIAL
    {
        public uint Flags; public uint Type; public string TargetName; public string Comment;
        public long LastWritten; public uint CredentialBlobSize; public IntPtr CredentialBlob;
        public uint Persist; public uint AttributeCount; public IntPtr Attributes;
        public string TargetAlias; public string UserName;
    }
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CredReadW(string target, uint type, uint flags, out IntPtr cred);
    [DllImport("advapi32.dll")]
    static extern void CredFree(IntPtr p);
    public static string Read(string target)
    {
        IntPtr p;
        if (!CredReadW(target, 1, 0, out p)) return "ERROR CredRead failed: error " + Marshal.GetLastWin32Error();
        var c = (CREDENTIAL)Marshal.PtrToStructure(p, typeof(CREDENTIAL));
        string secret = Marshal.PtrToStringUni(c.CredentialBlob, (int)c.CredentialBlobSize / 2);
        string r = "user=" + c.UserName + " secret=" + secret;
        CredFree(p);
        return r;
    }
}
'@
$r = [ProbeCred]::Read($Target)
Write-Output $r
if ($r.StartsWith('ERROR')) { exit 1 }
exit 0
