using System;
using System.Collections;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

namespace Yap.NsisSmoke
{
    public sealed class NsisInstallDirectory
    {
        private NsisInstallDirectory(string value)
        {
            Value = value;
        }

        internal string Value { get; }

        public static NsisInstallDirectory Create(string value)
        {
            if (string.IsNullOrEmpty(value))
                throw new ArgumentException("NSIS install directory is required.", nameof(value));
            if (!Path.IsPathFullyQualified(value))
                throw new ArgumentException("NSIS install directory must be absolute.", nameof(value));
            if (value.IndexOfAny(new[] { '"', '\r', '\n', '\0' }) >= 0)
                throw new ArgumentException("NSIS install directory must not contain quotes, CR, LF, or NUL.", nameof(value));
            return new NsisInstallDirectory(value);
        }
    }

    public sealed class LaunchRequest
    {
        private LaunchRequest(
            string executablePath,
            string[] arguments,
            string stdoutPath,
            string stderrPath,
            string workingDirectory,
            IDictionary environment,
            NsisInstallDirectory nsisDirectory)
        {
            ExecutablePath = ValidateExecutablePath(executablePath);
            Arguments = new ReadOnlyCollection<string>(ValidateAndCopyArguments(arguments));
            StdoutPath = ValidateAbsolutePath(stdoutPath, nameof(stdoutPath));
            StderrPath = ValidateAbsolutePath(stderrPath, nameof(stderrPath));
            if (StringComparer.OrdinalIgnoreCase.Equals(
                Path.GetFullPath(StdoutPath),
                Path.GetFullPath(StderrPath)))
            {
                throw new ArgumentException("Standard output and standard error paths must differ.");
            }
            WorkingDirectory = ValidateWorkingDirectory(workingDirectory);
            CopyEnvironment(
                environment,
                out IReadOnlyDictionary<string, string> overrides,
                out IReadOnlySet<string> removals);
            EnvironmentOverrides = overrides;
            EnvironmentRemovals = removals;
            NsisDirectory = nsisDirectory;
        }

        public string ExecutablePath { get; }

        public IReadOnlyList<string> Arguments { get; }

        public string StdoutPath { get; }

        public string StderrPath { get; }

        public string WorkingDirectory { get; }

        public IReadOnlyDictionary<string, string> EnvironmentOverrides { get; }

        public IReadOnlySet<string> EnvironmentRemovals { get; }

        internal NsisInstallDirectory NsisDirectory { get; }

        public static LaunchRequest Create(
            string executablePath,
            string[] arguments,
            string stdoutPath,
            string stderrPath,
            string workingDirectory,
            IDictionary environment)
        {
            LaunchRequest request = new LaunchRequest(
                executablePath,
                arguments,
                stdoutPath,
                stderrPath,
                workingDirectory,
                environment,
                null);
            request.BuildCommandLine();
            return request;
        }

        public static LaunchRequest CreateNsisInstaller(
            string executablePath,
            string[] arguments,
            NsisInstallDirectory installDirectory,
            string stdoutPath,
            string stderrPath,
            string workingDirectory,
            IDictionary environment)
        {
            if (installDirectory == null)
                throw new ArgumentNullException(nameof(installDirectory));
            if (arguments == null)
                throw new ArgumentNullException(nameof(arguments));
            if (arguments.Any(argument =>
                argument != null && argument.StartsWith("/D=", StringComparison.OrdinalIgnoreCase)))
            {
                throw new ArgumentException("NSIS arguments must not already contain /D=.", nameof(arguments));
            }

            LaunchRequest request = new LaunchRequest(
                executablePath,
                arguments,
                stdoutPath,
                stderrPath,
                workingDirectory,
                environment,
                installDirectory);
            request.BuildCommandLine();
            return request;
        }

        internal string BuildCommandLine()
        {
            StringBuilder value = new StringBuilder();
            value.Append('"').Append(ExecutablePath).Append('"');
            foreach (string argument in Arguments)
                value.Append(' ').Append(QuoteNormalArgument(argument));
            if (NsisDirectory != null)
                value.Append(" /D=").Append(NsisDirectory.Value);
            if (value.Length + 1 > 32767)
                throw new ArgumentException("Windows command line exceeds 32,767 UTF-16 characters.");
            return value.ToString();
        }

        internal static string QuoteNormalArgument(string argument)
        {
            if (argument.IndexOf('\0') >= 0)
                throw new ArgumentException("Arguments must not contain NUL.", nameof(argument));
            bool quote = argument.Length == 0 || argument.Any(c => char.IsWhiteSpace(c) || c == '"');
            if (!quote)
                return argument;
            StringBuilder result = new StringBuilder("\"");
            int backslashes = 0;
            foreach (char current in argument)
            {
                if (current == '\\')
                {
                    backslashes++;
                    continue;
                }
                if (current == '"')
                {
                    result.Append('\\', checked(backslashes * 2 + 1));
                    result.Append('"');
                    backslashes = 0;
                    continue;
                }
                result.Append('\\', backslashes);
                result.Append(current);
                backslashes = 0;
            }
            result.Append('\\', checked(backslashes * 2));
            return result.Append('"').ToString();
        }

        private static string ValidateExecutablePath(string value)
        {
            string path = ValidateAbsolutePath(value, nameof(value));
            if (!File.Exists(path))
                throw new ArgumentException("Executable path must identify an existing file.", nameof(value));
            return path;
        }

        private static string ValidateAbsolutePath(string value, string parameterName)
        {
            if (string.IsNullOrEmpty(value) || value.IndexOf('\0') >= 0 || !Path.IsPathFullyQualified(value))
                throw new ArgumentException("Path must be absolute and must not contain NUL.", parameterName);
            try
            {
                Path.GetFullPath(value);
            }
            catch (Exception exception) when (
                exception is ArgumentException ||
                exception is NotSupportedException ||
                exception is PathTooLongException)
            {
                throw new ArgumentException("Path must be valid and absolute.", parameterName, exception);
            }
            return value;
        }

        private static string ValidateWorkingDirectory(string value)
        {
            if (string.IsNullOrEmpty(value))
                return null;
            string path = ValidateAbsolutePath(value, nameof(value));
            if (!Directory.Exists(path))
                throw new ArgumentException("Working directory must exist.", nameof(value));
            return path;
        }

        private static string[] ValidateAndCopyArguments(string[] arguments)
        {
            if (arguments == null)
                throw new ArgumentNullException(nameof(arguments));
            string[] copy = (string[])arguments.Clone();
            foreach (string argument in copy)
            {
                if (argument == null)
                    throw new ArgumentException("Arguments must not contain null.", nameof(arguments));
                if (argument.IndexOf('\0') >= 0)
                    throw new ArgumentException("Arguments must not contain NUL.", nameof(arguments));
            }
            return copy;
        }

        private static void CopyEnvironment(
            IDictionary environment,
            out IReadOnlyDictionary<string, string> overrides,
            out IReadOnlySet<string> removals)
        {
            if (environment == null)
                throw new ArgumentNullException(nameof(environment));

            Dictionary<string, string> overrideValues = new Dictionary<string, string>(
                StringComparer.OrdinalIgnoreCase);
            HashSet<string> removalValues = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            HashSet<string> names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (DictionaryEntry item in environment)
            {
                if (!(item.Key is string name))
                    throw new ArgumentException("Environment names must be strings.", nameof(environment));
                ValidateEnvironmentName(name, nameof(environment));
                if (!names.Add(name))
                    throw new ArgumentException("Environment names must be unique ignoring case.", nameof(environment));

                if (item.Value == null)
                {
                    removalValues.Add(name);
                    continue;
                }
                if (!(item.Value is string value))
                    throw new ArgumentException("Environment values must be strings or null.", nameof(environment));
                if (value.IndexOf('\0') >= 0)
                    throw new ArgumentException("Environment values must not contain NUL.", nameof(environment));
                overrideValues.Add(name, value);
            }

            overrides = new ReadOnlyDictionary<string, string>(overrideValues);
            removals = new ReadOnlyEnvironmentNameSet(removalValues);
        }

        private static void ValidateEnvironmentName(string name, string parameterName)
        {
            if (name.Length == 0 || name[0] == '=' || name.IndexOf('=') >= 0 || name.IndexOf('\0') >= 0)
                throw new ArgumentException("Environment names must be non-empty and must not contain '=' or NUL.", parameterName);
        }

        private sealed class ReadOnlyEnvironmentNameSet : IReadOnlySet<string>
        {
            private readonly HashSet<string> values;

            internal ReadOnlyEnvironmentNameSet(IEnumerable<string> values)
            {
                this.values = new HashSet<string>(values, StringComparer.OrdinalIgnoreCase);
            }

            public int Count => values.Count;

            public bool Contains(string item) => values.Contains(item);

            public bool IsProperSubsetOf(IEnumerable<string> other) => values.IsProperSubsetOf(other);

            public bool IsProperSupersetOf(IEnumerable<string> other) => values.IsProperSupersetOf(other);

            public bool IsSubsetOf(IEnumerable<string> other) => values.IsSubsetOf(other);

            public bool IsSupersetOf(IEnumerable<string> other) => values.IsSupersetOf(other);

