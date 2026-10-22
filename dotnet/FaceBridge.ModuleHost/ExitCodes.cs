namespace FaceBridge.ModuleHost;

internal static class ExitCodes
{
    public const int Ok = 0;
    public const int BadArguments = 2;
    public const int ConnectFailed = 3;
    public const int LoadFailed = 4;
    public const int ModuleCrashed = 5;
    public const int ParentGone = 6;
}
