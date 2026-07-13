namespace Yap.NsisSmoke.Testing
{
    public static class LaunchRequestProbe
    {
        public static string BuildCommandLine(LaunchRequest request) => request.BuildCommandLine();

        public static string BuildEnvironmentBlockText(LaunchRequest request, string[] inherited) =>
            EnvironmentBlockBuilder.BuildBlockText(request, inherited);
    }
}