            public bool Overlaps(IEnumerable<string> other) => values.Overlaps(other);

            public bool SetEquals(IEnumerable<string> other) => values.SetEquals(other);

            public IEnumerator<string> GetEnumerator() => values.GetEnumerator();

            IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();
        }
    }

    internal static class EnvironmentBlockBuilder
    {
        internal static string BuildBlockText(LaunchRequest request, IEnumerable<string> inherited)
        {
            Dictionary<string, string> values = new Dictionary<string, string>(
                StringComparer.OrdinalIgnoreCase);
            foreach (string entry in inherited)
            {
                int separator = entry.StartsWith("=", StringComparison.Ordinal)
                    ? entry.IndexOf('=', 1)
                    : entry.IndexOf('=');
                if (separator <= 0)
                    throw new InvalidOperationException("Inherited environment entry is malformed.");
                string name = entry.Substring(0, separator);
                if (!values.TryAdd(name, entry.Substring(separator + 1)))
                    throw new InvalidOperationException("Inherited environment contains duplicate names.");
            }
            foreach (string name in request.EnvironmentRemovals)
                values.Remove(name);
            foreach (KeyValuePair<string, string> item in request.EnvironmentOverrides)
            {
                values.Remove(item.Key);
                values.Add(item.Key, item.Value);
            }
            StringBuilder block = new StringBuilder();
            foreach (KeyValuePair<string, string> item in values
                .OrderBy(item => item.Key, StringComparer.OrdinalIgnoreCase)
                .ThenBy(item => item.Key, StringComparer.Ordinal))
                block.Append(item.Key).Append('=').Append(item.Value).Append('\0');
            if (block.Length == 0)
                block.Append('\0');
            return block.Append('\0').ToString();
        }
    }

    public enum ContainedProcessStage
    {
        Redirect,
        CreateJob,
        CreateProcess,
        AssignJob,
        CaptureIdentity,
        Resume,
        Wait,
        Terminate,
        Dispose
    }

    public sealed class RootExitReport
    {
        public bool Exited { get; }

        public uint? ExitCode { get; }

        public long ElapsedMilliseconds { get; }

        internal RootExitReport(bool exited, uint? exitCode, long elapsedMilliseconds)
        {
            Exited = exited;
            ExitCode = exitCode;
            ElapsedMilliseconds = elapsedMilliseconds;
        }
    }

    public sealed class JobQuiescenceReport
    {
        public bool Quiescent => true;

        public int PollIterations { get; }

        public long ElapsedMilliseconds { get; }

        internal JobQuiescenceReport(int pollIterations, long elapsedMilliseconds)
        {
            PollIterations = pollIterations;
            ElapsedMilliseconds = elapsedMilliseconds;
        }
    }

    public sealed class TerminationReport
    {
        public uint RequestedExitCode { get; }

        public RootExitReport RootExit { get; }

        public JobQuiescenceReport Quiescence { get; }

        internal TerminationReport(
            uint requestedExitCode,
            RootExitReport rootExit,
            JobQuiescenceReport quiescence)
        {
            RequestedExitCode = requestedExitCode;
            RootExit = rootExit;
            Quiescence = quiescence;
        }
    }

    public sealed class ContainedProcessException : Exception
    {
        internal ContainedProcessException(
            ContainedProcessStage stage,
            string message,
            int? nativeErrorCode,
            bool cleanupProven,
            IEnumerable<string> cleanupErrors,
            Exception innerException = null)
            : base(message, innerException)
        {
            Stage = stage;
            NativeErrorCode = nativeErrorCode;
            CleanupProven = cleanupProven;
            CleanupErrors = new ReadOnlyCollection<string>(
                (cleanupErrors ?? Array.Empty<string>()).ToArray());
        }

        public ContainedProcessStage Stage { get; }

        public int? NativeErrorCode { get; }

        public bool CleanupProven { get; }

        public IReadOnlyList<string> CleanupErrors { get; }

        internal static ContainedProcessException From(
            ContainedProcessStage stage,
            Exception error,
            LaunchCleanupResult cleanup)
        {
            ContainedProcessException contained = error as ContainedProcessException;
            return new ContainedProcessException(
                contained?.Stage ?? stage,
                contained?.Message ?? error.Message,
                contained?.NativeErrorCode,
                cleanup.CleanupProven,
                cleanup.CleanupErrors,
                error);
        }
    }

    internal static class ContainedProcessFailures
    {
        internal static ContainedProcessException NativeFailure(
            ContainedProcessStage stage,
            string message,
            int? nativeErrorCode)
        {
            return new ContainedProcessException(
                stage,
                message,
                nativeErrorCode,
                false,
                Array.Empty<string>());
        }

        internal static ContainedProcessException LogicalFailure(
            ContainedProcessStage stage,
            string message,
            int? nativeErrorCode = null)
        {
            return new ContainedProcessException(
                stage,
                message,
                nativeErrorCode,
                false,
                Array.Empty<string>());
        }
    }

    internal sealed class LaunchCleanupResult
    {
        internal LaunchCleanupResult(bool cleanupProven, IEnumerable<string> cleanupErrors)
        {
            CleanupProven = cleanupProven;
            CleanupErrors = new ReadOnlyCollection<string>(cleanupErrors.ToArray());
        }

        internal bool CleanupProven { get; }

        internal IReadOnlyList<string> CleanupErrors { get; }
    }

    internal readonly struct NativeCallResult<T>
    {
        private NativeCallResult(bool succeeded, T value, int? errorCode)
        {
            Succeeded = succeeded;
            Value = value;
            ErrorCode = errorCode;
        }

        internal bool Succeeded { get; }

        internal T Value { get; }

        internal int? ErrorCode { get; }

        internal static NativeCallResult<T> Success(T value) =>
            new NativeCallResult<T>(true, value, null);

        internal static NativeCallResult<T> Failure(int errorCode) =>
            new NativeCallResult<T>(false, default(T), errorCode);
    }

    internal static class NativeConstants
    {
        internal const uint WaitObject0 = 0x00000000;
        internal const uint WaitTimeout = 0x00000102;
        internal const uint WaitFailed = 0xFFFFFFFF;
        internal const uint ResumeFailed = 0xFFFFFFFF;
        internal const uint StillActive = 259;
        internal const uint MaximumWaitMilliseconds = 0xFFFFFFFE;
    }

    internal enum NativeAllocationKind
    {
        InheritedHandleArray,
        CommandLine,
        EnvironmentBlock
    }

    internal sealed class CreatedProcessHandles
    {
        internal CreatedProcessHandles(SafeProcessHandle processHandle, SafeThreadHandle threadHandle)
        {
            ProcessHandle = processHandle;
            ThreadHandle = threadHandle;
        }

        internal SafeProcessHandle ProcessHandle { get; }

        internal SafeThreadHandle ThreadHandle { get; }
    }

    internal sealed class ProcessIdentity
    {
        internal ProcessIdentity(uint processId, long creationFileTime, string executablePath)
        {
            ProcessId = processId;
            CreationFileTime = creationFileTime;
            ExecutablePath = executablePath;
        }

        internal uint ProcessId { get; }

        internal long CreationFileTime { get; }

        internal string ExecutablePath { get; }
    }

    internal sealed class SafeProcessHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        internal SafeProcessHandle(IntPtr value) : base(true)
        {
            SetHandle(value);
        }

        internal IntPtr NativeValue => DangerousGetHandle();

        internal void MarkClosed() => SetHandleAsInvalid();

