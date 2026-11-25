using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

public class RunResult
{
    public int ExitCode = -1;
    public string Output = "";
    public string LaunchError;
    public bool TimedOut;
    public bool InJob;
    public double Seconds;
}

public static class Sandbox
{
    const uint TOKEN_ALL_ACCESS = 0xF01FF;
    const uint DISABLE_MAX_PRIVILEGE = 0x1;
    const int TokenUser = 1, TokenGroups = 2, TokenPrivileges = 3, TokenDefaultDacl = 6, TokenRestrictedSids = 11;
    const uint SE_GROUP_LOGON_ID = 0xC0000000;
    const uint SE_PRIVILEGE_ENABLED = 0x2, SE_PRIVILEGE_REMOVED = 0x4;
    const uint CREATE_SUSPENDED = 0x4, CREATE_NO_WINDOW = 0x08000000;
    const int STARTF_USESTDHANDLES = 0x100;
    const uint GENERIC_READ = 0x80000000, GENERIC_ALL = 0x10000000;

    [StructLayout(LayoutKind.Sequential)]
    struct SID_AND_ATTRIBUTES { public IntPtr Sid; public uint Attributes; }

    [StructLayout(LayoutKind.Sequential)]
    struct LUID { public uint LowPart; public int HighPart; }

    [StructLayout(LayoutKind.Sequential)]
    struct TOKEN_PRIVILEGES_ONE { public uint Count; public LUID Luid; public uint Attributes; }

