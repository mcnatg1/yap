using System;
using System.Collections;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

namespace Yap.NsisSmoke.Testing
{
    public static class LaunchRequestProbe
    {
        public static string BuildCommandLine(LaunchRequest request) => request.BuildCommandLine();

        public static string BuildEnvironmentBlockText(LaunchRequest request, string[] inherited) =>
            EnvironmentBlockBuilder.BuildBlockText(request, inherited);
    }

    public enum ScriptedFailurePoint
    {
        None,
        OpenStdin,
        OpenStdout,
        OpenStderr,
        CreateJob,
        ConfigureJob,
        InitializeAttributeList,
        UpdateAttributeList,
        CaptureEnvironment,
        CreateProcess,
        PostCreateOwnershipHandoff,
        AssignJob,
        CaptureIdentity,
        ResumeThread,
        ReleaseParentStdin,
        ReleaseParentStdout,
        ReleaseParentStderr,
        ReleaseThread,
        ReleaseAttributeList,
        ReleaseInheritedHandleArray,
        ReleaseCommandBuffer,
        ReleaseEnvironmentBuffer
    }

    public enum InjectedFailurePoint
    {
        CreateProcess,
        AssignJob,
        CaptureIdentity,
        ResumeThread
    }

    public sealed class ScriptedNativeScenario
    {
        private static int nextProcessId = 40000;
        private readonly object sync = new object();
        private readonly List<string> operationLog = new List<string>();
        private readonly Dictionary<IntPtr, string> openHandles = new Dictionary<IntPtr, string>();
        private readonly HashSet<ScriptedFailurePoint> consumedReleaseFailures =
            new HashSet<ScriptedFailurePoint>();
        private int resumeThreadCallCount;
        private int jobTerminationCount;
        private int liveChildCount;
        private IntPtr rootEvent;
        private bool rootSignaled;
        private uint activeProcessCount;
        private string requestedExecutablePath;
        private bool identityLogged;
        private bool unrelatedProcessSimulated;

        public ScriptedNativeScenario()
        {
            FailurePoint = ScriptedFailurePoint.None;
            CleanupWaitSignals = true;
            RootWaitSignals = true;
            ResumeThreadResult = 1;
            ResumeThreadLastError = 5;
            RootExitCode = 0;
            RootProcessId = unchecked((uint)Interlocked.Increment(ref nextProcessId));
            RootCreationFileTime = DateTime.UtcNow.ToFileTimeUtc();
        }

        public ScriptedFailurePoint FailurePoint { get; set; }

        public bool CleanupWaitSignals { get; set; }

        public bool RootWaitSignals { get; set; }

        public bool RootInitiallyExited { get; set; }

        public bool JobRemainsActive { get; set; }

        public uint RootExitCode { get; set; }

        public uint ResumeThreadResult { get; set; }

        public int ResumeThreadLastError { get; set; }

        public int WaitForSingleObjectLastError { get; set; }

        public int GetExitCodeProcessLastError { get; set; }

        public int QueryInformationJobObjectLastError { get; set; }

        public int TerminateJobObjectLastError { get; set; }

        public string CapturedExecutablePath { get; set; }

        public uint RootProcessId { get; }

        public long RootCreationFileTime { get; }

        public int OpenHandleCount
        {
            get
            {
                lock (sync)
                    return openHandles.Count;
            }
        }

        public int ResumeThreadCallCount => Volatile.Read(ref resumeThreadCallCount);

        public int JobTerminationCount => Volatile.Read(ref jobTerminationCount);

        public int LiveChildCount => Volatile.Read(ref liveChildCount);

        public int ProcessReacquisitionCount => 0;

        public IReadOnlyList<string> OperationLog
        {
            get
            {
                lock (sync)
                    return new ReadOnlyCollection<string>(operationLog.ToArray());
            }
        }

        public void SimulateUnrelatedProcessWithSamePid()
        {
            lock (sync)
                unrelatedProcessSimulated = true;
        }

