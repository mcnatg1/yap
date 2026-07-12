$script:YapTestTreeSentinelName = ".yap-test-tree-sentinel"
$script:YapTestTreeSentinelContents = "yap-test-owned-tree-v1"

if (-not ("Yap.NsisSmoke.KillOnCloseJob" -as [type])) {
  Add-Type -Language CSharp -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

namespace Yap.NsisSmoke
{
    public sealed class KillOnCloseJob : IDisposable
    {
        private const uint JobObjectLimitKillOnJobClose = 0x00002000;
        private const int JobObjectBasicAccountingInformation = 1;
        private const int JobObjectBasicProcessIdList = 3;
        private const int JobObjectExtendedLimitInformation = 9;
        private const int ErrorMoreData = 234;
        private const uint GenericRead = 0x80000000;
        private const uint GenericWrite = 0x40000000;
        private const uint FileShareRead = 0x00000001;
        private const uint FileShareWrite = 0x00000002;
        private const uint FileShareDelete = 0x00000004;
        private const uint CreateAlways = 2;
        private const uint OpenExisting = 3;
        private const uint FileAttributeNormal = 0x00000080;
        private const uint CreateSuspended = 0x00000004;
        private const uint CreateNoWindow = 0x08000000;
        private const uint ExtendedStartupInfoPresent = 0x00080000;
        private const uint StartfUseShowWindow = 0x00000001;
        private const uint StartfUseStdHandles = 0x00000100;
        private const long ProcThreadAttributeHandleList = 0x00020002;
        private const ushort SwHide = 0;
        private const uint LaunchFailureExitCode = 125;
        private const uint WaitObject0 = 0;
        private const uint WaitTimeout = 0x00000102;
        private const uint WaitFailed = 0xFFFFFFFF;
        private const uint FailureCleanupWaitMilliseconds = 5000;
        private static readonly IntPtr InvalidHandleValue = new IntPtr(-1);
        private IntPtr handle;

        private KillOnCloseJob(IntPtr handle)
        {
            this.handle = handle;
        }

        public static KillOnCloseJob Create()
        {
            IntPtr handle = CreateJobObject(IntPtr.Zero, null);
            if (handle == IntPtr.Zero)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateJobObject failed.");
            KillOnCloseJob job = new KillOnCloseJob(handle);
            try
            {
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION information = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                information.BasicLimitInformation.LimitFlags = JobObjectLimitKillOnJobClose;
                int size = Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION));
                if (!SetInformationJobObject(handle, JobObjectExtendedLimitInformation, ref information, size))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "SetInformationJobObject failed.");
                return job;
            }
            catch
            {
                job.Dispose();
                throw;
            }
        }

        public Process StartProcess(
            string filePath,
            string[] arguments,
            string stdoutPath,
            string stderrPath,
            bool failAssignmentForTest)
        {
            EnsureOpen();
            if (String.IsNullOrWhiteSpace(filePath))
                throw new ArgumentException("Process path must not be empty.", "filePath");
            if (String.IsNullOrWhiteSpace(stdoutPath))
                throw new ArgumentException("Standard-output path must not be empty.", "stdoutPath");
            if (String.IsNullOrWhiteSpace(stderrPath))
                throw new ArgumentException("Standard-error path must not be empty.", "stderrPath");

            SECURITY_ATTRIBUTES inheritable = new SECURITY_ATTRIBUTES();
            inheritable.nLength = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES));
            inheritable.bInheritHandle = true;
            IntPtr stdoutHandle = InvalidHandleValue;
            IntPtr stderrHandle = InvalidHandleValue;
            IntPtr stdinHandle = InvalidHandleValue;
            IntPtr attributeList = IntPtr.Zero;
            IntPtr inheritedHandleList = IntPtr.Zero;
            bool attributeListInitialized = false;
            PROCESS_INFORMATION processInformation = new PROCESS_INFORMATION();
            Process process = null;
            bool created = false;
            bool assigned = false;
            bool resumed = false;
            try
            {
                stdoutHandle = CreateFile(
                    stdoutPath,
                    GenericWrite,
                    FileShareRead | FileShareWrite | FileShareDelete,
                    ref inheritable,
                    CreateAlways,
                    FileAttributeNormal,
                    IntPtr.Zero);
                EnsureValidFileHandle(stdoutHandle, "Opening redirected standard output failed.");

                stderrHandle = CreateFile(
                    stderrPath,
                    GenericWrite,
                    FileShareRead | FileShareWrite | FileShareDelete,
                    ref inheritable,
                    CreateAlways,
                    FileAttributeNormal,
                    IntPtr.Zero);
                EnsureValidFileHandle(stderrHandle, "Opening redirected standard error failed.");

                stdinHandle = CreateFile(
                    "NUL",
                    GenericRead,
                    FileShareRead | FileShareWrite,
                    ref inheritable,
                    OpenExisting,
                    FileAttributeNormal,
                    IntPtr.Zero);
                EnsureValidFileHandle(stdinHandle, "Opening the null standard input failed.");

                IntPtr attributeListSize = IntPtr.Zero;
                InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref attributeListSize);
                if (attributeListSize == IntPtr.Zero)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Sizing the inherited-handle list failed.");
                attributeList = Marshal.AllocHGlobal(attributeListSize);
                if (!InitializeProcThreadAttributeList(attributeList, 1, 0, ref attributeListSize))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Initializing the inherited-handle list failed.");
                attributeListInitialized = true;
                inheritedHandleList = Marshal.AllocHGlobal(IntPtr.Size * 3);
                Marshal.WriteIntPtr(inheritedHandleList, 0, stdinHandle);
                Marshal.WriteIntPtr(inheritedHandleList, IntPtr.Size, stdoutHandle);
                Marshal.WriteIntPtr(inheritedHandleList, IntPtr.Size * 2, stderrHandle);
                if (!UpdateProcThreadAttribute(
                    attributeList,
                    0,
                    new IntPtr(ProcThreadAttributeHandleList),
                    inheritedHandleList,
                    new IntPtr(IntPtr.Size * 3),
                    IntPtr.Zero,
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Restricting inherited process handles failed.");
                }

                STARTUPINFOEX startupInformation = new STARTUPINFOEX();
                startupInformation.StartupInfo.cb = checked((uint)Marshal.SizeOf(typeof(STARTUPINFOEX)));
                startupInformation.StartupInfo.dwFlags = StartfUseShowWindow | StartfUseStdHandles;
                startupInformation.StartupInfo.wShowWindow = SwHide;
                startupInformation.StartupInfo.hStdInput = stdinHandle;
                startupInformation.StartupInfo.hStdOutput = stdoutHandle;
                startupInformation.StartupInfo.hStdError = stderrHandle;
                startupInformation.lpAttributeList = attributeList;

                StringBuilder commandLine = new StringBuilder(BuildCommandLine(filePath, arguments));
                if (!CreateProcess(
                    filePath,
                    commandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    true,
                    CreateSuspended | CreateNoWindow | ExtendedStartupInfoPresent,
                    IntPtr.Zero,
                    null,
                    ref startupInformation,
                    out processInformation))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "CreateProcess failed.");
                }
                created = true;

                if (failAssignmentForTest)
                    throw new InvalidOperationException(
                        "Injected assignment failure before process execution. ProcessId=" +
                        processInformation.dwProcessId + ".");
                if (!AssignProcessToJobObject(handle, processInformation.hProcess))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Assigning the suspended process to the job failed.");
                assigned = true;

                bool isInJob;
                if (!IsProcessInJob(processInformation.hProcess, handle, out isInJob))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Verifying process job membership failed.");
                if (!isInJob)
                    throw new InvalidOperationException("The suspended process was not assigned to the containment job.");

                process = Process.GetProcessById(checked((int)processInformation.dwProcessId));
                // Materialize an independent Process handle before closing the
                // CreateProcess handle returned below.
                IntPtr ignored = process.Handle;
                uint previousSuspendCount = ResumeThread(processInformation.hThread);
                if (previousSuspendCount == UInt32.MaxValue)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Resuming the contained process failed.");
                if (previousSuspendCount != 1)
                    throw new InvalidOperationException(
                        "Contained process had unexpected suspend count " + previousSuspendCount + ".");
                resumed = true;
                Process launched = process;
                process = null;
                return launched;
            }
            catch (Exception launchError)
            {
                string cleanupError = null;
                if (created && !resumed)
                {
                    bool terminated = assigned
                        ? TerminateJobObject(handle, LaunchFailureExitCode)
                        : TerminateProcess(processInformation.hProcess, LaunchFailureExitCode);
                    if (!terminated)
                    {
                        cleanupError = new Win32Exception(Marshal.GetLastWin32Error()).Message;
                    }
                    else
                    {
                        uint waitResult = WaitForSingleObject(
                            processInformation.hProcess,
                            FailureCleanupWaitMilliseconds);
                        if (waitResult == WaitTimeout)
                            cleanupError = "Timed out reaping the suspended process.";
                        else if (waitResult == WaitFailed)
                            cleanupError = new Win32Exception(Marshal.GetLastWin32Error()).Message;
                        else if (waitResult != WaitObject0)
                            cleanupError = "Unexpected process wait result " + waitResult + ".";
                    }
                }
                if (process != null)
                    process.Dispose();
                if (cleanupError != null)
                {
                    throw new InvalidOperationException(
                        "Contained process launch failed and cleanup was not proven: " + cleanupError,
                        launchError);
                }
                throw;
            }
            finally
            {
                if (attributeListInitialized)
                    DeleteProcThreadAttributeList(attributeList);
                if (inheritedHandleList != IntPtr.Zero)
                    Marshal.FreeHGlobal(inheritedHandleList);
                if (attributeList != IntPtr.Zero)
                    Marshal.FreeHGlobal(attributeList);
                CloseValidHandle(processInformation.hThread);
                CloseValidHandle(processInformation.hProcess);
                CloseValidHandle(stdinHandle);
                CloseValidHandle(stderrHandle);
                CloseValidHandle(stdoutHandle);
            }
        }

        public uint ActiveProcessCount
        {
            get
            {
                EnsureOpen();
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION information;
                int size = Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION));
                if (!QueryInformationJobObjectAccounting(
                    handle,
                    JobObjectBasicAccountingInformation,
                    out information,
                    size,
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "QueryInformationJobObject failed.");
                }
                return information.ActiveProcesses;
            }
        }

        public int[] GetProcessIds()
        {
            EnsureOpen();
            int capacity = 16;
            while (true)
            {
                int size = 8 + (capacity * IntPtr.Size);
                IntPtr buffer = Marshal.AllocHGlobal(size);
                try
                {
                    for (int offset = 0; offset < size; offset += 4)
                        Marshal.WriteInt32(buffer, offset, 0);
                    bool succeeded = QueryInformationJobObjectBuffer(
                        handle,
                        JobObjectBasicProcessIdList,
                        buffer,
                        size,
                        IntPtr.Zero);
                    int error = succeeded ? 0 : Marshal.GetLastWin32Error();
                    int assigned = Marshal.ReadInt32(buffer, 0);
                    int count = Marshal.ReadInt32(buffer, 4);
                    if (assigned > capacity || (!succeeded && error == ErrorMoreData))
                    {
                        capacity = Math.Max(capacity * 2, assigned);
                        continue;
                    }
                    if (!succeeded)
                        throw new Win32Exception(error, "QueryInformationJobObject process list failed.");
                    List<int> processIds = new List<int>(count);
                    for (int index = 0; index < count; index++)
                    {
                        long value = Marshal.ReadIntPtr(buffer, 8 + (index * IntPtr.Size)).ToInt64();
                        if (value > 0 && value <= Int32.MaxValue)
                            processIds.Add((int)value);
                    }
                    return processIds.ToArray();
                }
                finally
                {
                    Marshal.FreeHGlobal(buffer);
                }
            }
        }

        public void Terminate(uint exitCode)
        {
            EnsureOpen();
            if (!TerminateJobObject(handle, exitCode))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "TerminateJobObject failed.");
        }

        public void Dispose()
        {
            if (handle == IntPtr.Zero)
                return;
            CloseHandle(handle);
            handle = IntPtr.Zero;
        }

        private void EnsureOpen()
        {
            if (handle == IntPtr.Zero)
                throw new ObjectDisposedException("KillOnCloseJob");
        }

        private static void EnsureValidFileHandle(IntPtr fileHandle, string message)
        {
            if (fileHandle == IntPtr.Zero || fileHandle == InvalidHandleValue)
                throw new Win32Exception(Marshal.GetLastWin32Error(), message);
        }

        private static void CloseValidHandle(IntPtr value)
        {
            if (value != IntPtr.Zero && value != InvalidHandleValue)
                CloseHandle(value);
        }

        private static string BuildCommandLine(string filePath, string[] arguments)
        {
            StringBuilder commandLine = new StringBuilder(QuoteArgument(filePath));
            if (arguments != null)
            {
                foreach (string argument in arguments)
                {
                    commandLine.Append(' ');
                    // Preserve Start-Process's existing ArgumentList contract.
                    // In particular, NSIS requires its final /D= value to be
                    // an unquoted raw command-line tail even when it has spaces.
                    commandLine.Append(argument ?? String.Empty);
                }
            }
            return commandLine.ToString();
        }

        private static string QuoteArgument(string argument)
        {
            bool requiresQuotes = argument.Length == 0;
            for (int index = 0; index < argument.Length && !requiresQuotes; index++)
            {
                char value = argument[index];
                requiresQuotes = Char.IsWhiteSpace(value) || value == '"';
            }
            if (!requiresQuotes)
                return argument;

            StringBuilder quoted = new StringBuilder();
            quoted.Append('"');
            int backslashes = 0;
            foreach (char value in argument)
            {
                if (value == '\\')
                {
                    backslashes++;
                    continue;
                }
                if (value == '"')
                {
                    quoted.Append('\\', (backslashes * 2) + 1);
                    quoted.Append('"');
                    backslashes = 0;
                    continue;
                }
                quoted.Append('\\', backslashes);
                quoted.Append(value);
                backslashes = 0;
            }
            quoted.Append('\\', backslashes * 2);
            quoted.Append('"');
            return quoted.ToString();
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
        {
            public long TotalUserTime;
            public long TotalKernelTime;
            public long ThisPeriodTotalUserTime;
            public long ThisPeriodTotalKernelTime;
            public uint TotalPageFaultCount;
            public uint TotalProcesses;
            public uint ActiveProcesses;
            public uint TotalTerminatedProcesses;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_LIMIT_INFORMATION
        {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct IO_COUNTERS
        {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct SECURITY_ATTRIBUTES
        {
            public int nLength;
            public IntPtr lpSecurityDescriptor;
            [MarshalAs(UnmanagedType.Bool)]
            public bool bInheritHandle;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct STARTUPINFO
        {
            public uint cb;
            public string lpReserved;
            public string lpDesktop;
            public string lpTitle;
            public uint dwX;
            public uint dwY;
            public uint dwXSize;
            public uint dwYSize;
            public uint dwXCountChars;
            public uint dwYCountChars;
            public uint dwFillAttribute;
            public uint dwFlags;
            public ushort wShowWindow;
            public ushort cbReserved2;
            public IntPtr lpReserved2;
            public IntPtr hStdInput;
            public IntPtr hStdOutput;
            public IntPtr hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct STARTUPINFOEX
        {
            public STARTUPINFO StartupInfo;
            public IntPtr lpAttributeList;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct PROCESS_INFORMATION
        {
            public IntPtr hProcess;
            public IntPtr hThread;
            public uint dwProcessId;
            public uint dwThreadId;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateJobObject(IntPtr jobAttributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetInformationJobObject(
            IntPtr job,
            int informationClass,
            ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION information,
            int informationLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool IsProcessInJob(
            IntPtr process,
            IntPtr job,
            [MarshalAs(UnmanagedType.Bool)] out bool result);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateFile(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            ref SECURITY_ATTRIBUTES securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CreateProcess(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            [MarshalAs(UnmanagedType.Bool)] bool inheritHandles,
            uint creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref STARTUPINFOEX startupInfo,
            out PROCESS_INFORMATION processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool InitializeProcThreadAttributeList(
            IntPtr attributeList,
            int attributeCount,
            int flags,
            ref IntPtr size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool UpdateProcThreadAttribute(
            IntPtr attributeList,
            uint flags,
            IntPtr attribute,
            IntPtr value,
            IntPtr size,
            IntPtr previousValue,
            IntPtr returnSize);

        [DllImport("kernel32.dll")]
        private static extern void DeleteProcThreadAttributeList(IntPtr attributeList);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateProcess(IntPtr process, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

        [DllImport("kernel32.dll", EntryPoint = "QueryInformationJobObject", SetLastError = true)]
        private static extern bool QueryInformationJobObjectAccounting(
            IntPtr job,
            int informationClass,
            out JOBOBJECT_BASIC_ACCOUNTING_INFORMATION information,
            int informationLength,
            IntPtr returnLength);

        [DllImport("kernel32.dll", EntryPoint = "QueryInformationJobObject", SetLastError = true)]
        private static extern bool QueryInformationJobObjectBuffer(
            IntPtr job,
            int informationClass,
            IntPtr information,
            int informationLength,
            IntPtr returnLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateJobObject(IntPtr job, uint exitCode);

        [DllImport("kernel32.dll")]
        private static extern bool CloseHandle(IntPtr handle);
    }
}
"@
}

function Assert-SafePathToken {
  param([Parameter(Mandatory)][string]$Token)

  if ($Token -in @(".", "..") -or $Token -notmatch "^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$") {
    throw "Unsafe path token: $Token"
  }
  return $Token
}

function Get-PathRelativeTo {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd("\", "/")
  $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
  $separators = [char[]]@("\", "/")
  $rootParts = @($rootFull.Split($separators, [StringSplitOptions]::RemoveEmptyEntries))
  $candidateParts = @($candidateFull.Split($separators, [StringSplitOptions]::RemoveEmptyEntries))
  $common = 0
  while (
    $common -lt $rootParts.Count -and
    $common -lt $candidateParts.Count -and
    [string]::Equals($rootParts[$common], $candidateParts[$common], [StringComparison]::OrdinalIgnoreCase)
  ) {
    $common++
  }
  if ($common -eq 0) { return $candidateFull }

  $relativeParts = [System.Collections.Generic.List[string]]::new()
  for ($index = $common; $index -lt $rootParts.Count; $index++) { $relativeParts.Add("..") }
  for ($index = $common; $index -lt $candidateParts.Count; $index++) {
    $relativeParts.Add($candidateParts[$index])
  }
  if ($relativeParts.Count -eq 0) { return "." }
  return [string]::Join([System.IO.Path]::DirectorySeparatorChar, $relativeParts)
}

function Test-StrictChildPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $relative = Get-PathRelativeTo -Root $Root -Candidate $Candidate
  if ([string]::IsNullOrWhiteSpace($relative) -or $relative -eq ".") { return $false }
  if ([System.IO.Path]::IsPathRooted($relative)) { return $false }
  $firstSegment = $relative.Split([System.IO.Path]::DirectorySeparatorChar)[0]
  return $firstSegment -ne ".."
}

function Get-ValidatedChildPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Token
  )

  $safeToken = Assert-SafePathToken -Token $Token
  $candidate = [System.IO.Path]::GetFullPath((Join-Path $Root $safeToken))
  if (-not (Test-StrictChildPath -Root $Root -Candidate $candidate)) {
    throw "Path token did not resolve to a strict child of the configured root."
  }
  return $candidate
}

function Get-TauriNsisToolPaths {
  param([Parameter(Mandatory)][string]$Root)

  $rootFull = [System.IO.Path]::GetFullPath($Root)
  if (-not (Test-Path -LiteralPath $rootFull -PathType Container)) {
    throw "Tauri NSIS cache root is missing: $rootFull"
  }
  Assert-NoReparsePoints -Path $rootFull
  $launcherPath = Join-Path $rootFull "makensis.exe"
  $compilerPath = Join-Path $rootFull "Bin\makensis.exe"
  if (-not (Test-Path -LiteralPath $launcherPath -PathType Leaf)) {
    throw "Tauri NSIS launcher is missing: $launcherPath"
  }
  if (-not (Test-Path -LiteralPath $compilerPath -PathType Leaf)) {
    throw "Tauri NSIS compiler is missing: $compilerPath"
  }
  return [pscustomobject]@{
    LauncherPath = $launcherPath
    CompilerPath = $compilerPath
  }
}

function Get-Sha256Hex {
  param([Parameter(Mandatory)][string]$Path)

  $fullPath = [System.IO.Path]::GetFullPath($Path)
  if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) {
    throw "SHA-256 input file does not exist: $fullPath"
  }
  $stream = [System.IO.File]::OpenRead($fullPath)
  $sha256 = [System.Security.Cryptography.SHA256]::Create()
  try {
    return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "")
  } finally {
    $sha256.Dispose()
    $stream.Dispose()
  }
}

function Assert-PathIsNotReparsePoint {
  param([Parameter(Mandatory)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) { return }
  $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
  if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "Reparse point is not allowed in NSIS smoke paths: $($item.FullName)"
  }
}

function Assert-NoReparsePoints {
  param([Parameter(Mandatory)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) { return }
  $pending = [System.Collections.Generic.Stack[string]]::new()
  $pending.Push([System.IO.Path]::GetFullPath($Path))
  while ($pending.Count -gt 0) {
    $current = $pending.Pop()
    $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
      throw "Reparse point is not allowed in NSIS smoke paths: $($item.FullName)"
    }
    if ($item.PSIsContainer) {
      foreach ($child in Get-ChildItem -LiteralPath $item.FullName -Force -ErrorAction Stop) {
        $pending.Push($child.FullName)
      }
    }
  }
}

function Assert-ValidatedTreeOwnership {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [System.IO.Path]::GetFullPath($Root)
  $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
  if (-not (Test-StrictChildPath -Root $rootFull -Candidate $candidateFull)) {
    throw "Refusing recursive deletion outside a strict child of $rootFull."
  }
  $leaf = [System.IO.Path]::GetFileName($candidateFull.TrimEnd("\", "/"))
  if ($leaf -in @("Yap", "com.mcnatg1.yap")) {
    throw "Refusing test cleanup of the production Yap directory: $candidateFull"
  }
  if (-not (Test-Path -LiteralPath $candidateFull -PathType Container)) {
    throw "Test-owned directory does not exist: $candidateFull"
  }
  Assert-NoReparsePoints -Path $candidateFull
  $sentinel = Join-Path $candidateFull $script:YapTestTreeSentinelName
  if (-not (Test-Path -LiteralPath $sentinel -PathType Leaf)) {
    throw "Refusing recursive deletion without the test-data sentinel: $candidateFull"
  }
  if ((Get-Content -LiteralPath $sentinel -Raw).TrimEnd() -cne $script:YapTestTreeSentinelContents) {
    throw "Refusing recursive deletion with an invalid test-data sentinel: $candidateFull"
  }
}

function Initialize-ValidatedTree {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate
  )

  $rootFull = [System.IO.Path]::GetFullPath($Root)
  $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
  if (-not (Test-StrictChildPath -Root $rootFull -Candidate $candidateFull)) {
    throw "Test-owned path must be a strict child of $rootFull."
  }
  $leaf = [System.IO.Path]::GetFileName($candidateFull.TrimEnd("\", "/"))
  if ($leaf -in @("Yap", "com.mcnatg1.yap")) {
    throw "Refusing to initialize the production Yap directory as test data: $candidateFull"
  }
  Assert-PathIsNotReparsePoint -Path $rootFull
  if (Test-Path -LiteralPath $candidateFull) {
    Assert-NoReparsePoints -Path $candidateFull
    $sentinel = Join-Path $candidateFull $script:YapTestTreeSentinelName
    if (Test-Path -LiteralPath $sentinel -PathType Leaf) {
      Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $candidateFull
      return $candidateFull
    }
    if (@(Get-ChildItem -LiteralPath $candidateFull -Force -ErrorAction Stop).Count -gt 0) {
      throw "Refusing to claim a non-empty directory without a test-data sentinel: $candidateFull"
    }
  } else {
    New-Item -ItemType Directory -Force $candidateFull | Out-Null
  }
  Set-Content `
    -LiteralPath (Join-Path $candidateFull $script:YapTestTreeSentinelName) `
    -Value $script:YapTestTreeSentinelContents `
    -Encoding ascii
  Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $candidateFull
  return $candidateFull
}

function Remove-ValidatedTree {
  param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Candidate,
    [scriptblock]$BeforeQuarantineRevalidation = $null
  )

  $rootFull = [System.IO.Path]::GetFullPath($Root)
  $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
  if (-not (Test-Path -LiteralPath $candidateFull)) { return }
  Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $candidateFull
  $leaf = [System.IO.Path]::GetFileName($candidateFull.TrimEnd("\", "/"))
  $quarantineFull = Join-Path ([System.IO.Path]::GetDirectoryName($candidateFull)) ".$leaf.delete-quarantine"
  if (-not (Test-StrictChildPath -Root $rootFull -Candidate $quarantineFull)) {
    throw "Deletion quarantine must remain a strict child of $rootFull."
  }
  if (Test-Path -LiteralPath $quarantineFull) {
    throw "Refusing recursive deletion because the fixed quarantine is not empty: $quarantineFull"
  }
  [System.IO.Directory]::Move($candidateFull, $quarantineFull)
  if ($null -ne $BeforeQuarantineRevalidation) {
    & $BeforeQuarantineRevalidation $quarantineFull
  }
  Assert-ValidatedTreeOwnership -Root $rootFull -Candidate $quarantineFull
  Remove-Item -LiteralPath $quarantineFull -Recurse -Force -ErrorAction Stop
  if (Test-Path -LiteralPath $quarantineFull) {
    throw "Recursive cleanup did not remove the fixed quarantine $quarantineFull."
  }
}

function Test-ProcessAlive {
  param([Parameter(Mandatory)][int]$ProcessId)

  try {
    [void](Get-Process -Id $ProcessId -ErrorAction Stop)
    return $true
  } catch [Microsoft.PowerShell.Commands.ProcessCommandException] {
    return $false
  }
}

function Test-ProcessIdentityAlive {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$ExpectedIdentity
  )

  $liveProcess = $null
  try {
    $liveProcess = Get-Process -Id $ProcessId -ErrorAction Stop
    # Bind the observation to one OS handle before comparing creation time so
    # a later PID reuse cannot turn a completed cleanup into a false failure.
    [void]$liveProcess.Handle
    $actualIdentity = ConvertTo-ProcessCreationIdentity -Timestamp $liveProcess.StartTime
    $normalizedExpected = Normalize-ProcessCreationIdentity -Identity $ExpectedIdentity
    return $actualIdentity -ceq $normalizedExpected
  } catch [Microsoft.PowerShell.Commands.ProcessCommandException], `
      [System.ComponentModel.Win32Exception], `
      [System.InvalidOperationException] {
    return $false
  } finally {
    if ($null -ne $liveProcess) { $liveProcess.Dispose() }
  }
}

function Get-ProcessTreeIds {
  param([Parameter(Mandatory)][int]$RootProcessId)

  $tracked = [System.Collections.Generic.HashSet[int]]::new()
  $identities = @{}
  $reused = [System.Collections.Generic.HashSet[int]]::new()
  [void]$tracked.Add($RootProcessId)
  $snapshot = @(Get-ProcessSnapshot)
  Update-TrackedProcessIds -TrackedProcessIds $tracked -TrackedProcessIdentityById $identities -ReusedProcessIds $reused -Snapshot $snapshot
  return @($tracked | Sort-Object)
}

function Invoke-BoundedProcessSnapshot {
  param(
    [Parameter(Mandatory)][string]$SnapshotScript,
    [Parameter(Mandatory)][double]$TimeoutSeconds
  )

  if ($TimeoutSeconds -le 0) { throw "Process snapshot timeout must be positive." }
  $pipeline = [System.Management.Automation.PowerShell]::Create()
  $asyncResult = $null
  try {
    [void]$pipeline.AddScript($SnapshotScript)
    $asyncResult = $pipeline.BeginInvoke()
    $timeoutMilliseconds = [Math]::Max(1, [int][Math]::Ceiling($TimeoutSeconds * 1000))
    if (-not $asyncResult.AsyncWaitHandle.WaitOne($timeoutMilliseconds)) {
      $pipeline.Stop()
      throw "Process snapshot exceeded the $TimeoutSeconds second deadline."
    }
    $result = @($pipeline.EndInvoke($asyncResult))
    if ($pipeline.HadErrors) {
      $message = @($pipeline.Streams.Error | ForEach-Object { $_.ToString() }) -join "; "
      throw "Process snapshot failed: $message"
    }
    return $result
  } finally {
    if ($null -ne $asyncResult) { $asyncResult.AsyncWaitHandle.Dispose() }
    $pipeline.Dispose()
  }
}

function Get-ProcessSnapshot {
  param(
    [double]$TimeoutSeconds = 10,
    [string]$SnapshotProviderScript = ""
  )

  $operationTimeoutSeconds = [Math]::Max(1, [int][Math]::Ceiling($TimeoutSeconds))
  $snapshotScript = if ([string]::IsNullOrWhiteSpace($SnapshotProviderScript)) {
    @"
Get-CimInstance Win32_Process -OperationTimeoutSec $operationTimeoutSeconds -ErrorAction Stop | ForEach-Object {
  [pscustomobject]@{
    ProcessId = [int]`$_.ProcessId
    ParentProcessId = [int]`$_.ParentProcessId
    ExecutablePath = `$_.ExecutablePath
    CreationDate = `$_.CreationDate
  }
}
"@
  } else {
    $SnapshotProviderScript
  }

  $snapshot = @(Invoke-BoundedProcessSnapshot `
    -SnapshotScript $snapshotScript `
    -TimeoutSeconds $TimeoutSeconds)
  return @(
    $snapshot | ForEach-Object {
      $creationIdentity = if ($_.PSObject.Properties.Name -contains "CreationIdentity") {
        Normalize-ProcessCreationIdentity -Identity ([string]$_.CreationIdentity)
      } elseif ($_.PSObject.Properties.Name -contains "CreationDate" -and $null -ne $_.CreationDate) {
        ConvertTo-ProcessCreationIdentity -Timestamp ([DateTime]$_.CreationDate)
      } else {
        ""
      }
      if ([string]::IsNullOrWhiteSpace($creationIdentity)) {
        throw "Process snapshot omitted creation identity for PID $($_.ProcessId)."
      }
      [pscustomobject]@{
        ProcessId = [int]$_.ProcessId
        ParentProcessId = [int]$_.ParentProcessId
        ExecutablePath = $_.ExecutablePath
        CreationIdentity = $creationIdentity
      }
    }
  )
}

function ConvertTo-ProcessCreationIdentity {
  param([Parameter(Mandatory)][DateTime]$Timestamp)

  $utcTicks = $Timestamp.ToUniversalTime().Ticks
  $milliseconds = [Math]::Floor($utcTicks / [TimeSpan]::TicksPerMillisecond)
  return ([long]$milliseconds).ToString([Globalization.CultureInfo]::InvariantCulture)
}

function Normalize-ProcessCreationIdentity {
  param([string]$Identity)

  if ($Identity -notmatch "^\d+$") { return $Identity }
  $value = [long]$Identity
  $maximumMillisecondIdentity = [long]([DateTime]::MaxValue.Ticks / [TimeSpan]::TicksPerMillisecond)
  if ($value -gt $maximumMillisecondIdentity) {
    $value = [long][Math]::Floor($value / [TimeSpan]::TicksPerMillisecond)
  }
  return $value.ToString([Globalization.CultureInfo]::InvariantCulture)
}

function Stop-VerifiedProcessHandle {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][string]$ExpectedIdentity,
    [Parameter(Mandatory)][AllowEmptyCollection()][System.Collections.Generic.HashSet[int]]$ReusedProcessIds
  )

  $liveProcess = Get-Process -Id $ProcessId -ErrorAction Stop
  try {
    # Materialize one OS handle before identity comparison; Kill then targets that handle, not a reused PID lookup.
    [void]$liveProcess.Handle
    $actualIdentity = ConvertTo-ProcessCreationIdentity -Timestamp $liveProcess.StartTime
    if ($actualIdentity -cne $ExpectedIdentity) {
      [void]$ReusedProcessIds.Add($ProcessId)
      return $false
    }
    $liveProcess.Kill()
    return $true
  } finally {
    $liveProcess.Dispose()
  }
}

function Update-TrackedProcessIds {
  param(
    [Parameter(Mandatory)][System.Collections.Generic.HashSet[int]]$TrackedProcessIds,
    [Parameter(Mandatory)][hashtable]$TrackedProcessIdentityById,
    [Parameter(Mandatory)][AllowEmptyCollection()][System.Collections.Generic.HashSet[int]]$ReusedProcessIds,
    [Parameter(Mandatory)][AllowEmptyCollection()][object[]]$Snapshot
  )

  $changed = $true
  while ($changed) {
    $changed = $false
    foreach ($process in $Snapshot) {
      $processId = [int]$process.ProcessId
      $parentProcessId = [int]$process.ParentProcessId
      $creationIdentity = [string]$process.CreationIdentity
      if ($TrackedProcessIds.Contains($processId)) {
        if (-not $TrackedProcessIdentityById.ContainsKey($processId)) {
          $TrackedProcessIdentityById[$processId] = $creationIdentity
        } elseif ([string]$TrackedProcessIdentityById[$processId] -cne $creationIdentity) {
          [void]$ReusedProcessIds.Add($processId)
        }
        continue
      }
      if (
        $processId -le 0 -or
        $processId -eq $PID -or
        -not $TrackedProcessIds.Contains($parentProcessId) -or
        -not $TrackedProcessIdentityById.ContainsKey($parentProcessId)
      ) {
        continue
      }
      $parent = $Snapshot | Where-Object { [int]$_.ProcessId -eq $parentProcessId } | Select-Object -First 1
      if ($null -ne $parent -and [string]$parent.CreationIdentity -cne [string]$TrackedProcessIdentityById[$parentProcessId]) {
        continue
      }
      [void]$TrackedProcessIds.Add($processId)
      $TrackedProcessIdentityById[$processId] = $creationIdentity
      $changed = $true
    }
  }
}

function Get-TrackedProcessDepth {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][hashtable]$ParentById,
    [Parameter(Mandatory)][System.Collections.Generic.HashSet[int]]$TrackedProcessIds
  )

  $depth = 0
  $current = $ProcessId
  $visited = [System.Collections.Generic.HashSet[int]]::new()
  while ($ParentById.ContainsKey($current) -and $visited.Add($current)) {
    $parent = [int]$ParentById[$current]
    if (-not $TrackedProcessIds.Contains($parent)) { break }
    $depth++
    $current = $parent
  }
  return $depth
}

function ConvertTo-SerializableIdentityMap {
  param([Parameter(Mandatory)][System.Collections.IDictionary]$IdentityById)

  $evidence = [ordered]@{}
  foreach ($processId in @($IdentityById.Keys | Sort-Object)) {
    $evidence[[string]$processId] = [string]$IdentityById[$processId]
  }
  return $evidence
}

function Stop-TrackedProcessesBounded {
  param(
    [Parameter(Mandatory)][int[]]$ProcessIds,
    [double]$TimeoutSeconds = 5,
    [double]$QuiescenceTimeoutSeconds = 2,
    [int]$QuiescencePasses = 2,
    [double]$SnapshotTimeoutSeconds = 10,
    [int]$PollMilliseconds = 50,
    [string]$SnapshotProviderScript = "",
    [hashtable]$ExpectedProcessIdentityById = @{},
    [scriptblock]$ProcessStopper = $null
  )

  if ($TimeoutSeconds -le 0) { throw "Process termination timeout must be positive." }
  if ($QuiescenceTimeoutSeconds -le 0) { throw "Process quiescence timeout must be positive." }
  if ($QuiescencePasses -lt 2) { throw "Process cleanup requires at least two quiescence passes." }
  if ($SnapshotTimeoutSeconds -le 0) { throw "Process snapshot timeout must be positive." }
  if ($PollMilliseconds -lt 0) { throw "Process poll interval must not be negative." }
  $tracked = [System.Collections.Generic.HashSet[int]]::new()
  $identities = @{}
  $reused = [System.Collections.Generic.HashSet[int]]::new()
  foreach ($processId in $ProcessIds) {
    if ($processId -le 0 -or $processId -eq $PID) {
      throw "Invalid process ID for bounded cleanup: $processId"
    }
    [void]$tracked.Add($processId)
    if ($ExpectedProcessIdentityById.ContainsKey($processId)) {
      $identities[$processId] = Normalize-ProcessCreationIdentity `
        -Identity ([string]$ExpectedProcessIdentityById[$processId])
    }
  }
  if ($tracked.Count -eq 0) { throw "Bounded cleanup requires at least one process ID." }

  $terminated = [System.Collections.Generic.HashSet[int]]::new()
  $terminationErrors = [System.Collections.Generic.List[string]]::new()
  $terminationDeadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  $quiescenceDeadline = $null
  $iterations = 0
  $quietPasses = 0
  while ($true) {
    $iterations++
    $snapshot = @(Get-ProcessSnapshot `
      -TimeoutSeconds $SnapshotTimeoutSeconds `
      -SnapshotProviderScript $SnapshotProviderScript)
    Update-TrackedProcessIds -TrackedProcessIds $tracked -TrackedProcessIdentityById $identities -ReusedProcessIds $reused -Snapshot $snapshot
    $alive = @($snapshot | Where-Object {
      $id = [int]$_.ProcessId
      $tracked.Contains($id) -and $identities.ContainsKey($id) -and [string]$_.CreationIdentity -ceq [string]$identities[$id]
    })
    if ($alive.Count -eq 0) {
      if ($null -eq $quiescenceDeadline) {
        $quiescenceDeadline = [DateTime]::UtcNow.AddSeconds($QuiescenceTimeoutSeconds)
      }
      $quietPasses++
    } else {
      $quietPasses = 0
      $quiescenceDeadline = $null
      $parentById = @{}
      foreach ($process in $snapshot) {
        $parentById[[int]$process.ProcessId] = [int]$process.ParentProcessId
      }
      $ordered = @(
        $alive | Sort-Object {
          -(Get-TrackedProcessDepth `
            -ProcessId ([int]$_.ProcessId) `
            -ParentById $parentById `
            -TrackedProcessIds $tracked)
        }
      )
      foreach ($process in $ordered) {
        $processId = [int]$process.ProcessId
        try {
          if ($null -ne $ProcessStopper) {
            & $ProcessStopper $process
            $stopped = $true
          } else {
            $stopped = Stop-VerifiedProcessHandle `
              -ProcessId $processId `
              -ExpectedIdentity ([string]$identities[$processId]) `
              -ReusedProcessIds $reused
          }
          if ($stopped) { [void]$terminated.Add($processId) }
        } catch {
          if (Test-ProcessAlive -ProcessId $processId) {
            $terminationErrors.Add("${processId}:$($_.Exception.Message)")
          }
        }
      }
    }
    $now = [DateTime]::UtcNow
    if (
      $null -ne $quiescenceDeadline -and
      $quietPasses -ge $QuiescencePasses -and
      $now -ge $quiescenceDeadline
    ) {
      return [pscustomobject]@{
        DiscoveredProcessIds = @($tracked | Sort-Object)
        TerminatedProcessIds = @($terminated | Sort-Object)
        ResidualProcessIds = @()
        TerminationErrors = @($terminationErrors)
        Iterations = $iterations
        QuiescencePasses = $quietPasses
        ReusedProcessIds = @($reused | Sort-Object)
        ProcessIdentityById = ConvertTo-SerializableIdentityMap -IdentityById $identities
      }
    }
    if ($null -eq $quiescenceDeadline -and $now -ge $terminationDeadline) { break }
    if ($null -ne $quiescenceDeadline -and $now -ge $quiescenceDeadline) { break }
    if ($PollMilliseconds -gt 0) { Start-Sleep -Milliseconds $PollMilliseconds }
  }

  $snapshot = @(Get-ProcessSnapshot `
    -TimeoutSeconds $SnapshotTimeoutSeconds `
    -SnapshotProviderScript $SnapshotProviderScript)
  Update-TrackedProcessIds -TrackedProcessIds $tracked -TrackedProcessIdentityById $identities -ReusedProcessIds $reused -Snapshot $snapshot
  $residuals = @(
    $snapshot |
      Where-Object {
        $id = [int]$_.ProcessId
        $tracked.Contains($id) -and $identities.ContainsKey($id) -and [string]$_.CreationIdentity -ceq [string]$identities[$id]
      } |
      ForEach-Object { [int]$_.ProcessId } |
      Sort-Object
  )
  $report = [ordered]@{
    discoveredProcessIds = @($tracked | Sort-Object)
    terminatedProcessIds = @($terminated | Sort-Object)
    residualProcessIds = $residuals
    terminationErrors = @($terminationErrors)
    iterations = $iterations
    quiescencePasses = $quietPasses
    reusedProcessIds = @($reused | Sort-Object)
  }
  $phase = if ($null -eq $quiescenceDeadline) { "termination" } else { "quiescence" }
  throw "Process cleanup exceeded its $phase deadline: $($report | ConvertTo-Json -Compress -Depth 4)"
}

function Stop-ProcessTreeBounded {
  param(
    [Parameter(Mandatory)][int]$RootProcessId,
    [int[]]$SeedProcessIds = @(),
    [double]$TimeoutSeconds = 5,
    [double]$QuiescenceTimeoutSeconds = 2,
    [double]$SnapshotTimeoutSeconds = 10,
    [string]$RootProcessIdentity = "",
    [hashtable]$SeedProcessIdentityById = @{}
  )

  $expected = @{}
  foreach ($entry in $SeedProcessIdentityById.GetEnumerator()) {
    $expected[[int]$entry.Key] = Normalize-ProcessCreationIdentity -Identity ([string]$entry.Value)
  }
  if (-not [string]::IsNullOrWhiteSpace($RootProcessIdentity)) {
    $expected[$RootProcessId] = Normalize-ProcessCreationIdentity -Identity $RootProcessIdentity
  }
  return Stop-TrackedProcessesBounded `
    -ProcessIds (@($RootProcessId) + @($SeedProcessIds)) `
    -TimeoutSeconds $TimeoutSeconds `
    -QuiescenceTimeoutSeconds $QuiescenceTimeoutSeconds `
    -SnapshotTimeoutSeconds $SnapshotTimeoutSeconds `
    -ExpectedProcessIdentityById $expected
}

function Start-ProcessWithEnvironment {
  param(
    [Parameter(Mandatory)][string]$FilePath,
    [string[]]$ArgumentList = @(),
    [Parameter(Mandatory)][System.Collections.IDictionary]$Environment,
    [string]$StdoutPath = "",
    [string]$StderrPath = ""
  )

  if (
    -not [string]::IsNullOrWhiteSpace($StdoutPath) -and
    -not [string]::IsNullOrWhiteSpace($StderrPath) -and
    [System.IO.Path]::GetFullPath($StdoutPath) -eq [System.IO.Path]::GetFullPath($StderrPath)
  ) {
    throw "Process stdout and stderr paths must be different."
  }
  $previous = @{}
  try {
    foreach ($entry in $Environment.GetEnumerator()) {
      $key = [string]$entry.Key
      $previous[$key] = [Environment]::GetEnvironmentVariable($key, "Process")
      [Environment]::SetEnvironmentVariable($key, [string]$entry.Value, "Process")
    }
    $parameters = @{
      FilePath = $FilePath
      PassThru = $true
      WindowStyle = "Hidden"
    }
    if ($ArgumentList.Count -gt 0) {
      $parameters.ArgumentList = $ArgumentList
    }
    if (-not [string]::IsNullOrWhiteSpace($StdoutPath)) {
      $parameters.RedirectStandardOutput = $StdoutPath
    }
    if (-not [string]::IsNullOrWhiteSpace($StderrPath)) {
      $parameters.RedirectStandardError = $StderrPath
    }
    return Start-Process @parameters
  } finally {
    foreach ($entry in $Environment.GetEnumerator()) {
      $key = [string]$entry.Key
      [Environment]::SetEnvironmentVariable($key, $previous[$key], "Process")
    }
  }
}

function Enter-SmokeRunLock {
  param(
    [Parameter(Mandatory)][string]$ProductKey,
    [Parameter(Mandatory)][string]$ProfileRoot,
    [scriptblock]$MutexFactory = $null
  )

  $safeProductKey = [regex]::Replace($ProductKey, "[^A-Za-z0-9_.-]", "_")
  if ([string]::IsNullOrWhiteSpace($safeProductKey)) { throw "Smoke-run product key is invalid." }
  $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $mutexName = "Global\Yap.NsisSmoke.$sid.$safeProductKey"
  $mutex = $null
  try {
    $mutex = if ($null -ne $MutexFactory) {
      & $MutexFactory $mutexName
    } else {
      [System.Threading.Mutex]::new($false, $mutexName)
    }
    $owned = $false
    try {
      $owned = $mutex.WaitOne(0)
    } catch [System.Threading.AbandonedMutexException] {
      $owned = $true
    }
    if (-not $owned) {
      $mutex.Dispose()
      throw "Another $ProductKey installer smoke run already owns the isolated test namespace."
    }
    return [pscustomobject]@{ Kind = "GlobalMutex"; Handle = $mutex; Owned = $true; Name = $mutexName }
  } catch [System.UnauthorizedAccessException], [System.Security.SecurityException] {
    if ($null -ne $mutex) { $mutex.Dispose() }
  }

  $profileFull = [System.IO.Path]::GetFullPath($ProfileRoot)
  Assert-PathIsNotReparsePoint -Path $profileFull
  if (-not (Test-Path -LiteralPath $profileFull -PathType Container)) {
    throw "Smoke-run profile-lock root does not exist: $profileFull"
  }
  $lockPath = Join-Path $profileFull ".yap-nsis-smoke-$safeProductKey.lock"
  try {
    $stream = [System.IO.File]::Open(
      $lockPath,
      [System.IO.FileMode]::OpenOrCreate,
      [System.IO.FileAccess]::ReadWrite,
      [System.IO.FileShare]::None
    )
    return [pscustomobject]@{ Kind = "ProfileFile"; Handle = $stream; Owned = $true; Name = $lockPath }
  } catch [System.IO.IOException] {
    throw "Another $ProductKey installer smoke run already owns the isolated test namespace."
  }
}

function Exit-SmokeRunLock {
  param([Parameter(Mandatory)][object]$Lock)

  if ($Lock.Kind -ceq "GlobalMutex" -and $Lock.Owned) {
    $Lock.Handle.ReleaseMutex()
    $Lock.Owned = $false
  }
  $Lock.Handle.Dispose()
}

function Start-JobContainedProcess {
  param(
    [Parameter(Mandatory)][string]$FilePath,
    [string[]]$ArgumentList = @(),
    [Parameter(Mandatory)][string]$StdoutPath,
    [Parameter(Mandatory)][string]$StderrPath,
    [switch]$FailAssignmentForTest
  )

  $job = $null
  $process = $null
  try {
    $resolvedFilePath = if (Test-Path -LiteralPath $FilePath -PathType Leaf) {
      (Get-Item -LiteralPath $FilePath -Force -ErrorAction Stop).FullName
    } else {
      $command = Get-Command -Name $FilePath -CommandType Application -ErrorAction Stop |
        Select-Object -First 1
      if ($null -eq $command -or [string]::IsNullOrWhiteSpace($command.Source)) {
        throw "Executable could not be resolved: $FilePath"
      }
      $command.Source
    }
    $resolvedFilePath = [System.IO.Path]::GetFullPath($resolvedFilePath)
    $job = [Yap.NsisSmoke.KillOnCloseJob]::Create()
    $process = $job.StartProcess(
      $resolvedFilePath,
      [string[]]@($ArgumentList),
      [System.IO.Path]::GetFullPath($StdoutPath),
      [System.IO.Path]::GetFullPath($StderrPath),
      $FailAssignmentForTest.IsPresent
    )
    return [pscustomobject]@{ Job = $job; Process = $process }
  } catch {
    $startError = $_.Exception
    if ($null -ne $job) {
      try { $job.Terminate(125) } catch {}
      $job.Dispose()
    }
    if ($null -ne $process) {
      if ($process.WaitForExit(2000)) { $process.WaitForExit() }
      $process.Dispose()
    }
    throw $startError
  }
}

function Stop-JobContainedProcess {
  param(
    [Parameter(Mandatory)][Yap.NsisSmoke.KillOnCloseJob]$Job,
    [Parameter(Mandatory)][System.Diagnostics.Process]$Process,
    [Parameter(Mandatory)][AllowEmptyCollection()][System.Collections.Generic.HashSet[int]]$DiscoveredProcessIds,
    [Parameter(Mandatory)][AllowEmptyCollection()][System.Collections.Generic.HashSet[int]]$ReusedProcessIds,
    [double]$TimeoutSeconds = 5,
    [int]$PollMilliseconds = 50
  )

  $terminated = @($Job.GetProcessIds() | Sort-Object -Unique)
  foreach ($processId in $terminated) { [void]$DiscoveredProcessIds.Add([int]$processId) }
  $Job.Terminate(1)
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  $quietPasses = 0
  $iterations = 0
  do {
    $iterations++
    $active = [int]$Job.ActiveProcessCount
    foreach ($processId in $Job.GetProcessIds()) {
      [void]$DiscoveredProcessIds.Add([int]$processId)
    }
    if ($active -eq 0) {
      $quietPasses++
      if ($quietPasses -ge 2) {
        # Job emptiness proves every inheriting process released redirected handles.
        $Process.WaitForExit()
        return [pscustomobject]@{
          DiscoveredProcessIds = @($DiscoveredProcessIds | Sort-Object)
          TerminatedProcessIds = $terminated
          ResidualProcessIds = @()
          TerminationErrors = @()
          Iterations = $iterations
          QuiescencePasses = $quietPasses
          ReusedProcessIds = @($ReusedProcessIds | Sort-Object)
        }
      }
    } else {
      $quietPasses = 0
    }
    if ($PollMilliseconds -gt 0) { Start-Sleep -Milliseconds $PollMilliseconds }
  } while ([DateTime]::UtcNow -lt $deadline)

  $residuals = @($Job.GetProcessIds() | Sort-Object -Unique)
  $report = [ordered]@{
    discoveredProcessIds = @($DiscoveredProcessIds | Sort-Object)
    terminatedProcessIds = $terminated
    residualProcessIds = $residuals
    terminationErrors = @()
    iterations = $iterations
    quiescencePasses = $quietPasses
    reusedProcessIds = @($ReusedProcessIds | Sort-Object)
  }
  throw "Job cleanup exceeded its $TimeoutSeconds second deadline: $($report | ConvertTo-Json -Compress -Depth 4)"
}

function Invoke-ProcessWithDeadline {
  param(
    [Parameter(Mandatory)][string]$FilePath,
    [string[]]$ArgumentList = @(),
    [Parameter(Mandatory)][double]$TimeoutSeconds,
    [Parameter(Mandatory)][string]$StdoutPath,
    [Parameter(Mandatory)][string]$StderrPath,
    [double]$QuiescenceTimeoutSeconds = 2,
    [double]$SnapshotTimeoutSeconds = 10,
    [int]$PollMilliseconds = 50,
    [string]$SnapshotProviderScript = ""
  )

  if ($TimeoutSeconds -le 0) { throw "Process timeout must be positive." }
  if ($QuiescenceTimeoutSeconds -le 0) { throw "Process quiescence timeout must be positive." }
  if ($SnapshotTimeoutSeconds -le 0) { throw "Process snapshot timeout must be positive." }
  if ($PollMilliseconds -lt 0) { throw "Process poll interval must not be negative." }
  if ([System.IO.Path]::GetFullPath($StdoutPath) -eq [System.IO.Path]::GetFullPath($StderrPath)) {
    throw "Process stdout and stderr paths must be different."
  }
  New-Item -ItemType Directory -Force ([System.IO.Path]::GetDirectoryName($StdoutPath)) | Out-Null
  New-Item -ItemType Directory -Force ([System.IO.Path]::GetDirectoryName($StderrPath)) | Out-Null

  $context = Start-JobContainedProcess `
    -FilePath $FilePath `
    -ArgumentList $ArgumentList `
    -StdoutPath $StdoutPath `
    -StderrPath $StderrPath
  $job = $context.Job
  $process = $context.Process
  try {
    $startedAt = [DateTime]::UtcNow
    $runtimeDeadline = $startedAt.AddSeconds($TimeoutSeconds)
    $quiescenceDeadline = $null
    $tracked = [System.Collections.Generic.HashSet[int]]::new()
    $trackedIdentities = @{}
    $reusedProcessIds = [System.Collections.Generic.HashSet[int]]::new()
    [void]$tracked.Add($process.Id)
    $trackedIdentities[$process.Id] = ConvertTo-ProcessCreationIdentity -Timestamp $process.StartTime
    foreach ($processId in $job.GetProcessIds()) { [void]$tracked.Add([int]$processId) }

    $monitorError = $null
    try {
      $snapshot = @(Get-ProcessSnapshot `
        -TimeoutSeconds $SnapshotTimeoutSeconds `
        -SnapshotProviderScript $SnapshotProviderScript)
      Update-TrackedProcessIds `
        -TrackedProcessIds $tracked `
        -TrackedProcessIdentityById $trackedIdentities `
        -ReusedProcessIds $reusedProcessIds `
        -Snapshot $snapshot
    } catch {
      $monitorError = $_.Exception
    }

    $quietPasses = 0
    $iterations = 0
    $completed = $false
    while ($null -eq $monitorError) {
      $iterations++
      try {
        foreach ($processId in $job.GetProcessIds()) { [void]$tracked.Add([int]$processId) }
        $active = [int]$job.ActiveProcessCount
      } catch {
        $monitorError = $_.Exception
        break
      }
      $now = [DateTime]::UtcNow
      if ($active -eq 0) {
        if ($now -gt $runtimeDeadline) { break }
        if ($null -eq $quiescenceDeadline) {
          $quiescenceDeadline = $now.AddSeconds($QuiescenceTimeoutSeconds)
        }
        $quietPasses++
        if ($quietPasses -ge 2 -and $now -ge $quiescenceDeadline) {
          $completed = $true
          break
        }
      } else {
        $quietPasses = 0
        $quiescenceDeadline = $null
        if ($now -ge $runtimeDeadline) { break }
      }
      if ($PollMilliseconds -gt 0) { Start-Sleep -Milliseconds $PollMilliseconds }
    }

    if (-not $completed) {
      $cleanupEvidence = "cleanup was not attempted"
      $cleanupSucceeded = $false
      try {
        $cleanup = Stop-JobContainedProcess `
          -Job $job `
          -Process $process `
          -DiscoveredProcessIds $tracked `
          -ReusedProcessIds $reusedProcessIds `
          -TimeoutSeconds 5 `
          -PollMilliseconds $PollMilliseconds
        $cleanupSucceeded = $true
        $cleanupEvidence = [ordered]@{
          discoveredProcessIds = @($cleanup.DiscoveredProcessIds)
          terminatedProcessIds = @($cleanup.TerminatedProcessIds)
          residualProcessIds = @($cleanup.ResidualProcessIds)
          terminationErrors = @($cleanup.TerminationErrors)
          iterations = $cleanup.Iterations
          quiescencePasses = $cleanup.QuiescencePasses
          reusedProcessIds = @($cleanup.ReusedProcessIds)
        } | ConvertTo-Json -Compress -Depth 4
      } catch {
        $cleanupEvidence = $_.Exception.Message
      }
      if (-not $cleanupSucceeded) {
        $job.Dispose()
        $job = $null
        if ($process.WaitForExit(1000)) { $process.WaitForExit() }
      }
      if ($null -ne $monitorError) {
        throw "Process monitoring failed: $($monitorError.Message) Cleanup evidence: $cleanupEvidence"
      }
      throw "Process $($process.Id) or its descendants exceeded the $TimeoutSeconds second deadline during runtime. Cleanup evidence: $cleanupEvidence"
    }

    $process.WaitForExit()
    return [pscustomobject]@{
      ProcessId = $process.Id
      ProcessIds = @($tracked | Sort-Object)
      ProcessIdentityById = ConvertTo-SerializableIdentityMap -IdentityById $trackedIdentities
      ReusedProcessIds = @($reusedProcessIds | Sort-Object)
      ExitCode = $process.ExitCode
      DurationMs = [int]([DateTime]::UtcNow - $startedAt).TotalMilliseconds
      DiscoveryIterations = $iterations
      QuiescencePasses = $quietPasses
      ResidualProcessIds = @()
    }
  } finally {
    $process.Dispose()
    if ($null -ne $job) { $job.Dispose() }
  }
}

function Assert-ProcessSurvives {
  param(
    [Parameter(Mandatory)][int]$ProcessId,
    [Parameter(Mandatory)][double]$DurationSeconds
  )

  if ($DurationSeconds -le 0) { throw "Process survival duration must be positive." }
  $deadline = [DateTime]::UtcNow.AddSeconds($DurationSeconds)
  while ([DateTime]::UtcNow -lt $deadline) {
    if (-not (Test-ProcessAlive -ProcessId $ProcessId)) {
      throw "Process $ProcessId exited before the $DurationSeconds second launch probe completed."
    }
    Start-Sleep -Milliseconds 50
  }
}

function Wait-PathAbsent {
  param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][double]$TimeoutSeconds
  )

  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  while ((Test-Path -LiteralPath $Path) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
  }
  if (Test-Path -LiteralPath $Path) {
    throw "Path remained after the $TimeoutSeconds second deadline: $Path"
  }
}

function Get-ProcessesUnderPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [double]$SnapshotTimeoutSeconds = 10,
    [string]$SnapshotProviderScript = ""
  )

  $matches = @()
  foreach ($process in Get-ProcessSnapshot `
    -TimeoutSeconds $SnapshotTimeoutSeconds `
    -SnapshotProviderScript $SnapshotProviderScript) {
    if ([string]::IsNullOrWhiteSpace($process.ExecutablePath)) { continue }
    if (Test-StrictChildPath -Root $Root -Candidate $process.ExecutablePath) {
      $matches += [pscustomobject]@{
        ProcessId = [int]$process.ProcessId
        ExecutablePath = $process.ExecutablePath
      }
    }
  }
  return $matches
}

function Assert-NoProcessesUnderPath {
  param(
    [Parameter(Mandatory)][string]$Root,
    [double]$SnapshotTimeoutSeconds = 10,
    [string]$SnapshotProviderScript = ""
  )

  $matches = @(Get-ProcessesUnderPath `
    -Root $Root `
    -SnapshotTimeoutSeconds $SnapshotTimeoutSeconds `
    -SnapshotProviderScript $SnapshotProviderScript)
  if ($matches.Count -gt 0) {
    $footprint = $matches | ForEach-Object { "$($_.ProcessId):$($_.ExecutablePath)" }
    throw "Processes remain under the install root: $($footprint -join ', ')."
  }
}

Export-ModuleMember -Function `
  Assert-NoProcessesUnderPath, `
  Assert-NoReparsePoints, `
  Assert-PathIsNotReparsePoint, `
  Assert-ProcessSurvives, `
  Assert-SafePathToken, `
  Enter-SmokeRunLock, `
  Exit-SmokeRunLock, `
  Get-ProcessesUnderPath, `
  Get-ProcessTreeIds, `
  Get-Sha256Hex, `
  Get-TauriNsisToolPaths, `
  Get-ValidatedChildPath, `
  Initialize-ValidatedTree, `
  Invoke-ProcessWithDeadline, `
  Remove-ValidatedTree, `
  Start-ProcessWithEnvironment, `
  Stop-ProcessTreeBounded, `
  Stop-TrackedProcessesBounded, `
  Test-ProcessAlive, `
  Test-ProcessIdentityAlive, `
  Test-StrictChildPath, `
  Wait-PathAbsent