    [StructLayout(LayoutKind.Sequential)]
    struct SECURITY_ATTRIBUTES { public int nLength; public IntPtr lpSecurityDescriptor; public bool bInheritHandle; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO
    {
        public int cb; public string lpReserved; public string lpDesktop; public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }

    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION
    {
        public long PerProcessUserTimeLimit, PerJobUserTimeLimit; public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize; public uint ActiveProcessLimit;
        public UIntPtr Affinity; public uint PriorityClass, SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct IO_COUNTERS { public ulong A, B, C, D, E, F; }

    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
    {
        public JOBOBJECT_BASIC_LIMIT_INFORMATION Basic; public IO_COUNTERS Io;
        public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
    }

    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
    [DllImport("kernel32.dll")] static extern uint SetErrorMode(uint mode);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern int FormatMessageW(uint flags, IntPtr src, int id, int lang, StringBuilder buf, int size, IntPtr args);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool GetTokenInformation(IntPtr token, int cls, IntPtr info, int len, out int retLen);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SetTokenInformation(IntPtr token, int cls, IntPtr info, int len);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool CreateRestrictedToken(IntPtr existing, uint flags, uint disableSidCount, IntPtr sidsToDisable, uint deletePrivilegeCount, IntPtr privilegesToDelete, uint restrictedSidCount, [In] SID_AND_ATTRIBUTES[] sidsToRestrict, out IntPtr newToken);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool ConvertStringSidToSidW(string s, out IntPtr sid);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool ConvertSidToStringSidW(IntPtr sid, out IntPtr s);
    [DllImport("advapi32.dll")] static extern int GetLengthSid(IntPtr sid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool InitializeAcl(IntPtr acl, int len, int rev);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool AddAccessAllowedAce(IntPtr acl, int rev, uint mask, IntPtr sid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool InitializeSecurityDescriptor(IntPtr sd, int rev);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SetSecurityDescriptorDacl(IntPtr sd, bool present, IntPtr acl, bool defaulted);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool SetKernelObjectSecurity(IntPtr h, uint info, IntPtr sd);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool LookupPrivilegeNameW(string sys, ref LUID luid, StringBuilder name, ref int len);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool LookupPrivilegeValueW(string sys, string name, out LUID luid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool AdjustTokenPrivileges(IntPtr token, bool disableAll, ref TOKEN_PRIVILEGES_ONE state, int len, IntPtr prev, IntPtr retLen);
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool CreateProcessAsUserW(IntPtr token, string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string cwd, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool CreateProcessW(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string cwd, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool CreatePipe(out IntPtr read, out IntPtr write, ref SECURITY_ATTRIBUTES sa, int size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetHandleInformation(IntPtr h, uint mask, uint flags);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateFileW(string name, uint access, uint share, ref SECURITY_ATTRIBUTES sa, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateJobObjectW(IntPtr sa, string name);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int cls, ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION info, int len);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateJobObject(IntPtr job, uint code);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)] static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetExitCodeProcess(IntPtr process, out uint code);

    public static string ErrText(int code)
    {
        var sb = new StringBuilder(512);
        int n = FormatMessageW(0x1000 | 0x200, IntPtr.Zero, code, 0, sb, sb.Capacity, IntPtr.Zero);
        return n > 0 ? sb.ToString().Trim() : "unknown error";
    }

    static Exception Fail(string what)
    {
        int e = Marshal.GetLastWin32Error();
        return new Exception(what + " failed: error " + e + " (" + ErrText(e) + ")");
    }

    public static void QuietErrors() { SetErrorMode(0x0001 | 0x0002 | 0x8000); }

    static IntPtr OwnToken()
    {
        IntPtr t;
        if (!OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, out t)) throw Fail("OpenProcessToken");
        return t;
    }

    static IntPtr Info(IntPtr token, int cls)
    {
        int len;
        GetTokenInformation(token, cls, IntPtr.Zero, 0, out len);
        if (len <= 0) throw Fail("GetTokenInformation(size)");
        IntPtr buf = Marshal.AllocHGlobal(len);
        if (!GetTokenInformation(token, cls, buf, len, out len)) { Marshal.FreeHGlobal(buf); throw Fail("GetTokenInformation"); }
        return buf;
    }

    public static string SidString(IntPtr sid)
    {
        IntPtr s;
        if (!ConvertSidToStringSidW(sid, out s)) throw Fail("ConvertSidToStringSid");
        string r = Marshal.PtrToStringUni(s);
        LocalFree(s);
        return r;
    }

    static IntPtr SidPtr(string s)
    {
        IntPtr p;
        if (!ConvertStringSidToSidW(s, out p)) throw Fail("ConvertStringSidToSid(" + s + ")");
        return p;
    }

    public static string ParseSid(string s)
    {
        IntPtr p;
        if (!ConvertStringSidToSidW(s, out p)) { int e = Marshal.GetLastWin32Error(); return "invalid: error " + e + " (" + ErrText(e) + ")"; }
        string back = SidString(p);
        LocalFree(p);
        return "valid, parses back as " + back;
    }

    static List<string[]> GroupList(IntPtr token, int cls)
    {
        var list = new List<string[]>();
        IntPtr buf = Info(token, cls);
        try
        {
            int count = Marshal.ReadInt32(buf);
            for (int i = 0; i < count; i++)
            {
                IntPtr e = IntPtr.Add(buf, IntPtr.Size + i * IntPtr.Size * 2);
                IntPtr sid = Marshal.ReadIntPtr(e);
                uint attr = (uint)Marshal.ReadInt32(IntPtr.Add(e, IntPtr.Size));
                list.Add(new string[] { SidString(sid), attr.ToString("X8") });
            }
        }
        finally { Marshal.FreeHGlobal(buf); }
        return list;
    }

    public static string UserSid()
    {
        IntPtr t = OwnToken();
        try
        {
            IntPtr b = Info(t, TokenUser);
            try { return SidString(Marshal.ReadIntPtr(b)); } finally { Marshal.FreeHGlobal(b); }
        }
        finally { CloseHandle(t); }
    }

    public static string LogonSid()
    {
        IntPtr t = OwnToken();
        try
        {
            foreach (var g in GroupList(t, TokenGroups))
            {
                uint a = Convert.ToUInt32(g[1], 16);
                if ((a & SE_GROUP_LOGON_ID) == SE_GROUP_LOGON_ID) return g[0];
            }
            return null;
        }
        finally { CloseHandle(t); }
    }

    public static string[] RestrictedSidsOf(IntPtr token)
    {
        var l = new List<string>();
        foreach (var g in GroupList(token, TokenRestrictedSids)) l.Add(g[0]);
        return l.ToArray();
    }

    public static string[] Privileges()
    {
        var l = new List<string>();
        IntPtr t = OwnToken();
        try
        {
            IntPtr b = Info(t, TokenPrivileges);
            try
            {
                int count = Marshal.ReadInt32(b);
                for (int i = 0; i < count; i++)
                {
                    IntPtr e = IntPtr.Add(b, 4 + i * 12);
                    var luid = new LUID { LowPart = (uint)Marshal.ReadInt32(e), HighPart = Marshal.ReadInt32(IntPtr.Add(e, 4)) };
                    uint attr = (uint)Marshal.ReadInt32(IntPtr.Add(e, 8));
                    var sb = new StringBuilder(128);
                    int len = sb.Capacity;
                    LookupPrivilegeNameW(null, ref luid, sb, ref len);
                    l.Add(sb.ToString() + ((attr & SE_PRIVILEGE_ENABLED) != 0 ? " (enabled)" : " (disabled)"));
                }
            }
            finally { Marshal.FreeHGlobal(b); }
        }
        finally { CloseHandle(t); }
        return l.ToArray();
    }

    public static string RemovePrivilege(string name)
    {
        LUID luid;
        if (!LookupPrivilegeValueW(null, name, out luid)) return name + ": unknown privilege name";
        IntPtr t = OwnToken();
        try
        {
            var tp = new TOKEN_PRIVILEGES_ONE { Count = 1, Luid = luid, Attributes = SE_PRIVILEGE_REMOVED };
            if (!AdjustTokenPrivileges(t, false, ref tp, Marshal.SizeOf(typeof(TOKEN_PRIVILEGES_ONE)), IntPtr.Zero, IntPtr.Zero))
                return name + ": AdjustTokenPrivileges failed, error " + Marshal.GetLastWin32Error();
            int e = Marshal.GetLastWin32Error();
            return e == 1300 ? name + ": not held by this process token" : name + ": REMOVED from this process token";
        }
        finally { CloseHandle(t); }
    }

    static IntPtr BuildAcl(string[] sids, uint mask)
    {
        int size = 8;
        var ptrs = new List<IntPtr>();
        foreach (var s in sids) { IntPtr p = SidPtr(s); ptrs.Add(p); size += 8 + GetLengthSid(p); }
        size = (size + 3) & ~3;
        IntPtr acl = Marshal.AllocHGlobal(size);
        if (!InitializeAcl(acl, size, 2)) throw Fail("InitializeAcl");
        foreach (var p in ptrs) if (!AddAccessAllowedAce(acl, 2, mask, p)) throw Fail("AddAccessAllowedAce");
        return acl;
    }

    public static IntPtr MakeToken(string[] restrictSids, bool disableMaxPrivilege, string[] defaultDaclSids, string[] tokenDaclSids)
    {
        IntPtr own = OwnToken();
        try
        {
            var arr = new SID_AND_ATTRIBUTES[restrictSids.Length];
            for (int i = 0; i < arr.Length; i++) { arr[i].Sid = SidPtr(restrictSids[i]); arr[i].Attributes = 0; }
            IntPtr nt;
            if (!CreateRestrictedToken(own, disableMaxPrivilege ? DISABLE_MAX_PRIVILEGE : 0, 0, IntPtr.Zero, 0, IntPtr.Zero, (uint)arr.Length, arr, out nt))
                throw Fail("CreateRestrictedToken");
            if (defaultDaclSids != null && defaultDaclSids.Length > 0)
            {
                IntPtr acl = BuildAcl(defaultDaclSids, GENERIC_ALL);
                IntPtr holder = Marshal.AllocHGlobal(IntPtr.Size);
                Marshal.WriteIntPtr(holder, acl);
                if (!SetTokenInformation(nt, TokenDefaultDacl, holder, IntPtr.Size)) throw Fail("SetTokenInformation(TokenDefaultDacl)");
            }
            if (tokenDaclSids != null && tokenDaclSids.Length > 0)
            {
                IntPtr acl = BuildAcl(tokenDaclSids, TOKEN_ALL_ACCESS);
                IntPtr sd = Marshal.AllocHGlobal(64);
                if (!InitializeSecurityDescriptor(sd, 1)) throw Fail("InitializeSecurityDescriptor");
                if (!SetSecurityDescriptorDacl(sd, true, acl, false)) throw Fail("SetSecurityDescriptorDacl");
                if (!SetKernelObjectSecurity(nt, 4, sd)) throw Fail("SetKernelObjectSecurity(token DACL)");
            }
            return nt;
        }
        finally { CloseHandle(own); }
    }

    public static RunResult Run(IntPtr token, string cmdline, string cwd, int timeoutMs)
    {
        var r = new RunResult();
        var sw = Stopwatch.StartNew();
        var sa = new SECURITY_ATTRIBUTES();
        sa.nLength = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES));
        sa.bInheritHandle = true;
        IntPtr rd, wr;
        if (!CreatePipe(out rd, out wr, ref sa, 0)) { r.LaunchError = Fail("CreatePipe").Message; return r; }
        SetHandleInformation(rd, 1, 0);
        IntPtr nul = CreateFileW("NUL", GENERIC_READ, 3, ref sa, 3, 0, IntPtr.Zero);
        var si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdInput = nul;
        si.hStdOutput = wr;
        si.hStdError = wr;
        PROCESS_INFORMATION pi;
        var cmd = new StringBuilder(cmdline, Math.Max(cmdline.Length + 1, 1024));
        uint flags = CREATE_SUSPENDED | CREATE_NO_WINDOW;
        bool ok = token == IntPtr.Zero
            ? CreateProcessW(null, cmd, IntPtr.Zero, IntPtr.Zero, true, flags, IntPtr.Zero, cwd, ref si, out pi)
            : CreateProcessAsUserW(token, null, cmd, IntPtr.Zero, IntPtr.Zero, true, flags, IntPtr.Zero, cwd, ref si, out pi);
        int err = Marshal.GetLastWin32Error();
        CloseHandle(wr);
        if (nul != new IntPtr(-1)) CloseHandle(nul);
        if (!ok)
        {
            CloseHandle(rd);
            r.LaunchError = (token == IntPtr.Zero ? "CreateProcess" : "CreateProcessAsUser") + " failed: error " + err + " (" + ErrText(err) + ")";
            r.Seconds = sw.Elapsed.TotalSeconds;
            return r;
        }
        IntPtr job = CreateJobObjectW(IntPtr.Zero, null);
        if (job != IntPtr.Zero)
        {
            var info = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
            info.Basic.LimitFlags = 0x2000;
            SetInformationJobObject(job, 9, ref info, Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION)));
            r.InJob = AssignProcessToJobObject(job, pi.hProcess);
        }
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        var ms = new MemoryStream();
        var fs = new FileStream(new SafeFileHandle(rd, true), FileAccess.Read, 4096, false);
        var reader = new Thread(() =>
        {
            var buf = new byte[4096];
            try
            {
                int n;
                while ((n = fs.Read(buf, 0, buf.Length)) > 0) { lock (ms) { ms.Write(buf, 0, n); } }
            }
            catch { }
        });
        reader.IsBackground = true;
        reader.Start();
        uint w = WaitForSingleObject(pi.hProcess, (uint)timeoutMs);
        if (w != 0)
        {
            r.TimedOut = true;
            if (job != IntPtr.Zero) TerminateJobObject(job, 0xDEAD);
            TerminateProcess(pi.hProcess, 0xDEAD);
            WaitForSingleObject(pi.hProcess, 5000);
        }
        uint code;
        GetExitCodeProcess(pi.hProcess, out code);
        r.ExitCode = unchecked((int)code);
        if (!reader.Join(15000))
        {
            if (job != IntPtr.Zero) TerminateJobObject(job, 0xDEAD);
            reader.Join(5000);
        }
        if (job != IntPtr.Zero) CloseHandle(job);
        CloseHandle(pi.hProcess);
        byte[] bytes;
        lock (ms) { bytes = ms.ToArray(); }
        r.Output = Encoding.UTF8.GetString(bytes);
        r.Seconds = sw.Elapsed.TotalSeconds;
        return r;
    }
}