        internal void Log(string operation)
        {
            lock (sync)
                operationLog.Add(operation);
        }

        internal IntPtr CreateEventHandle(string kind, bool signaled = false)
        {
            IntPtr value = ScriptedNativeMethods.CreateEventW(IntPtr.Zero, true, signaled, null);
            if (value == IntPtr.Zero)
            {
                int error = Marshal.GetLastWin32Error();
                throw new InvalidOperationException("Creating a scripted event handle failed with " + error + ".");
            }
            lock (sync)
                openHandles.Add(value, kind);
            return value;
        }

        internal NativeCallResult<bool> CloseEventHandle(
            IntPtr value,
            ScriptedFailurePoint releasePoint,
            Action markClosed)
        {
            lock (sync)
            {
                if (releasePoint != ScriptedFailurePoint.None &&
                    FailurePoint == releasePoint &&
                    consumedReleaseFailures.Add(releasePoint))
                    return NativeCallResult<bool>.Failure(3000 + (int)releasePoint);
            }

            bool succeeded = ScriptedNativeMethods.CloseHandle(value);
            if (!succeeded)
            {
                int error = Marshal.GetLastWin32Error();
                return NativeCallResult<bool>.Failure(error);
            }
            markClosed();
            lock (sync)
                openHandles.Remove(value);
            return NativeCallResult<bool>.Success(true);
        }

        internal string GetHandleKind(IntPtr value)
        {
            lock (sync)
                return openHandles[value];
        }

        internal bool ShouldFail(ScriptedFailurePoint point) => FailurePoint == point;

        internal void NoteRequestedExecutable(string path)
        {
            requestedExecutablePath = path;
        }

        internal string GetCapturedExecutable() => CapturedExecutablePath ?? requestedExecutablePath;

        internal void NoteIdentityCapture()
        {
            lock (sync)
            {
                if (identityLogged)
                    return;
                identityLogged = true;
                operationLog.Add("CaptureIdentity");
            }
        }

        internal void NoteProcessCreated(IntPtr eventHandle)
        {
            rootEvent = eventHandle;
            rootSignaled = RootInitiallyExited;
            activeProcessCount = RootInitiallyExited ? 0u : 1u;
            Volatile.Write(ref liveChildCount, RootInitiallyExited ? 0 : 1);
        }

        internal void SignalTermination()
        {
            if (CleanupWaitSignals)
            {
                rootSignaled = true;
                Volatile.Write(ref liveChildCount, 0);
                if (rootEvent != IntPtr.Zero)
                    ScriptedNativeMethods.SetEvent(rootEvent);
            }
            if (!JobRemainsActive)
                activeProcessCount = 0;
        }

        internal bool IsRootSignaled => rootSignaled && RootWaitSignals;

        internal bool UnrelatedProcessSimulated => unrelatedProcessSimulated;

        internal uint ActiveProcessCount => activeProcessCount;

        internal void NoteResumeCall() => Interlocked.Increment(ref resumeThreadCallCount);

        internal void NoteJobTermination() => Interlocked.Increment(ref jobTerminationCount);

        internal void CleanupUntransferredCreatedProcess(
            IntPtr processEvent,
            IntPtr threadEvent,
            SafeProcessHandle ownedProcess,
            SafeThreadHandle ownedThread)
        {
            Log("TerminatePostCreateRoot");
            SignalTermination();
            if (threadEvent != IntPtr.Zero)
            {
                CloseEventHandle(
                    threadEvent,
                    ScriptedFailurePoint.None,
                    ownedThread == null ? (Action)(() => { }) : ownedThread.MarkClosed);
                ownedThread?.Dispose();
            }
            if (processEvent != IntPtr.Zero)
            {
                CloseEventHandle(
                    processEvent,
                    ScriptedFailurePoint.None,
                    ownedProcess == null ? (Action)(() => { }) : ownedProcess.MarkClosed);
                ownedProcess?.Dispose();
            }
        }
    }

