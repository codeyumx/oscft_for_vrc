using System;
using System.IO;
using System.Net;

namespace FaceBridge.ModuleHost;

internal sealed record Options(IPEndPoint Endpoint, string Token, string ModulePath, string DataDir, int ParentPid)
{
    /// <summary>Parses the launch command line (see spec/ipc-protocol.md). Returns null and an error text when invalid.</summary>
    public static Options? Parse(string[] args, out string error)
    {
        string? connect = null, token = null, module = null, dataDir = null, parent = null;
        for (int i = 0; i < args.Length; i += 2)
        {
            if (i + 1 >= args.Length)
            {
                error = $"Missing value for {args[i]}.";
                return null;
            }

            switch (args[i])
            {
                case "--connect": connect = args[i + 1]; break;
                case "--token": token = args[i + 1]; break;
                case "--module": module = args[i + 1]; break;
                case "--data-dir": dataDir = args[i + 1]; break;
                case "--parent-pid": parent = args[i + 1]; break;
                default:
                    error = $"Unknown argument {args[i]}.";
                    return null;
            }
        }

        if (connect == null || token == null || module == null || dataDir == null || parent == null)
        {
            error = "Required: --connect <ip:port> --token <token> --module <dll> --data-dir <dir> --parent-pid <pid>.";
            return null;
        }

        if (!IPEndPoint.TryParse(connect, out IPEndPoint? endpoint) || endpoint.Port == 0)
        {
            error = $"Invalid --connect value '{connect}'.";
            return null;
        }

        if (token.Length == 0)
        {
            error = "Empty --token.";
            return null;
        }

        if (!Path.IsPathRooted(module) || !Path.IsPathRooted(dataDir))
        {
            error = "--module and --data-dir must be absolute paths.";
            return null;
        }

        if (!int.TryParse(parent, out int pid) || pid <= 0)
        {
            error = $"Invalid --parent-pid value '{parent}'.";
            return null;
        }

        error = string.Empty;
        return new Options(endpoint, token, module, dataDir, pid);
    }
}
