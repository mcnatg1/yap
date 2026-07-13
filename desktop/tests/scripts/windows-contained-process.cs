using System;
using System.Collections;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.IO;
using System.Linq;
using System.Text;

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
}