    public static class ContainedProcessTestFactory
    {
        public static WindowsContainedProcessLauncher CreateScriptedLauncher(
            ScriptedNativeScenario scenario)
        {
            if (scenario == null)
                throw new ArgumentNullException(nameof(scenario));
            return new WindowsContainedProcessLauncher(new ScriptedWindowsProcessApi(scenario));
        }

        public static IReadOnlyList<ContainedProcessException> CaptureConcurrentLaunchFailures(
            LaunchRequest request,
            ScriptedNativeScenario[] scenarios)
        {
            if (request == null)
                throw new ArgumentNullException(nameof(request));
            if (scenarios == null)
                throw new ArgumentNullException(nameof(scenarios));

            Task<ContainedProcessException>[] tasks = scenarios.Select(scenario => Task.Run(() =>
            {
                try
                {
                    CreateScriptedLauncher(scenario).Launch(request);
                    throw new InvalidOperationException("The scripted concurrent launch unexpectedly succeeded.");
                }
                catch (ContainedProcessException error)
                {
                    return error;
                }
            })).ToArray();
            ContainedProcessException[] errors = Task.WhenAll(tasks).GetAwaiter().GetResult();
            return new ReadOnlyCollection<ContainedProcessException>(errors);
        }

        public static WindowsContainedProcessLauncher CreateFaultingNativeLauncher(
            InjectedFailurePoint point) => CreateFaultingNativeLauncher(point, 5);

        public static WindowsContainedProcessLauncher CreateFaultingNativeLauncher(
            InjectedFailurePoint point,
            int nativeErrorCode)
        {
            if (nativeErrorCode <= 0)
                throw new ArgumentOutOfRangeException(nameof(nativeErrorCode));
            return new WindowsContainedProcessLauncher(
                new FaultInjectingNativeWindowsProcessApi(point, nativeErrorCode));
        }
    }

    internal sealed class ScriptedWindowsProcessApi : IWindowsProcessApi
    {
        private readonly ScriptedNativeScenario scenario;

        internal ScriptedWindowsProcessApi(ScriptedNativeScenario scenario)
        {
            this.scenario = scenario;
        }

        public NativeCallResult<SafeRedirectHandle> OpenStandardInput()
        {
            scenario.Log("OpenStdin");
            if (scenario.ShouldFail(ScriptedFailurePoint.OpenStdin))
                return NativeCallResult<SafeRedirectHandle>.Failure(1001);
            return NativeCallResult<SafeRedirectHandle>.Success(
                new SafeRedirectHandle(scenario.CreateEventHandle("stdin")));
        }

        public NativeCallResult<SafeRedirectHandle> OpenStandardOutput(string path)
        {
            int call = Interlocked.Increment(ref outputOpenCount);
            ScriptedFailurePoint point = call == 1
                ? ScriptedFailurePoint.OpenStdout
                : ScriptedFailurePoint.OpenStderr;
            scenario.Log(call == 1 ? "OpenStdout" : "OpenStderr");
            if (scenario.ShouldFail(point))
                return NativeCallResult<SafeRedirectHandle>.Failure(call == 1 ? 1002 : 1003);
            return NativeCallResult<SafeRedirectHandle>.Success(
                new SafeRedirectHandle(scenario.CreateEventHandle(call == 1 ? "stdout" : "stderr")));
        }

        private int outputOpenCount;

        public NativeCallResult<SafeJobHandle> CreateJob()
        {
            scenario.Log("CreateJob");
            if (scenario.ShouldFail(ScriptedFailurePoint.CreateJob))
                return NativeCallResult<SafeJobHandle>.Failure(1004);
            return NativeCallResult<SafeJobHandle>.Success(
                new SafeJobHandle(scenario.CreateEventHandle("job")));
        }

