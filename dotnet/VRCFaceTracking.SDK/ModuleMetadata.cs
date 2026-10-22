using System.Collections.Generic;
using System.IO;

namespace VRCFaceTracking;

/// <summary>Per-module information a module fills in during <c>Initialize</c>.</summary>
public struct ModuleMetadata
{
    public delegate void ActiveChange(bool state);

    public ActiveChange OnActiveChange;

    private bool _active;

    public List<Stream> StaticImages { get; set; }

    public string Name { get; set; }

    /// <summary>Setting this raises <see cref="OnActiveChange"/>.</summary>
    public bool Active
    {
        get => _active;
        set
        {
            _active = value;
            OnActiveChange?.Invoke(value);
        }
    }

    public bool UsingEye { get; set; }

    public bool UsingExpression { get; set; }
}
