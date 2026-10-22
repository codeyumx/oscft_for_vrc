using System;
using System.Collections.Generic;

namespace FaceBridge.Sdk;

/// <summary>One option of a choice setting.</summary>
/// <param name="Value">Stored value.</param>
/// <param name="Label">Text shown to the user.</param>
public sealed record ChoiceOption(string Value, string Label);

/// <summary>Raised by <see cref="ModuleSettings.Changed"/> when the user changes a setting in FaceBridge.</summary>
public sealed class SettingChangedEventArgs : EventArgs
{
    public SettingChangedEventArgs(string key, object value)
    {
        Key = key;
        Value = value;
    }

    public string Key { get; }

    /// <summary><see cref="int"/>, <see cref="float"/>, <see cref="bool"/> or <see cref="string"/>, per the setting's type.</summary>
    public object Value { get; }
}

internal enum SettingKind
{
    Int,
    Float,
    Bool,
    String,
    Choice,

    /// <summary>Read-only text owned by the module; not stored and never changed by the host.</summary>
    Text,

    /// <summary>A button; has no value, only a handler.</summary>
    Button,
}

internal sealed class SettingField
{
    public SettingField(SettingKind kind, string key, string label, string? description, object defaultValue)
    {
        Kind = kind;
        Key = key;
        Label = label;
        Description = description;
        Default = defaultValue;
        Value = defaultValue;
    }

    public SettingKind Kind { get; }
    public string Key { get; }
    public string Label { get; }
    public string? Description { get; }
    public object Default { get; }
    public double? Min { get; init; }
    public double? Max { get; init; }
    public double? Step { get; init; }
    public IReadOnlyList<ChoiceOption>? Options { get; init; }

    /// <summary>The handler of a <see cref="SettingKind.Button"/>.</summary>
    public Action? OnPressed { get; init; }

    public object Value { get; set; }

    /// <summary>Whether the field has a value the user edits and the host stores (not text or button).</summary>
    public bool IsStored => Kind is not (SettingKind.Text or SettingKind.Button);

    public string KindName => Kind switch
    {
        SettingKind.Int => "int",
        SettingKind.Float => "float",
        SettingKind.Bool => "bool",
        SettingKind.String => "string",
        SettingKind.Choice => "choice",
        SettingKind.Text => "text",
        _ => "button",
    };

    /// <summary>Converts <paramref name="input"/> (CLR number/bool/string) to this field's type, clamped; false if not convertible.</summary>
    public bool TryCoerce(object? input, out object result)
    {
        result = Default;
        switch (Kind)
        {
            case SettingKind.Int:
                if (!TryToDouble(input, out double whole) || double.IsNaN(whole) || double.IsInfinity(whole))
                {
                    return false;
                }

                double rounded = Math.Round(whole);
                rounded = Math.Clamp(rounded, Min ?? int.MinValue, Max ?? int.MaxValue);
                rounded = Math.Clamp(rounded, int.MinValue, int.MaxValue);
                result = (int)rounded;
                return true;

            case SettingKind.Float:
                if (!TryToDouble(input, out double real) || double.IsNaN(real) || double.IsInfinity(real))
                {
                    return false;
                }

                real = Math.Clamp(real, Min ?? float.MinValue, Max ?? float.MaxValue);
                result = (float)Math.Clamp(real, float.MinValue, float.MaxValue);
                return true;

            case SettingKind.Bool:
                if (input is bool flag)
                {
                    result = flag;
                    return true;
                }

                return false;

            case SettingKind.String or SettingKind.Text:
                if (input is string text)
                {
                    result = text;
                    return true;
                }

                return false;

            case SettingKind.Choice:
                if (input is string choice && ContainsOption(choice))
                {
                    result = choice;
                    return true;
                }

                return false;

            default:
                return false;
        }
    }

    private bool ContainsOption(string value)
    {
        foreach (ChoiceOption option in Options!)
        {
            if (string.Equals(option.Value, value, StringComparison.Ordinal))
            {
                return true;
            }
        }

        return false;
    }

    private static bool TryToDouble(object? input, out double value)
    {
        switch (input)
        {
            case int i: value = i; return true;
            case long l: value = l; return true;
            case short s: value = s; return true;
            case byte b: value = b; return true;
            case float f: value = f; return true;
            case double dbl: value = dbl; return true;
            case decimal m: value = (double)m; return true;
            default: value = 0; return false;
        }
    }
}