        public NativeCallResult<bool> ConfigureKillOnCloseJob(SafeJobHandle jobHandle)
        {
            scenario.Log("ConfigureJob");
            return scenario.ShouldFail(ScriptedFailurePoint.ConfigureJob)
                ? NativeCallResult<bool>.Failure(1005)
                : NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<IntPtr> InitializeAttributeList(int attributeCount)
        {
            scenario.Log("InitializeAttributeList");
            if (scenario.ShouldFail(ScriptedFailurePoint.InitializeAttributeList))
                return NativeCallResult<IntPtr>.Failure(1006);
            IntPtr value = Marshal.AllocHGlobal(64);
            for (int index = 0; index < 64; index++)
                Marshal.WriteByte(value, index, 0);
            return NativeCallResult<IntPtr>.Success(value);
        }

        public NativeCallResult<bool> UpdateHandleList(
            IntPtr attributeList,
            IntPtr inheritedHandleArray,
            int handleCount)
        {
            scenario.Log("UpdateAttributeList");
            return scenario.ShouldFail(ScriptedFailurePoint.UpdateAttributeList)
                ? NativeCallResult<bool>.Failure(1007)
                : NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<IntPtr> GetEnvironmentStrings()
        {
            scenario.Log("CaptureEnvironment");
            if (scenario.ShouldFail(ScriptedFailurePoint.CaptureEnvironment))
                return NativeCallResult<IntPtr>.Failure(1008);

            List<string> entries = new List<string>();
            foreach (DictionaryEntry item in Environment.GetEnvironmentVariables())
                entries.Add((string)item.Key + "=" + (string)item.Value);
            string block = string.Join("\0", entries
                .OrderBy(value => value, StringComparer.OrdinalIgnoreCase)
                .ThenBy(value => value, StringComparer.Ordinal)) + "\0\0";
            return NativeCallResult<IntPtr>.Success(Marshal.StringToHGlobalUni(block));
        }

        public NativeCallResult<bool> FreeEnvironmentStrings(IntPtr environment)
        {
            Marshal.FreeHGlobal(environment);
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
            scenario.Log("CreateProcessSuspended");
            if (scenario.ShouldFail(ScriptedFailurePoint.CreateProcess))
                return NativeCallResult<CreatedProcessHandles>.Failure(1009);

            scenario.NoteRequestedExecutable(request.ExecutablePath);
            IntPtr processEvent = IntPtr.Zero;
            IntPtr threadEvent = IntPtr.Zero;
            SafeProcessHandle ownedProcess = null;
            SafeThreadHandle ownedThread = null;
            bool transferred = false;
            try
            {
                processEvent = scenario.CreateEventHandle("process", scenario.RootInitiallyExited);
                scenario.NoteProcessCreated(processEvent);
                threadEvent = scenario.CreateEventHandle("thread");
                if (scenario.ShouldFail(ScriptedFailurePoint.PostCreateOwnershipHandoff))
                    throw new InvalidOperationException("Scripted post-create ownership handoff failed.");
                ownedProcess = new SafeProcessHandle(processEvent);
                ownedThread = new SafeThreadHandle(threadEvent);
                CreatedProcessHandles owners = new CreatedProcessHandles(ownedProcess, ownedThread);
                NativeCallResult<CreatedProcessHandles> result =
                    NativeCallResult<CreatedProcessHandles>.Success(owners);
                transferred = true;
                return result;
            }
            finally
            {
                if (processEvent != IntPtr.Zero && !transferred)
                {
                    scenario.CleanupUntransferredCreatedProcess(
                        processEvent,
                        threadEvent,
                        ownedProcess,
                        ownedThread);
                }
            }
        }

        public NativeCallResult<bool> AssignProcessToJob(
            SafeJobHandle jobHandle,
            SafeProcessHandle processHandle)
        {
            scenario.Log("AssignJob");
            return scenario.ShouldFail(ScriptedFailurePoint.AssignJob)
                ? NativeCallResult<bool>.Failure(1010)
                : NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> IsProcessInJob(
            SafeProcessHandle processHandle,
            SafeJobHandle jobHandle)
        {
            scenario.Log("VerifyJobMembership");
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<uint> GetProcessId(SafeProcessHandle processHandle)
        {
            scenario.NoteIdentityCapture();
            return scenario.ShouldFail(ScriptedFailurePoint.CaptureIdentity)
                ? NativeCallResult<uint>.Failure(1011)
                : NativeCallResult<uint>.Success(scenario.RootProcessId);
        }

        public NativeCallResult<long> GetProcessCreationFileTime(SafeProcessHandle processHandle) =>
            NativeCallResult<long>.Success(scenario.RootCreationFileTime);

        public NativeCallResult<string> QueryProcessImagePath(SafeProcessHandle processHandle) =>
            NativeCallResult<string>.Success(scenario.GetCapturedExecutable());

        public NativeCallResult<uint> ResumeThread(SafeThreadHandle threadHandle)
        {
            scenario.Log("ResumeThread");
            scenario.NoteResumeCall();
            if (scenario.ShouldFail(ScriptedFailurePoint.ResumeThread) ||
                scenario.ResumeThreadResult == NativeConstants.ResumeFailed)
            {
                return NativeCallResult<uint>.Failure(scenario.ResumeThreadLastError);
            }
            return NativeCallResult<uint>.Success(scenario.ResumeThreadResult);
        }

        public NativeCallResult<uint> WaitForSingleObject(
            SafeProcessHandle processHandle,
            uint milliseconds)
        {
            _ = scenario.UnrelatedProcessSimulated;
            if (scenario.WaitForSingleObjectLastError != 0)
                return NativeCallResult<uint>.Failure(scenario.WaitForSingleObjectLastError);
            return NativeCallResult<uint>.Success(
                scenario.IsRootSignaled ? NativeConstants.WaitObject0 : NativeConstants.WaitTimeout);
        }

        public NativeCallResult<uint> GetExitCode(SafeProcessHandle processHandle)
        {
            if (scenario.GetExitCodeProcessLastError != 0)
                return NativeCallResult<uint>.Failure(scenario.GetExitCodeProcessLastError);
            return NativeCallResult<uint>.Success(scenario.RootExitCode);
        }

        public NativeCallResult<bool> TerminateProcess(
            SafeProcessHandle processHandle,
            uint exitCode)
        {
            scenario.Log("TerminateProcess");
            scenario.SignalTermination();
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<bool> TerminateJob(SafeJobHandle jobHandle, uint exitCode)
        {
            scenario.Log("TerminateJobObject");
            scenario.NoteJobTermination();
            if (scenario.TerminateJobObjectLastError != 0)
                return NativeCallResult<bool>.Failure(scenario.TerminateJobObjectLastError);
            scenario.SignalTermination();
            return NativeCallResult<bool>.Success(true);
        }

        public NativeCallResult<uint> QueryActiveProcessCount(SafeJobHandle jobHandle)
        {
            if (scenario.QueryInformationJobObjectLastError != 0)
                return NativeCallResult<uint>.Failure(scenario.QueryInformationJobObjectLastError);
            return NativeCallResult<uint>.Success(scenario.ActiveProcessCount);
        }

        public NativeCallResult<bool> CloseRedirectHandle(SafeRedirectHandle handle)
        {
            string kind = FindHandleKind(handle.NativeValue);
            ScriptedFailurePoint point = kind == "stdin"
                ? ScriptedFailurePoint.ReleaseParentStdin
                : kind == "stdout"
                    ? ScriptedFailurePoint.ReleaseParentStdout
                    : ScriptedFailurePoint.ReleaseParentStderr;
            return scenario.CloseEventHandle(handle.NativeValue, point, handle.MarkClosed);
        }

        public NativeCallResult<bool> CloseThreadHandle(SafeThreadHandle handle) =>
            scenario.CloseEventHandle(
                handle.NativeValue,
                ScriptedFailurePoint.ReleaseThread,
                handle.MarkClosed);

        public NativeCallResult<bool> CloseProcessHandle(SafeProcessHandle handle) =>
            scenario.CloseEventHandle(handle.NativeValue, ScriptedFailurePoint.None, handle.MarkClosed);

        public NativeCallResult<bool> CloseJobHandle(SafeJobHandle handle) =>
            scenario.CloseEventHandle(handle.NativeValue, ScriptedFailurePoint.None, handle.MarkClosed);

        private string FindHandleKind(IntPtr value)
        {
            return scenario.GetHandleKind(value);
        }

        public NativeCallResult<bool> ReleaseAttributeList(IntPtr attributeList)
        {
            if (scenario.ShouldFail(ScriptedFailurePoint.ReleaseAttributeList) &&
                Interlocked.Exchange(ref attributeReleaseFailed, 1) == 0)
            {
                return NativeCallResult<bool>.Failure(3017);
            }
            Marshal.FreeHGlobal(attributeList);
            return NativeCallResult<bool>.Success(true);
        }

        private int attributeReleaseFailed;

        public NativeCallResult<bool> FreeAllocation(IntPtr allocation, NativeAllocationKind kind)
        {
            ScriptedFailurePoint point = kind == NativeAllocationKind.InheritedHandleArray
                ? ScriptedFailurePoint.ReleaseInheritedHandleArray
                : kind == NativeAllocationKind.CommandLine
                    ? ScriptedFailurePoint.ReleaseCommandBuffer
                    : ScriptedFailurePoint.ReleaseEnvironmentBuffer;
            if (scenario.ShouldFail(point))
            {
                lock (releaseSync)
                {
                    if (failedAllocations.Add(point))
                        return NativeCallResult<bool>.Failure(3000 + (int)point);
                }
            }
            Marshal.FreeHGlobal(allocation);
            return NativeCallResult<bool>.Success(true);
        }

        private readonly object releaseSync = new object();
        private readonly HashSet<ScriptedFailurePoint> failedAllocations =
            new HashSet<ScriptedFailurePoint>();

    }

    internal sealed class FaultInjectingNativeWindowsProcessApi : IWindowsProcessApi
    {
        private readonly InjectedFailurePoint point;
        private readonly int nativeErrorCode;
        private readonly IWindowsProcessApi inner = NativeWindowsProcessApi.Instance;

        internal FaultInjectingNativeWindowsProcessApi(
            InjectedFailurePoint point,
            int nativeErrorCode)
        {
            this.point = point;
            this.nativeErrorCode = nativeErrorCode;
        }

        public NativeCallResult<SafeRedirectHandle> OpenStandardInput() => inner.OpenStandardInput();

        public NativeCallResult<SafeRedirectHandle> OpenStandardOutput(string path) =>
            inner.OpenStandardOutput(path);

        public NativeCallResult<SafeJobHandle> CreateJob() => inner.CreateJob();

        public NativeCallResult<bool> ConfigureKillOnCloseJob(SafeJobHandle jobHandle) =>
            inner.ConfigureKillOnCloseJob(jobHandle);

        public NativeCallResult<IntPtr> InitializeAttributeList(int attributeCount) =>
            inner.InitializeAttributeList(attributeCount);

        public NativeCallResult<bool> UpdateHandleList(
            IntPtr attributeList,
            IntPtr inheritedHandleArray,
            int handleCount) => inner.UpdateHandleList(attributeList, inheritedHandleArray, handleCount);

        public NativeCallResult<IntPtr> GetEnvironmentStrings() => inner.GetEnvironmentStrings();

        public NativeCallResult<bool> FreeEnvironmentStrings(IntPtr environment) =>
            inner.FreeEnvironmentStrings(environment);

        public NativeCallResult<CreatedProcessHandles> CreateProcessSuspended(
            LaunchRequest request,
            IntPtr commandLine,
            IntPtr environment,
            IntPtr attributeList,
            SafeRedirectHandle standardInput,
            SafeRedirectHandle standardOutput,
            SafeRedirectHandle standardError)
        {
            if (point == InjectedFailurePoint.CreateProcess)
                return NativeCallResult<CreatedProcessHandles>.Failure(nativeErrorCode);
            return inner.CreateProcessSuspended(
                request,
                commandLine,
                environment,
                attributeList,
                standardInput,
                standardOutput,
                standardError);
        }

        public NativeCallResult<bool> AssignProcessToJob(
            SafeJobHandle jobHandle,
            SafeProcessHandle processHandle)
        {
            if (point == InjectedFailurePoint.AssignJob)
                return NativeCallResult<bool>.Failure(nativeErrorCode);
            return inner.AssignProcessToJob(jobHandle, processHandle);
        }

        public NativeCallResult<bool> IsProcessInJob(
            SafeProcessHandle processHandle,
            SafeJobHandle jobHandle) => inner.IsProcessInJob(processHandle, jobHandle);

        public NativeCallResult<uint> GetProcessId(SafeProcessHandle processHandle)
        {
            if (point == InjectedFailurePoint.CaptureIdentity)
                return NativeCallResult<uint>.Failure(nativeErrorCode);
            return inner.GetProcessId(processHandle);
        }

        public NativeCallResult<long> GetProcessCreationFileTime(SafeProcessHandle processHandle) =>
            inner.GetProcessCreationFileTime(processHandle);

        public NativeCallResult<string> QueryProcessImagePath(SafeProcessHandle processHandle) =>
            inner.QueryProcessImagePath(processHandle);

        public NativeCallResult<uint> ResumeThread(SafeThreadHandle threadHandle)
        {
            if (point == InjectedFailurePoint.ResumeThread)
                return NativeCallResult<uint>.Failure(nativeErrorCode);
            return inner.ResumeThread(threadHandle);
        }

        public NativeCallResult<uint> WaitForSingleObject(
            SafeProcessHandle processHandle,
            uint milliseconds) => inner.WaitForSingleObject(processHandle, milliseconds);

        public NativeCallResult<uint> GetExitCode(SafeProcessHandle processHandle) =>
            inner.GetExitCode(processHandle);

        public NativeCallResult<bool> TerminateProcess(
            SafeProcessHandle processHandle,
            uint exitCode) => inner.TerminateProcess(processHandle, exitCode);

        public NativeCallResult<bool> TerminateJob(SafeJobHandle jobHandle, uint exitCode) =>
            inner.TerminateJob(jobHandle, exitCode);

        public NativeCallResult<uint> QueryActiveProcessCount(SafeJobHandle jobHandle) =>
            inner.QueryActiveProcessCount(jobHandle);

        public NativeCallResult<bool> CloseRedirectHandle(SafeRedirectHandle handle) =>
            inner.CloseRedirectHandle(handle);

        public NativeCallResult<bool> CloseThreadHandle(SafeThreadHandle handle) =>
            inner.CloseThreadHandle(handle);

        public NativeCallResult<bool> CloseProcessHandle(SafeProcessHandle handle) =>
            inner.CloseProcessHandle(handle);

        public NativeCallResult<bool> CloseJobHandle(SafeJobHandle handle) =>
            inner.CloseJobHandle(handle);

        public NativeCallResult<bool> ReleaseAttributeList(IntPtr attributeList) =>
            inner.ReleaseAttributeList(attributeList);

        public NativeCallResult<bool> FreeAllocation(IntPtr allocation, NativeAllocationKind kind) =>
            inner.FreeAllocation(allocation, kind);

    }

    internal static class ScriptedNativeMethods
    {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        internal static extern IntPtr CreateEventW(
            IntPtr eventAttributes,
            [MarshalAs(UnmanagedType.Bool)] bool manualReset,
            [MarshalAs(UnmanagedType.Bool)] bool initialState,
            string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool SetEvent(IntPtr handle);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool CloseHandle(IntPtr handle);
    }
}
