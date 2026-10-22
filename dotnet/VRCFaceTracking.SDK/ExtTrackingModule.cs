using System;
using Microsoft.Extensions.Logging;
using VRCFaceTracking.Core.Library;

namespace VRCFaceTracking;

/// <summary>Base class of every tracking module.</summary>
public abstract class ExtTrackingModule
{
    /// <summary>Lifecycle state, polled by the host.</summary>
    public ModuleState Status = ModuleState.Uninitialized;

    /// <summary>Assigned by the host before <see cref="Supported"/> is read.</summary>
    public ILogger Logger;

    public ModuleMetadata ModuleInformation;

    /// <summary>Which of eye / expression tracking the module can provide at all.</summary>
    public virtual (bool SupportsEye, bool SupportsExpression) Supported => (true, true);

    /// <returns>Which of eye / expression tracking the module now provides.</returns>
    public abstract (bool eyeSuccess, bool expressionSuccess) Initialize(bool eyeAvailable, bool expressionAvailable);

    public abstract void Update();

    public abstract void Teardown();
}