        protected override bool ReleaseHandle() => NativeMethods.CloseHandle(handle);
    }

    internal sealed class SafeThreadHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        internal SafeThreadHandle(IntPtr value) : base(true)
        {
            SetHandle(value);
        }

        internal IntPtr NativeValue => DangerousGetHandle();

        internal void MarkClosed() => SetHandleAsInvalid();

        protected override bool ReleaseHandle() => NativeMethods.CloseHandle(handle);
    }

    internal sealed class SafeJobHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        internal SafeJobHandle(IntPtr value) : base(true)
        {
            SetHandle(value);
        }

        internal IntPtr NativeValue => DangerousGetHandle();

        internal void MarkClosed() => SetHandleAsInvalid();

        protected override bool ReleaseHandle() => NativeMethods.CloseHandle(handle);
    }

    internal sealed class SafeRedirectHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        internal SafeRedirectHandle(IntPtr value) : base(true)
        {
            SetHandle(value);
        }

        internal IntPtr NativeValue => DangerousGetHandle();

        internal void MarkClosed() => SetHandleAsInvalid();

        protected override bool ReleaseHandle() => NativeMethods.CloseHandle(handle);
    }

    internal interface IWindowsProcessApi
    {
        NativeCallResult<SafeRedirectHandle> OpenStandardInput();

        NativeCallResult<SafeRedirectHandle> OpenStandardOutput(string path);

        NativeCallResult<SafeJobHandle> CreateJob();

        NativeCallResult<bool> ConfigureKillOnCloseJob(SafeJobHandle jobHandle);

        NativeCallResult<IntPtr> InitializeAttributeList(int attributeCount);

        NativeCallResult<bool> UpdateHandleList(
            IntPtr attributeList,
            IntPtr inheritedHandleArray,
            int handleCount);

        NativeCallResult<IntPtr> GetEnvironmentStrings();

        NativeCallResult<bool> FreeEnvironmentStrings(IntPtr environment);

        NativeCallResult<CreatedProcessHandles> CreateProcessSuspended(
            LaunchRequest request,
            IntPtr commandLine,
            IntPtr environment,
            IntPtr attributeList,
            SafeRedirectHandle standardInput,
            SafeRedirectHandle standardOutput,
            SafeRedirectHandle standardError);

        NativeCallResult<bool> AssignProcessToJob(
            SafeJobHandle jobHandle,
            SafeProcessHandle processHandle);

        NativeCallResult<bool> IsProcessInJob(
            SafeProcessHandle processHandle,
            SafeJobHandle jobHandle);

        NativeCallResult<uint> GetProcessId(SafeProcessHandle processHandle);

        NativeCallResult<long> GetProcessCreationFileTime(SafeProcessHandle processHandle);

        NativeCallResult<string> QueryProcessImagePath(SafeProcessHandle processHandle);

        NativeCallResult<uint> ResumeThread(SafeThreadHandle threadHandle);

        NativeCallResult<uint> WaitForSingleObject(SafeProcessHandle processHandle, uint milliseconds);

        NativeCallResult<uint> GetExitCode(SafeProcessHandle processHandle);

        NativeCallResult<bool> TerminateProcess(SafeProcessHandle processHandle, uint exitCode);

        NativeCallResult<bool> TerminateJob(SafeJobHandle jobHandle, uint exitCode);

        NativeCallResult<uint> QueryActiveProcessCount(SafeJobHandle jobHandle);

        NativeCallResult<bool> CloseRedirectHandle(SafeRedirectHandle handle);

        NativeCallResult<bool> CloseThreadHandle(SafeThreadHandle handle);

        NativeCallResult<bool> CloseProcessHandle(SafeProcessHandle handle);

        NativeCallResult<bool> CloseJobHandle(SafeJobHandle handle);

        NativeCallResult<bool> ReleaseAttributeList(IntPtr attributeList);

        NativeCallResult<bool> FreeAllocation(IntPtr allocation, NativeAllocationKind kind);

        void NotifyLeaseConstructed();
    }

    internal sealed class NativeWindowsProcessApi : IWindowsProcessApi
    {
        internal static readonly NativeWindowsProcessApi Instance = new NativeWindowsProcessApi();

        private NativeWindowsProcessApi()
        {
        }

        public NativeCallResult<SafeRedirectHandle> OpenStandardInput() =>
            OpenRedirect("NUL", NativeMethods.GenericRead, NativeMethods.OpenExisting);

        public NativeCallResult<SafeRedirectHandle> OpenStandardOutput(string path) =>
            OpenRedirect(path, NativeMethods.GenericWrite, NativeMethods.CreateAlways);

        private static NativeCallResult<SafeRedirectHandle> OpenRedirect(
            string path,
            uint desiredAccess,
            uint creationDisposition)
        {
            NativeMethods.SecurityAttributes security = new NativeMethods.SecurityAttributes
            {
                Length = Marshal.SizeOf<NativeMethods.SecurityAttributes>(),
                InheritHandle = true,
                SecurityDescriptor = IntPtr.Zero
            };
            IntPtr value = NativeMethods.CreateFileW(
                path,
                desiredAccess,
                NativeMethods.FileShareRead | NativeMethods.FileShareWrite,
                ref security,
                creationDisposition,
                NativeMethods.FileAttributeNormal,
                IntPtr.Zero);
            if (value == NativeMethods.InvalidHandleValue)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<SafeRedirectHandle>.Failure(error);
            }
            return NativeCallResult<SafeRedirectHandle>.Success(new SafeRedirectHandle(value));
        }

        public NativeCallResult<SafeJobHandle> CreateJob()
        {
            IntPtr value = NativeMethods.CreateJobObjectW(IntPtr.Zero, null);
            if (value == IntPtr.Zero)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<SafeJobHandle>.Failure(error);
            }
            return NativeCallResult<SafeJobHandle>.Success(new SafeJobHandle(value));
        }

        public NativeCallResult<bool> ConfigureKillOnCloseJob(SafeJobHandle jobHandle)
        {
            NativeMethods.JobObjectExtendedLimitInformation information =
                new NativeMethods.JobObjectExtendedLimitInformation();
            information.BasicLimitInformation.LimitFlags = NativeMethods.JobObjectLimitKillOnJobClose;
            bool succeeded = NativeMethods.SetInformationJobObject(
                jobHandle,
                NativeMethods.JobObjectExtendedLimitInformationClass,
                ref information,
                (uint)Marshal.SizeOf<NativeMethods.JobObjectExtendedLimitInformation>());
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<IntPtr> InitializeAttributeList(int attributeCount)
        {
            IntPtr bytes = IntPtr.Zero;
            bool queried = NativeMethods.InitializeProcThreadAttributeList(
                IntPtr.Zero,
                attributeCount,
                0,
                ref bytes);
            int queryError = queried ? 0 : Marshal.GetLastWin32Error();
            if (bytes == IntPtr.Zero || (!queried && queryError != NativeMethods.ErrorInsufficientBuffer))
                return NativeCallResult<IntPtr>.Failure(queryError == 0 ? NativeMethods.ErrorInvalidParameter : queryError);

            IntPtr value = Marshal.AllocHGlobal(bytes);
            bool initialized = NativeMethods.InitializeProcThreadAttributeList(
                value,
                attributeCount,
                0,
                ref bytes);
            if (!initialized)
            {
                int error = Marshal.GetLastWin32Error();
                Marshal.FreeHGlobal(value);
                return NativeCallResult<IntPtr>.Failure(error);
            }
            return NativeCallResult<IntPtr>.Success(value);
        }

        public NativeCallResult<bool> UpdateHandleList(
            IntPtr attributeList,
            IntPtr inheritedHandleArray,
            int handleCount)
        {
            IntPtr bytes = new IntPtr(checked(handleCount * IntPtr.Size));
            bool succeeded = NativeMethods.UpdateProcThreadAttribute(
                attributeList,
                0,
                new IntPtr(NativeMethods.ProcThreadAttributeHandleList),
                inheritedHandleArray,
                bytes,
                IntPtr.Zero,
                IntPtr.Zero);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<IntPtr> GetEnvironmentStrings()
        {
            IntPtr value = NativeMethods.GetEnvironmentStringsW();
            if (value == IntPtr.Zero)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<IntPtr>.Failure(error);
            }
            return NativeCallResult<IntPtr>.Success(value);
        }

        public NativeCallResult<bool> FreeEnvironmentStrings(IntPtr environment)
        {
            bool succeeded = NativeMethods.FreeEnvironmentStringsW(environment);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<CreatedProcessHandles> CreateProcessSuspended(
            LaunchRequest request,
            IntPtr commandLine,
            IntPtr environment,
            IntPtr attributeList,
            SafeRedirectHandle standardInput,
            SafeRedirectHandle standardOutput,
            SafeRedirectHandle standardError)
        {
            NativeMethods.StartupInfoEx startup = new NativeMethods.StartupInfoEx
            {
                StartupInfo = new NativeMethods.StartupInfo
                {
                    Size = Marshal.SizeOf<NativeMethods.StartupInfoEx>(),
                    Flags = NativeMethods.StartfUseStdHandles,
                    StandardInput = standardInput.NativeValue,
                    StandardOutput = standardOutput.NativeValue,
                    StandardError = standardError.NativeValue
                },
                AttributeList = attributeList
            };
            NativeMethods.ProcessInformation processInformation;
            bool succeeded = NativeMethods.CreateProcessW(
                request.ExecutablePath,
                commandLine,
                IntPtr.Zero,
                IntPtr.Zero,
                true,
                NativeMethods.CreateSuspended |
                    NativeMethods.CreateNoWindow |
                    NativeMethods.ExtendedStartupInfoPresent |
                    NativeMethods.CreateUnicodeEnvironment,
                environment,
                request.WorkingDirectory,
                ref startup,
                out processInformation);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                GC.KeepAlive(standardInput);
                GC.KeepAlive(standardOutput);
                GC.KeepAlive(standardError);
                return NativeCallResult<CreatedProcessHandles>.Failure(error);
            }
            GC.KeepAlive(standardInput);
            GC.KeepAlive(standardOutput);
            GC.KeepAlive(standardError);
            return NativeCallResult<CreatedProcessHandles>.Success(
                new CreatedProcessHandles(
                    new SafeProcessHandle(processInformation.Process),
                    new SafeThreadHandle(processInformation.Thread)));
        }

        public NativeCallResult<bool> AssignProcessToJob(
            SafeJobHandle jobHandle,
            SafeProcessHandle processHandle)
        {
            bool succeeded = NativeMethods.AssignProcessToJobObject(jobHandle, processHandle);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> IsProcessInJob(
            SafeProcessHandle processHandle,
            SafeJobHandle jobHandle)
        {
            bool inJob;
            bool succeeded = NativeMethods.IsProcessInJob(processHandle, jobHandle, out inJob);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            return NativeCallResult<bool>.Success(inJob);
        }

        public NativeCallResult<uint> GetProcessId(SafeProcessHandle processHandle)
        {
            uint value = NativeMethods.GetProcessId(processHandle);
            if (value == 0)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<uint>.Failure(error);
            }
            return NativeCallResult<uint>.Success(value);
        }

        public NativeCallResult<long> GetProcessCreationFileTime(SafeProcessHandle processHandle)
        {
            NativeMethods.FileTime creation;
            NativeMethods.FileTime exit;
            NativeMethods.FileTime kernel;
            NativeMethods.FileTime user;
            bool succeeded = NativeMethods.GetProcessTimes(
                processHandle,
                out creation,
                out exit,
                out kernel,
                out user);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<long>.Failure(error);
            }
            ulong full = ((ulong)creation.HighDateTime << 32) | creation.LowDateTime;
            return NativeCallResult<long>.Success(unchecked((long)full));
        }

        public NativeCallResult<string> QueryProcessImagePath(SafeProcessHandle processHandle)
        {
            int capacity = 260;
            while (capacity <= NativeMethods.MaximumWindowsPath)
            {
                StringBuilder value = new StringBuilder(capacity);
                uint length = (uint)capacity;
                bool succeeded = NativeMethods.QueryFullProcessImageNameW(
                    processHandle,
                    0,
                    value,
                    ref length);
                if (succeeded)
                    return NativeCallResult<string>.Success(value.ToString(0, checked((int)length)));

                int error = Marshal.GetLastWin32Error();
                if (error != NativeMethods.ErrorInsufficientBuffer || capacity == NativeMethods.MaximumWindowsPath)
                    return NativeCallResult<string>.Failure(error);
                capacity = Math.Min(checked(capacity * 2), NativeMethods.MaximumWindowsPath);
            }
            return NativeCallResult<string>.Failure(NativeMethods.ErrorInsufficientBuffer);
        }

        public NativeCallResult<uint> ResumeThread(SafeThreadHandle threadHandle)
        {
            uint value = NativeMethods.ResumeThread(threadHandle);
            if (value == NativeConstants.ResumeFailed)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<uint>.Failure(error);
            }
            return NativeCallResult<uint>.Success(value);
        }

        public NativeCallResult<uint> WaitForSingleObject(
            SafeProcessHandle processHandle,
            uint milliseconds)
        {
            uint value = NativeMethods.WaitForSingleObject(processHandle, milliseconds);
            if (value == NativeConstants.WaitFailed)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<uint>.Failure(error);
            }
            return NativeCallResult<uint>.Success(value);
        }

        public NativeCallResult<uint> GetExitCode(SafeProcessHandle processHandle)
        {
            uint value;
            bool succeeded = NativeMethods.GetExitCodeProcess(processHandle, out value);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<uint>.Failure(error);
            }
            return NativeCallResult<uint>.Success(value);
        }

        public NativeCallResult<bool> TerminateProcess(
            SafeProcessHandle processHandle,
            uint exitCode)
        {
            bool succeeded = NativeMethods.TerminateProcess(processHandle, exitCode);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> TerminateJob(SafeJobHandle jobHandle, uint exitCode)
        {
            bool succeeded = NativeMethods.TerminateJobObject(jobHandle, exitCode);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<uint> QueryActiveProcessCount(SafeJobHandle jobHandle)
        {
            NativeMethods.JobObjectBasicAccountingInformation information;
            bool succeeded = NativeMethods.QueryInformationJobObject(
                jobHandle,
                NativeMethods.JobObjectBasicAccountingInformationClass,
                out information,
                (uint)Marshal.SizeOf<NativeMethods.JobObjectBasicAccountingInformation>(),
                IntPtr.Zero);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<uint>.Failure(error);
            }
            return NativeCallResult<uint>.Success(information.ActiveProcesses);
        }

        public NativeCallResult<bool> CloseRedirectHandle(SafeRedirectHandle handle)
        {
            bool succeeded = NativeMethods.CloseHandle(handle.NativeValue);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            handle.MarkClosed();
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> CloseThreadHandle(SafeThreadHandle handle)
        {
            bool succeeded = NativeMethods.CloseHandle(handle.NativeValue);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            handle.MarkClosed();
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> CloseProcessHandle(SafeProcessHandle handle)
        {
            bool succeeded = NativeMethods.CloseHandle(handle.NativeValue);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            handle.MarkClosed();
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> CloseJobHandle(SafeJobHandle handle)
        {
            bool succeeded = NativeMethods.CloseHandle(handle.NativeValue);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            handle.MarkClosed();
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> ReleaseAttributeList(IntPtr attributeList)
        {
            NativeMethods.DeleteProcThreadAttributeList(attributeList);
            Marshal.FreeHGlobal(attributeList);
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> FreeAllocation(IntPtr allocation, NativeAllocationKind kind)
        {
            Marshal.FreeHGlobal(allocation);
            return NativeCallResult<bool>.Success(true);
        }

        public void NotifyLeaseConstructed()
        {
        }
    }

    internal static class NativeMethods
    {
        internal static readonly IntPtr InvalidHandleValue = new IntPtr(-1);

        internal const int ErrorInvalidParameter = 87;
        internal const int ErrorInsufficientBuffer = 122;
        internal const int MaximumWindowsPath = 32767;
        internal const int JobObjectBasicAccountingInformationClass = 1;
        internal const int JobObjectExtendedLimitInformationClass = 9;
        internal const uint JobObjectLimitKillOnJobClose = 0x00002000;
        internal const int ProcThreadAttributeHandleList = 0x00020002;
        internal const uint GenericRead = 0x80000000;
        internal const uint GenericWrite = 0x40000000;
        internal const uint FileShareRead = 0x00000001;
        internal const uint FileShareWrite = 0x00000002;
        internal const uint CreateAlways = 2;
        internal const uint OpenExisting = 3;
        internal const uint FileAttributeNormal = 0x00000080;
        internal const uint CreateSuspended = 0x00000004;
        internal const uint CreateUnicodeEnvironment = 0x00000400;
        internal const uint CreateNoWindow = 0x08000000;
        internal const uint ExtendedStartupInfoPresent = 0x00080000;
        internal const uint StartfUseStdHandles = 0x00000100;

        [StructLayout(LayoutKind.Sequential)]
        internal struct SecurityAttributes
        {
            internal int Length;
            internal IntPtr SecurityDescriptor;

            [MarshalAs(UnmanagedType.Bool)]
            internal bool InheritHandle;
        }

        [StructLayout(LayoutKind.Sequential)]
        internal struct IoCounters
        {
            internal ulong ReadOperationCount;
            internal ulong WriteOperationCount;
            internal ulong OtherOperationCount;
            internal ulong ReadTransferCount;
            internal ulong WriteTransferCount;
            internal ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        internal struct JobObjectBasicLimitInformation
        {
            internal long PerProcessUserTimeLimit;
            internal long PerJobUserTimeLimit;
            internal uint LimitFlags;
            internal UIntPtr MinimumWorkingSetSize;
            internal UIntPtr MaximumWorkingSetSize;
            internal uint ActiveProcessLimit;
            internal UIntPtr Affinity;
            internal uint PriorityClass;
            internal uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        internal struct JobObjectExtendedLimitInformation
        {
            internal JobObjectBasicLimitInformation BasicLimitInformation;
            internal IoCounters IoInfo;
            internal UIntPtr ProcessMemoryLimit;
            internal UIntPtr JobMemoryLimit;
            internal UIntPtr PeakProcessMemoryUsed;
            internal UIntPtr PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        internal struct StartupInfo
        {
            internal int Size;
            internal IntPtr Reserved;
            internal IntPtr Desktop;
            internal IntPtr Title;
            internal uint X;
            internal uint Y;
            internal uint XSize;
            internal uint YSize;
            internal uint XCountChars;
            internal uint YCountChars;
            internal uint FillAttribute;
            internal uint Flags;
            internal ushort ShowWindow;
            internal ushort Reserved2Size;
            internal IntPtr Reserved2;
            internal IntPtr StandardInput;
            internal IntPtr StandardOutput;
            internal IntPtr StandardError;
        }

        [StructLayout(LayoutKind.Sequential)]
        internal struct StartupInfoEx
        {
            internal StartupInfo StartupInfo;
            internal IntPtr AttributeList;
        }

        [StructLayout(LayoutKind.Sequential)]
        internal struct ProcessInformation
        {
            internal IntPtr Process;
            internal IntPtr Thread;
            internal uint ProcessId;
            internal uint ThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        internal struct FileTime
        {
            internal uint LowDateTime;
            internal uint HighDateTime;
        }

        [StructLayout(LayoutKind.Sequential)]
        internal struct JobObjectBasicAccountingInformation
        {
            internal long TotalUserTime;
            internal long TotalKernelTime;
            internal long ThisPeriodTotalUserTime;
            internal long ThisPeriodTotalKernelTime;
            internal uint TotalPageFaultCount;
            internal uint TotalProcesses;
            internal uint ActiveProcesses;
            internal uint TotalTerminatedProcesses;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        internal static extern IntPtr CreateFileW(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            ref SecurityAttributes securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        internal static extern IntPtr CreateJobObjectW(IntPtr securityAttributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool SetInformationJobObject(
            SafeJobHandle job,
            int informationClass,
            ref JobObjectExtendedLimitInformation information,
            uint informationLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool InitializeProcThreadAttributeList(
            IntPtr attributeList,
            int attributeCount,
            int flags,
            ref IntPtr size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool UpdateProcThreadAttribute(
            IntPtr attributeList,
            uint flags,
            IntPtr attribute,
            IntPtr value,
            IntPtr size,
            IntPtr previousValue,
            IntPtr returnSize);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool CreateProcessW(
            string applicationName,
            IntPtr commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            [MarshalAs(UnmanagedType.Bool)] bool inheritHandles,
            uint creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref StartupInfoEx startupInfo,
            out ProcessInformation processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool AssignProcessToJobObject(
            SafeJobHandle job,
            SafeProcessHandle process);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool IsProcessInJob(
            SafeProcessHandle process,
            SafeJobHandle job,
            [MarshalAs(UnmanagedType.Bool)] out bool result);

        [DllImport("kernel32.dll", SetLastError = true)]
        internal static extern uint GetProcessId(SafeProcessHandle process);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool GetProcessTimes(
            SafeProcessHandle process,
            out FileTime creation,
            out FileTime exit,
            out FileTime kernel,
            out FileTime user);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool QueryFullProcessImageNameW(
            SafeProcessHandle process,
            uint flags,
            StringBuilder executableName,
            ref uint size);

        [DllImport("kernel32.dll", SetLastError = true)]
        internal static extern uint ResumeThread(SafeThreadHandle thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        internal static extern uint WaitForSingleObject(SafeProcessHandle handle, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool GetExitCodeProcess(SafeProcessHandle process, out uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool TerminateProcess(SafeProcessHandle process, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool TerminateJobObject(SafeJobHandle job, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool QueryInformationJobObject(
            SafeJobHandle job,
            int informationClass,
            out JobObjectBasicAccountingInformation information,
            uint informationLength,
            IntPtr returnLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        internal static extern IntPtr GetEnvironmentStringsW();

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool FreeEnvironmentStringsW(IntPtr environment);

        [DllImport("kernel32.dll")]
        internal static extern void DeleteProcThreadAttributeList(IntPtr attributeList);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool CloseHandle(IntPtr handle);
    }

    public sealed class WindowsContainedProcessLauncher
    {
        private readonly IWindowsProcessApi api;

        public WindowsContainedProcessLauncher()
            : this(NativeWindowsProcessApi.Instance)
        {
        }

        internal WindowsContainedProcessLauncher(IWindowsProcessApi api)
        {
            this.api = api ?? throw new ArgumentNullException(nameof(api));
        }

        public ContainedProcessLease Launch(LaunchRequest request)
        {
            if (request == null)
                throw new ArgumentNullException(nameof(request));

            LaunchResources resources = new LaunchResources();
            ContainedProcessStage stage = ContainedProcessStage.Redirect;
            try
            {
                resources.OpenStandardHandles(api, request);
                stage = ContainedProcessStage.CreateJob;
                resources.CreateKillOnCloseJob(api);
                stage = ContainedProcessStage.CreateProcess;
                resources.BuildAttributeList(api);
                resources.PinCommandLine(request.BuildCommandLine());
                resources.PinUnicodeEnvironment(api, request);
                resources.CreateSuspendedProcess(api, request);
                stage = ContainedProcessStage.AssignJob;
                resources.AssignAndVerifyJob(api);
                stage = ContainedProcessStage.CaptureIdentity;
                ProcessIdentity identity = resources.CaptureIdentity(api, request.ExecutablePath);
                stage = ContainedProcessStage.Resume;
                NativeCallResult<uint> resume = api.ResumeThread(resources.ThreadHandle);
                resources.ResumeMayHaveOccurred = true;
                if (!resume.Succeeded)
                {
                    throw ContainedProcessFailures.NativeFailure(
                        stage,
                        "ResumeThread failed.",
                        resume.ErrorCode);
                }
                if (resume.Value != 1)
                {
                    throw ContainedProcessFailures.LogicalFailure(
                        stage,
                        "ResumeThread returned an unexpected suspend count.",
                        null);
                }
                resources.CloseThread(api);
                stage = ContainedProcessStage.Dispose;
                resources.ReleaseLaunchOnlyResources(api);
                return resources.TransferLease(identity, api);
            }
            catch (Exception error)
            {
                LaunchCleanupResult cleanup = resources.CleanupFailedLaunch(
                    api,
                    TimeSpan.FromSeconds(5));
                throw ContainedProcessException.From(stage, error, cleanup);
            }
            finally
            {
                resources.DisposeRemainingResourcesNoThrow(api);
            }
        }
    }

    internal sealed class LaunchResources
    {
        private readonly List<string> releaseFailures = new List<string>();
        private SafeRedirectHandle standardInput;
        private SafeRedirectHandle standardOutput;
        private SafeRedirectHandle standardError;
        private SafeJobHandle jobHandle;
        private SafeProcessHandle processHandle;
        private SafeThreadHandle threadHandle;
        private IntPtr attributeList;
        private IntPtr inheritedHandleArray;
        private IntPtr commandLine;
        private IntPtr environmentBlock;
        private IntPtr inheritedEnvironment;
        private bool assignedToJob;
        private bool leaseTransferred;

        internal SafeThreadHandle ThreadHandle => threadHandle;

        internal bool ResumeMayHaveOccurred { get; set; }

        internal void OpenStandardHandles(IWindowsProcessApi api, LaunchRequest request)
        {
            NativeCallResult<SafeRedirectHandle> input = api.OpenStandardInput();
            if (!input.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.Redirect,
                    "Opening null standard input failed.",
                    input.ErrorCode);
            }
            standardInput = input.Value;

            NativeCallResult<SafeRedirectHandle> output = api.OpenStandardOutput(request.StdoutPath);
            if (!output.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.Redirect,
                    "Opening standard output failed.",
                    output.ErrorCode);
            }
            standardOutput = output.Value;

            NativeCallResult<SafeRedirectHandle> error = api.OpenStandardOutput(request.StderrPath);
            if (!error.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.Redirect,
                    "Opening standard error failed.",
                    error.ErrorCode);
            }
            standardError = error.Value;
        }

        internal void CreateKillOnCloseJob(IWindowsProcessApi api)
        {
            NativeCallResult<SafeJobHandle> create = api.CreateJob();
            if (!create.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CreateJob,
                    "Creating the Job Object failed.",
                    create.ErrorCode);
            }
            jobHandle = create.Value;

            NativeCallResult<bool> configure = api.ConfigureKillOnCloseJob(jobHandle);
            if (!configure.Succeeded || !configure.Value)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CreateJob,
                    "Configuring kill-on-close failed.",
                    configure.ErrorCode);
            }
        }

        internal void BuildAttributeList(IWindowsProcessApi api)
        {
            NativeCallResult<IntPtr> initialize = api.InitializeAttributeList(1);
            if (!initialize.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CreateProcess,
                    "Initializing the process attribute list failed.",
                    initialize.ErrorCode);
            }
            attributeList = initialize.Value;

            inheritedHandleArray = Marshal.AllocHGlobal(checked(3 * IntPtr.Size));
            Marshal.WriteIntPtr(inheritedHandleArray, 0, standardInput.NativeValue);
            Marshal.WriteIntPtr(inheritedHandleArray, IntPtr.Size, standardOutput.NativeValue);
            Marshal.WriteIntPtr(inheritedHandleArray, checked(2 * IntPtr.Size), standardError.NativeValue);
            NativeCallResult<bool> update = api.UpdateHandleList(attributeList, inheritedHandleArray, 3);
            if (!update.Succeeded || !update.Value)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CreateProcess,
                    "Updating the inherited-handle allowlist failed.",
                    update.ErrorCode);
            }
        }

        internal void PinCommandLine(string value)
        {
            try
            {
                commandLine = Marshal.StringToHGlobalUni(value);
            }
            catch (Exception error)
            {
                throw new ContainedProcessException(
                    ContainedProcessStage.CreateProcess,
                    "Allocating the mutable command line failed.",
                    null,
                    false,
                    Array.Empty<string>(),
                    error);
            }
        }

        internal void PinUnicodeEnvironment(IWindowsProcessApi api, LaunchRequest request)
        {
            NativeCallResult<IntPtr> inherited = api.GetEnvironmentStrings();
            if (!inherited.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CreateProcess,
                    "Capturing the inherited environment failed.",
                    inherited.ErrorCode);
            }
            inheritedEnvironment = inherited.Value;

            string[] entries;
            try
            {
                entries = ReadEnvironmentEntries(inheritedEnvironment);
            }
            catch (Exception error)
            {
                throw new ContainedProcessException(
                    ContainedProcessStage.CreateProcess,
                    "The inherited environment block was malformed.",
                    null,
                    false,
                    Array.Empty<string>(),
                    error);
            }

            NativeCallResult<bool> releaseInherited = api.FreeEnvironmentStrings(inheritedEnvironment);
            if (!releaseInherited.Succeeded || !releaseInherited.Value)
            {
                RecordReleaseFailure("Inherited environment block", releaseInherited.ErrorCode);
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CreateProcess,
                    "Releasing the inherited environment block failed.",
                    releaseInherited.ErrorCode);
            }
            inheritedEnvironment = IntPtr.Zero;

            try
            {
                string block = EnvironmentBlockBuilder.BuildBlockText(request, entries);
                environmentBlock = Marshal.StringToHGlobalUni(block);
            }
            catch (ContainedProcessException)
            {
                throw;
            }
            catch (Exception error)
            {
                throw new ContainedProcessException(
                    ContainedProcessStage.CreateProcess,
                    "Building the Unicode environment block failed.",
                    null,
                    false,
                    Array.Empty<string>(),
                    error);
            }
        }

        private static string[] ReadEnvironmentEntries(IntPtr environment)
        {
            const int maximumCharacters = 32767;
            List<string> entries = new List<string>();
            int cursor = 0;
            while (cursor < maximumCharacters)
            {
                int start = cursor;
                while (cursor < maximumCharacters && Marshal.ReadInt16(environment, checked(cursor * 2)) != 0)
                    cursor++;
                if (cursor >= maximumCharacters)
                    throw new InvalidOperationException("Environment terminator was not found.");
                if (cursor == start)
                    return entries.ToArray();
                entries.Add(Marshal.PtrToStringUni(
                    IntPtr.Add(environment, checked(start * 2)),
                    cursor - start));
                cursor++;
            }
            throw new InvalidOperationException("Environment terminator was not found.");
        }

        internal void CreateSuspendedProcess(IWindowsProcessApi api, LaunchRequest request)
        {
            NativeCallResult<CreatedProcessHandles> create = api.CreateProcessSuspended(
                request,
                commandLine,
                environmentBlock,
                attributeList,
                standardInput,
                standardOutput,
                standardError);
            if (!create.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CreateProcess,
                    "CreateProcessW failed.",
                    create.ErrorCode);
            }
            processHandle = create.Value.ProcessHandle;
            threadHandle = create.Value.ThreadHandle;
        }

        internal void AssignAndVerifyJob(IWindowsProcessApi api)
        {
            NativeCallResult<bool> assign = api.AssignProcessToJob(jobHandle, processHandle);
            if (!assign.Succeeded || !assign.Value)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.AssignJob,
                    "Assigning the process to the Job Object failed.",
                    assign.ErrorCode);
            }
            assignedToJob = true;

            NativeCallResult<bool> verify = api.IsProcessInJob(processHandle, jobHandle);
            if (!verify.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.AssignJob,
                    "Verifying Job membership failed.",
                    verify.ErrorCode);
            }
            if (!verify.Value)
            {
                throw ContainedProcessFailures.LogicalFailure(
                    ContainedProcessStage.AssignJob,
                    "The process was not a member of the requested Job Object.");
            }
        }

        internal ProcessIdentity CaptureIdentity(IWindowsProcessApi api, string requestedExecutable)
        {
            NativeCallResult<uint> processId = api.GetProcessId(processHandle);
            if (!processId.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CaptureIdentity,
                    "Capturing the process ID failed.",
                    processId.ErrorCode);
            }
            if (processId.Value == 0)
            {
                throw ContainedProcessFailures.LogicalFailure(
                    ContainedProcessStage.CaptureIdentity,
                    "The retained process handle had no positive process ID.");
            }

            NativeCallResult<long> creation = api.GetProcessCreationFileTime(processHandle);
            if (!creation.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CaptureIdentity,
                    "Capturing the process creation FILETIME failed.",
                    creation.ErrorCode);
            }

            NativeCallResult<string> image = api.QueryProcessImagePath(processHandle);
            if (!image.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.CaptureIdentity,
                    "Capturing the process image path failed.",
                    image.ErrorCode);
            }
            string observed = CanonicalizeExecutablePath(image.Value);
            string requested = CanonicalizeExecutablePath(requestedExecutable);
            if (!StringComparer.OrdinalIgnoreCase.Equals(observed, requested))
            {
                throw ContainedProcessFailures.LogicalFailure(
                    ContainedProcessStage.CaptureIdentity,
                    "The created process image did not match the requested executable.");
            }
            return new ProcessIdentity(processId.Value, creation.Value, observed);
        }

        private static string CanonicalizeExecutablePath(string value)
        {
            string path = value;
            if (path.StartsWith(@"\\?\UNC\", StringComparison.OrdinalIgnoreCase))
                path = @"\\" + path.Substring(8);
            else if (path.StartsWith(@"\\?\", StringComparison.OrdinalIgnoreCase))
                path = path.Substring(4);
            return Path.GetFullPath(path);
        }

        internal void CloseThread(IWindowsProcessApi api)
        {
            if (threadHandle == null)
                return;
            NativeCallResult<bool> close = api.CloseThreadHandle(threadHandle);
            if (!close.Succeeded || !close.Value)
            {
                RecordReleaseFailure("Primary thread handle", close.ErrorCode);
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.Resume,
                    "Closing the primary thread handle failed.",
                    close.ErrorCode);
            }
            threadHandle.Dispose();
            threadHandle = null;
        }

        internal void ReleaseLaunchOnlyResources(IWindowsProcessApi api)
        {
            int failuresBefore = releaseFailures.Count;
            int? firstError = null;
            ReleaseRedirect(api, ref standardInput, "Parent standard-input handle", ref firstError);
            ReleaseRedirect(api, ref standardOutput, "Parent standard-output handle", ref firstError);
            ReleaseRedirect(api, ref standardError, "Parent standard-error handle", ref firstError);
            ReleaseAttributeList(api, ref firstError);
            ReleaseAllocation(
                api,
                ref inheritedHandleArray,
                NativeAllocationKind.InheritedHandleArray,
                "Inherited-handle array",
                ref firstError);
            ReleaseAllocation(
                api,
                ref commandLine,
                NativeAllocationKind.CommandLine,
                "Mutable command-line buffer",
                ref firstError);
            ReleaseAllocation(
                api,
                ref environmentBlock,
                NativeAllocationKind.EnvironmentBlock,
                "Unicode environment buffer",
                ref firstError);
            ReleaseInheritedEnvironment(api, ref firstError);

            if (releaseFailures.Count != failuresBefore)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.Dispose,
                    "Releasing launch-only resources failed.",
                    firstError);
            }
        }

        internal ContainedProcessLease TransferLease(ProcessIdentity identity, IWindowsProcessApi api)
        {
            if (processHandle == null || jobHandle == null)
                throw new InvalidOperationException("Lease resources were incomplete.");
            api.NotifyLeaseConstructed();
            ContainedProcessLease lease = new ContainedProcessLease(
                processHandle,
                jobHandle,
                identity,
                api);
            processHandle = null;
            jobHandle = null;
            leaseTransferred = true;
            return lease;
        }

        internal LaunchCleanupResult CleanupFailedLaunch(
            IWindowsProcessApi api,
            TimeSpan timeout)
        {
            List<string> errors = new List<string>(releaseFailures);
            bool rootExited = processHandle == null;
            bool requiresJobQuiescence = assignedToJob || ResumeMayHaveOccurred;
            bool jobQuiescent = !requiresJobQuiescence;
            Stopwatch timer = Stopwatch.StartNew();

            try
            {
                if (processHandle != null)
                {
                    if (assignedToJob)
                    {
                        NativeCallResult<bool> terminate = api.TerminateJob(jobHandle, 0x59504150);
                        if (!terminate.Succeeded || !terminate.Value)
                            AddCleanupError(errors, "Terminating the failed-launch Job", terminate.ErrorCode);
                    }
                    else
                    {
                        NativeCallResult<bool> terminate = api.TerminateProcess(processHandle, 0x59504150);
                        if (!terminate.Succeeded || !terminate.Value)
                            AddCleanupError(errors, "Terminating the failed-launch process", terminate.ErrorCode);
                    }

                    uint waitMilliseconds = RemainingMilliseconds(timeout, timer);
                    NativeCallResult<uint> wait = api.WaitForSingleObject(processHandle, waitMilliseconds);
                    if (!wait.Succeeded)
                    {
                        AddCleanupError(errors, "Waiting for the failed-launch root", wait.ErrorCode);
                    }
                    else if (wait.Value == NativeConstants.WaitObject0)
                    {
                        rootExited = true;
                    }
                    else if (wait.Value == NativeConstants.WaitTimeout)
                    {
                        errors.Add("Root process did not signal before the failed-launch cleanup deadline.");
                    }
                    else
                    {
                        errors.Add("Root process wait returned an unexpected value during failed-launch cleanup.");
                    }
                }

                if (requiresJobQuiescence)
                    jobQuiescent = WaitForJobQuiescence(api, timeout, timer, errors);
            }
            catch (Exception error)
            {
                errors.Add("Unexpected failed-launch cleanup error: " + error.GetType().FullName + ".");
            }

            int? ignored = null;
            ReleaseThread(api, errors, ref ignored);
            ReleaseRedirect(api, ref standardInput, "Parent standard-input handle", errors, ref ignored);
            ReleaseRedirect(api, ref standardOutput, "Parent standard-output handle", errors, ref ignored);
            ReleaseRedirect(api, ref standardError, "Parent standard-error handle", errors, ref ignored);
            ReleaseAttributeList(api, errors, ref ignored);
            ReleaseAllocation(
                api,
                ref inheritedHandleArray,
                NativeAllocationKind.InheritedHandleArray,
                "Inherited-handle array",
                errors,
                ref ignored);
            ReleaseAllocation(
                api,
                ref commandLine,
                NativeAllocationKind.CommandLine,
                "Mutable command-line buffer",
                errors,
                ref ignored);
            ReleaseAllocation(
                api,
                ref environmentBlock,
                NativeAllocationKind.EnvironmentBlock,
                "Unicode environment buffer",
                errors,
                ref ignored);
            ReleaseInheritedEnvironment(api, errors, ref ignored);
            ReleaseProcess(api, errors);
            ReleaseJob(api, errors);

            bool proven = errors.Count == 0 && rootExited && jobQuiescent;
            return new LaunchCleanupResult(proven, errors);
        }

        private bool WaitForJobQuiescence(
            IWindowsProcessApi api,
            TimeSpan timeout,
            Stopwatch timer,
            List<string> errors)
        {
            while (timer.Elapsed < timeout)
            {
                NativeCallResult<uint> query = api.QueryActiveProcessCount(jobHandle);
                if (!query.Succeeded)
                {
                    AddCleanupError(errors, "Querying failed-launch Job state", query.ErrorCode);
                    return false;
                }
                if (query.Value == 0)
                    return true;
                int sleep = Math.Min(50, RemainingSleepMilliseconds(timeout, timer));
                if (sleep > 0)
                    Thread.Sleep(sleep);
            }
            errors.Add("Job quiescence was not proven before the failed-launch cleanup deadline.");
            return false;
        }

        private static uint RemainingMilliseconds(TimeSpan timeout, Stopwatch timer)
        {
            TimeSpan remaining = timeout - timer.Elapsed;
            if (remaining <= TimeSpan.Zero)
                return 0;
            double milliseconds = Math.Ceiling(remaining.TotalMilliseconds);
            return milliseconds >= NativeConstants.MaximumWaitMilliseconds
                ? NativeConstants.MaximumWaitMilliseconds
                : (uint)milliseconds;
        }

        private static int RemainingSleepMilliseconds(TimeSpan timeout, Stopwatch timer)
        {
            TimeSpan remaining = timeout - timer.Elapsed;
            if (remaining <= TimeSpan.Zero)
                return 0;
            return Math.Max(1, (int)Math.Min(int.MaxValue, Math.Ceiling(remaining.TotalMilliseconds)));
        }

        private static void AddCleanupError(List<string> errors, string operation, int? errorCode)
        {
            errors.Add(errorCode.HasValue
                ? operation + " failed with native error " + errorCode.Value + "."
                : operation + " failed.");
        }

        private void RecordReleaseFailure(string resource, int? errorCode)
        {
            AddCleanupError(releaseFailures, "Releasing " + resource, errorCode);
        }

        private void ReleaseThread(
            IWindowsProcessApi api,
            List<string> errors,
            ref int? firstError)
        {
            if (threadHandle == null)
                return;
            try
            {
                NativeCallResult<bool> close = api.CloseThreadHandle(threadHandle);
                if (close.Succeeded && close.Value)
                {
                    threadHandle.Dispose();
                    threadHandle = null;
                    return;
                }
                if (!firstError.HasValue)
                    firstError = close.ErrorCode;
                AddCleanupError(errors, "Releasing primary thread handle", close.ErrorCode);
            }
            catch (Exception error)
            {
                errors.Add("Releasing primary thread handle threw " + error.GetType().FullName + ".");
            }
        }

        private void ReleaseRedirect(
            IWindowsProcessApi api,
            ref SafeRedirectHandle handle,
            string resource,
            ref int? firstError)
        {
            ReleaseRedirect(api, ref handle, resource, releaseFailures, ref firstError);
        }

        private static void ReleaseRedirect(
            IWindowsProcessApi api,
            ref SafeRedirectHandle handle,
            string resource,
            List<string> errors,
            ref int? firstError)
        {
            if (handle == null)
                return;
            try
            {
                NativeCallResult<bool> close = api.CloseRedirectHandle(handle);
                if (close.Succeeded && close.Value)
                {
                    handle.Dispose();
                    handle = null;
                    return;
                }
                if (!firstError.HasValue)
                    firstError = close.ErrorCode;
                AddCleanupError(errors, "Releasing " + resource, close.ErrorCode);
            }
            catch (Exception error)
            {
                errors.Add("Releasing " + resource + " threw " + error.GetType().FullName + ".");
            }
        }

        private void ReleaseAttributeList(IWindowsProcessApi api, ref int? firstError)
        {
            ReleaseAttributeList(api, releaseFailures, ref firstError);
        }

        private void ReleaseAttributeList(
            IWindowsProcessApi api,
            List<string> errors,
            ref int? firstError)
        {
            if (attributeList == IntPtr.Zero)
                return;
            try
            {
                NativeCallResult<bool> release = api.ReleaseAttributeList(attributeList);
                if (release.Succeeded && release.Value)
                {
                    attributeList = IntPtr.Zero;
                    return;
                }
                if (!firstError.HasValue)
                    firstError = release.ErrorCode;
                AddCleanupError(errors, "Releasing process attribute list", release.ErrorCode);
            }
            catch (Exception error)
            {
                errors.Add("Releasing process attribute list threw " + error.GetType().FullName + ".");
            }
        }

        private void ReleaseAllocation(
            IWindowsProcessApi api,
            ref IntPtr allocation,
            NativeAllocationKind kind,
            string resource,
            ref int? firstError)
        {
            ReleaseAllocation(api, ref allocation, kind, resource, releaseFailures, ref firstError);
        }

        private static void ReleaseAllocation(
            IWindowsProcessApi api,
            ref IntPtr allocation,
            NativeAllocationKind kind,
            string resource,
            List<string> errors,
            ref int? firstError)
        {
            if (allocation == IntPtr.Zero)
                return;
            try
            {
                NativeCallResult<bool> release = api.FreeAllocation(allocation, kind);
                if (release.Succeeded && release.Value)
                {
                    allocation = IntPtr.Zero;
                    return;
                }
                if (!firstError.HasValue)
                    firstError = release.ErrorCode;
                AddCleanupError(errors, "Releasing " + resource, release.ErrorCode);
            }
            catch (Exception error)
            {
                errors.Add("Releasing " + resource + " threw " + error.GetType().FullName + ".");
            }
        }

        private void ReleaseInheritedEnvironment(IWindowsProcessApi api, ref int? firstError)
        {
            ReleaseInheritedEnvironment(api, releaseFailures, ref firstError);
        }

        private void ReleaseInheritedEnvironment(
            IWindowsProcessApi api,
            List<string> errors,
            ref int? firstError)
        {
            if (inheritedEnvironment == IntPtr.Zero)
                return;
            try
            {
                NativeCallResult<bool> release = api.FreeEnvironmentStrings(inheritedEnvironment);
                if (release.Succeeded && release.Value)
                {
                    inheritedEnvironment = IntPtr.Zero;
                    return;
                }
                if (!firstError.HasValue)
                    firstError = release.ErrorCode;
                AddCleanupError(errors, "Releasing inherited environment block", release.ErrorCode);
            }
            catch (Exception error)
            {
                errors.Add("Releasing inherited environment block threw " + error.GetType().FullName + ".");
            }
        }

        private void ReleaseProcess(IWindowsProcessApi api, List<string> errors)
        {
            if (processHandle == null)
                return;
            try
            {
                NativeCallResult<bool> close = api.CloseProcessHandle(processHandle);
                if (close.Succeeded && close.Value)
                {
                    processHandle.Dispose();
                    processHandle = null;
                    return;
                }
                AddCleanupError(errors, "Releasing root process handle", close.ErrorCode);
            }
            catch (Exception error)
            {
                errors.Add("Releasing root process handle threw " + error.GetType().FullName + ".");
            }
        }

        private void ReleaseJob(IWindowsProcessApi api, List<string> errors)
        {
            if (jobHandle == null)
                return;
            try
            {
                NativeCallResult<bool> close = api.CloseJobHandle(jobHandle);
                if (close.Succeeded && close.Value)
                {
                    jobHandle.Dispose();
                    jobHandle = null;
                    return;
                }
                AddCleanupError(errors, "Releasing Job handle", close.ErrorCode);
            }
            catch (Exception error)
            {
                errors.Add("Releasing Job handle threw " + error.GetType().FullName + ".");
            }
        }

        internal void DisposeRemainingResourcesNoThrow(IWindowsProcessApi api)
        {
            if (leaseTransferred)
                return;
            try
            {
                List<string> ignoredErrors = new List<string>();
                int? ignored = null;
                ReleaseThread(api, ignoredErrors, ref ignored);
                ReleaseRedirect(api, ref standardInput, "Parent standard-input handle", ignoredErrors, ref ignored);
                ReleaseRedirect(api, ref standardOutput, "Parent standard-output handle", ignoredErrors, ref ignored);
                ReleaseRedirect(api, ref standardError, "Parent standard-error handle", ignoredErrors, ref ignored);
                ReleaseAttributeList(api, ignoredErrors, ref ignored);
                ReleaseAllocation(api, ref inheritedHandleArray, NativeAllocationKind.InheritedHandleArray, "Inherited-handle array", ignoredErrors, ref ignored);
                ReleaseAllocation(api, ref commandLine, NativeAllocationKind.CommandLine, "Mutable command-line buffer", ignoredErrors, ref ignored);
                ReleaseAllocation(api, ref environmentBlock, NativeAllocationKind.EnvironmentBlock, "Unicode environment buffer", ignoredErrors, ref ignored);
                ReleaseInheritedEnvironment(api, ignoredErrors, ref ignored);
                ReleaseProcess(api, ignoredErrors);
                ReleaseJob(api, ignoredErrors);
            }
            catch
            {
            }
            finally
            {
                try { threadHandle?.Dispose(); } catch { }
                try { standardInput?.Dispose(); } catch { }
                try { standardOutput?.Dispose(); } catch { }
                try { standardError?.Dispose(); } catch { }
                try { processHandle?.Dispose(); } catch { }
                try { jobHandle?.Dispose(); } catch { }
            }
        }
    }

    public sealed class ContainedProcessLease : IDisposable
    {
        private readonly object stateLock = new object();
        private readonly IWindowsProcessApi api;
        private SafeProcessHandle processHandle;
        private SafeJobHandle jobHandle;
        private bool disposed;
        private TerminationReport terminationReport;

        internal ContainedProcessLease(
            SafeProcessHandle processHandle,
            SafeJobHandle jobHandle,
            ProcessIdentity identity,
            IWindowsProcessApi api)
        {
            this.processHandle = processHandle ?? throw new ArgumentNullException(nameof(processHandle));
            this.jobHandle = jobHandle ?? throw new ArgumentNullException(nameof(jobHandle));
            this.api = api ?? throw new ArgumentNullException(nameof(api));
            RootProcessId = identity.ProcessId;
            RootCreationFileTime = identity.CreationFileTime;
            RootExecutablePath = identity.ExecutablePath;
        }

        public uint RootProcessId { get; }

        public long RootCreationFileTime { get; }

        public string RootExecutablePath { get; }

        public RootExitReport WaitForRootExit(TimeSpan timeout)
        {
            lock (stateLock)
            {
                EnsureOpen();
                ValidatePositiveTimeout(timeout);
                return WaitForRootExitCore(timeout);
            }
        }

        public JobQuiescenceReport WaitForQuiescence(TimeSpan timeout)
        {
            lock (stateLock)
            {
                EnsureOpen();
                ValidatePositiveTimeout(timeout);
                return WaitForQuiescenceCore(timeout);
            }
        }

        public TerminationReport TerminateAndWait(uint exitCode, TimeSpan timeout)
        {
            lock (stateLock)
            {
                EnsureOpen();
                ValidatePositiveTimeout(timeout);
                if (terminationReport != null)
                    return terminationReport;

                Stopwatch timer = Stopwatch.StartNew();
                NativeCallResult<bool> terminate = api.TerminateJob(jobHandle, exitCode);
                if (!terminate.Succeeded || !terminate.Value)
                {
                    throw ContainedProcessFailures.NativeFailure(
                        ContainedProcessStage.Terminate,
                        "Job termination failed.",
                        terminate.ErrorCode);
                }

                TimeSpan rootRemaining = Remaining(timeout, timer);
                if (rootRemaining <= TimeSpan.Zero)
                    throw CleanupNotProven("Root process did not signal after Job termination.");
                RootExitReport root = WaitForRootExitCore(rootRemaining);
                if (!root.Exited)
                    throw CleanupNotProven("Root process did not signal after Job termination.");

                TimeSpan quiescenceRemaining = Remaining(timeout, timer);
                if (quiescenceRemaining <= TimeSpan.Zero)
                    throw CleanupNotProven("Job quiescence was not proven after termination.");
                JobQuiescenceReport quiescence = WaitForQuiescenceCore(quiescenceRemaining);
                terminationReport = new TerminationReport(exitCode, root, quiescence);
                return terminationReport;
            }
        }

        private RootExitReport WaitForRootExitCore(TimeSpan timeout)
        {
            Stopwatch timer = Stopwatch.StartNew();
            NativeCallResult<uint> waitCall = api.WaitForSingleObject(
                processHandle,
                ToBoundedMilliseconds(timeout));
            if (!waitCall.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.Wait,
                    "Process wait failed.",
                    waitCall.ErrorCode);
            }
            uint wait = waitCall.Value;
            if (wait == NativeConstants.WaitTimeout)
                return new RootExitReport(false, null, timer.ElapsedMilliseconds);
            if (wait != NativeConstants.WaitObject0)
            {
                throw ContainedProcessFailures.LogicalFailure(
                    ContainedProcessStage.Wait,
                    "Process wait returned an unexpected value.");
            }
            NativeCallResult<uint> exitCode = api.GetExitCode(processHandle);
            if (!exitCode.Succeeded)
            {
                throw ContainedProcessFailures.NativeFailure(
                    ContainedProcessStage.Wait,
                    "Process exit-code query failed.",
                    exitCode.ErrorCode);
            }
            return new RootExitReport(true, exitCode.Value, timer.ElapsedMilliseconds);
        }

        private JobQuiescenceReport WaitForQuiescenceCore(TimeSpan timeout)
        {
            Stopwatch timer = Stopwatch.StartNew();
            int iterations = 0;
            while (timer.Elapsed < timeout)
            {
                iterations++;
                NativeCallResult<uint> active = api.QueryActiveProcessCount(jobHandle);
                if (!active.Succeeded)
                {
                    throw ContainedProcessFailures.NativeFailure(
                        ContainedProcessStage.Wait,
                        "Job state query failed.",
                        active.ErrorCode);
                }
                if (active.Value == 0)
                    return new JobQuiescenceReport(iterations, timer.ElapsedMilliseconds);
                int sleep = Math.Min(50, RemainingSleepMilliseconds(timeout, timer));
                if (sleep > 0)
                    Thread.Sleep(sleep);
            }
            throw new ContainedProcessException(
                ContainedProcessStage.Wait,
                "Job quiescence was not proven before the deadline.",
                null,
                false,
                Array.Empty<string>());
        }

        private static ContainedProcessException CleanupNotProven(string message)
        {
            return new ContainedProcessException(
                ContainedProcessStage.Terminate,
                message,
                null,
                false,
                new[] { message });
        }

        private void EnsureOpen()
        {
            if (disposed)
                throw new ObjectDisposedException(nameof(ContainedProcessLease));
        }

        private static void ValidatePositiveTimeout(TimeSpan timeout)
        {
            if (timeout <= TimeSpan.Zero)
                throw new ArgumentOutOfRangeException(nameof(timeout), "Timeout must be positive.");
        }

        private static uint ToBoundedMilliseconds(TimeSpan timeout)
        {
            double milliseconds = Math.Ceiling(timeout.TotalMilliseconds);
            if (double.IsNaN(milliseconds) || milliseconds <= 0)
                throw new ArgumentOutOfRangeException(nameof(timeout), "Timeout must be positive.");
            return milliseconds >= NativeConstants.MaximumWaitMilliseconds
                ? NativeConstants.MaximumWaitMilliseconds
                : (uint)milliseconds;
        }

        private static TimeSpan Remaining(TimeSpan timeout, Stopwatch timer) => timeout - timer.Elapsed;

        private static int RemainingSleepMilliseconds(TimeSpan timeout, Stopwatch timer)
        {
            TimeSpan remaining = timeout - timer.Elapsed;
            if (remaining <= TimeSpan.Zero)
                return 0;
            return Math.Max(1, (int)Math.Min(int.MaxValue, Math.Ceiling(remaining.TotalMilliseconds)));
        }

        public void Dispose()
        {
            lock (stateLock)
            {
                if (disposed)
                    return;
                disposed = true;
                CloseProcessNoThrow();
                CloseJobNoThrow();
            }
            GC.SuppressFinalize(this);
        }

        private void CloseProcessNoThrow()
        {
            SafeProcessHandle owned = processHandle;
            processHandle = null;
            if (owned == null)
                return;
            try
            {
                NativeCallResult<bool> close = api.CloseProcessHandle(owned);
                if (!close.Succeeded || !close.Value)
                    owned.Dispose();
                else
                    owned.Dispose();
            }
            catch
            {
                try { owned.Dispose(); } catch { }
            }
        }

        private void CloseJobNoThrow()
        {
            SafeJobHandle owned = jobHandle;
            jobHandle = null;
            if (owned == null)
                return;
            try
            {
                NativeCallResult<bool> close = api.CloseJobHandle(owned);
                if (!close.Succeeded || !close.Value)
                    owned.Dispose();
                else
                    owned.Dispose();
            }
            catch
            {
                try { owned.Dispose(); } catch { }
            }
        }
    }
}
