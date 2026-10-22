using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using FaceBridge.Sdk.Hosting;

namespace FaceBridge.Sdk;

/// <summary>
/// A module's settings page. Declare the fields with the <c>Add*</c> methods, call <see cref="Declare"/>
/// once, then read values with the <c>Get*</c> methods.
/// <para>
/// Besides user-editable values (int, float, bool, string, choice) a page can hold read-only text the module updates
/// itself (<see cref="AddText"/>, <see cref="SetText"/>; e.g. a pairing status) and buttons
/// (<see cref="AddButton"/>; e.g. "Pair"). Text and buttons are never stored.
/// </para>
/// <para>
/// In FaceBridge the schema is shown in the module's settings page, values are stored by FaceBridge,
/// <see cref="Changed"/> fires when the user edits one, text changes show up live and pressing a button runs its
/// handler. In VRCFaceTracking (or anywhere else) the declared defaults apply, and values are read from and written
/// to a JSON file next to the module DLL (or, if that folder is not writable, under
/// <c>%APPDATA%\FaceBridge\module-settings</c>); nothing edits that file while the module runs, so
/// <see cref="Changed"/> never fires there, text is only visible to the module itself and buttons never fire.
/// </para>
/// All members are thread-safe. <see cref="Changed"/> handlers run on the thread that delivered the change; button
/// handlers run on a thread-pool thread.
/// </summary>
public sealed class ModuleSettings
{
    private readonly object _gate = new();
    private readonly List<SettingField> _fields = new();
    private readonly Dictionary<string, SettingField> _byKey = new(StringComparer.Ordinal);
    private readonly string _fallbackPath;
    private readonly string? _primaryPath;

    /// <summary>Settings of the module whose assembly calls this constructor.</summary>
    [MethodImpl(MethodImplOptions.NoInlining)]
    public ModuleSettings()
        : this(Assembly.GetCallingAssembly())
    {
    }

    /// <param name="moduleAssembly">The module's assembly; decides where the fallback JSON file lives.</param>
    public ModuleSettings(Assembly moduleAssembly)
    {
        ArgumentNullException.ThrowIfNull(moduleAssembly);
        string name = moduleAssembly.GetName().Name ?? "module";
        foreach (char c in Path.GetInvalidFileNameChars())
        {
            name = name.Replace(c, '_');
        }

        string appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
        _fallbackPath = Path.Combine(appData, "FaceBridge", "module-settings", name + ".json");

        string location = moduleAssembly.Location;
        string? dir = string.IsNullOrEmpty(location) ? null : Path.GetDirectoryName(location);
        _primaryPath = dir == null ? null : Path.Combine(dir, name + ".settings.json");
    }

    /// <summary>Raised when the user changes a setting in FaceBridge.</summary>
    public event EventHandler<SettingChangedEventArgs>? Changed;

    public ModuleSettings AddInt(string key, string label, int defaultValue, int? min = null, int? max = null, string? description = null)
    {
        if (min > max)
        {
            throw new ArgumentException("min must not exceed max.");
        }

        return Add(new SettingField(SettingKind.Int, key, label, description, ClampInt(defaultValue, min, max))
        {
            Min = min,
            Max = max,
        });
    }

    public ModuleSettings AddFloat(string key, string label, float defaultValue, float? min = null, float? max = null, float? step = null, string? description = null)
    {
        if (!float.IsFinite(defaultValue) || min is { } lo && !float.IsFinite(lo) || max is { } hi && !float.IsFinite(hi) || step is { } st && !float.IsFinite(st))
        {
            throw new ArgumentException("Float setting values must be finite.");
        }

        if (min > max)
        {
            throw new ArgumentException("min must not exceed max.");
        }

        float clamped = Math.Clamp(defaultValue, min ?? float.MinValue, max ?? float.MaxValue);
        return Add(new SettingField(SettingKind.Float, key, label, description, clamped)
        {
            Min = min,
            Max = max,
            Step = step,
        });
    }

    public ModuleSettings AddBool(string key, string label, bool defaultValue, string? description = null)
    {
        return Add(new SettingField(SettingKind.Bool, key, label, description, defaultValue));
    }

    public ModuleSettings AddString(string key, string label, string defaultValue, string? description = null)
    {
        ArgumentNullException.ThrowIfNull(defaultValue);
        return Add(new SettingField(SettingKind.String, key, label, description, defaultValue));
    }

    public ModuleSettings AddChoice(string key, string label, string defaultValue, IEnumerable<ChoiceOption> options, string? description = null)
    {
        ArgumentNullException.ThrowIfNull(defaultValue);
        ArgumentNullException.ThrowIfNull(options);
        var list = new List<ChoiceOption>(options);
        if (list.Count == 0)
        {
            throw new ArgumentException("A choice setting needs at least one option.", nameof(options));
        }

        if (!list.Exists(o => o.Value == defaultValue))
        {
            throw new ArgumentException("The default must be one of the options.", nameof(defaultValue));
        }

        return Add(new SettingField(SettingKind.Choice, key, label, description, defaultValue) { Options = list });
    }

    /// <summary>
    /// Read-only text shown in the settings page (newlines allowed), e.g. a connection or pairing status. Change it
    /// with <see cref="SetText"/>; read it with <see cref="GetString"/>. Not stored: every run starts with
    /// <paramref name="text"/>.
    /// </summary>
    public ModuleSettings AddText(string key, string label, string text = "", string? description = null)
    {
        ArgumentNullException.ThrowIfNull(text);
        return Add(new SettingField(SettingKind.Text, key, label, description, text));
    }

    /// <summary>
    /// A button in the settings page. In FaceBridge, pressing it runs <paramref name="onPressed"/> on a thread-pool
    /// thread (exceptions are logged, not rethrown); elsewhere it never fires. A button has no value:
    /// <c>Get*</c> and <see cref="Set"/> throw for it.
    /// </summary>
    public ModuleSettings AddButton(string key, string label, Action onPressed, string? description = null)
    {
        ArgumentNullException.ThrowIfNull(onPressed);
        return Add(new SettingField(SettingKind.Button, key, label, description, string.Empty) { OnPressed = onPressed });
    }

    /// <summary>
    /// Publishes the schema to FaceBridge (or loads the JSON file elsewhere) and applies stored values.
    /// May be called again after adding more fields.
    /// </summary>
    public void Declare()
    {
        IModuleHostBridge? bridge = ModuleHostRegistration.Bridge;
        string schema = BuildSchemaJson();
        if (bridge != null)
        {
            string stored = ModuleHostRegistration.Attach(this);
            ApplyJson(stored, raiseChanged: false);
            bridge.PublishSettingsSchema(schema);
        }
        else
        {
            LoadFile();
        }
    }

    public int GetInt(string key) => (int)Get(key, SettingKind.Int);

    public float GetFloat(string key) => (float)Get(key, SettingKind.Float);

    public bool GetBool(string key) => (bool)Get(key, SettingKind.Bool);

    /// <summary>Value of a string or choice setting, or the current text of a text field.</summary>
    public string GetString(string key)
    {
        lock (_gate)
        {
            SettingField field = Field(key);
            if (field.Kind is not (SettingKind.String or SettingKind.Choice or SettingKind.Text))
            {
                throw new InvalidOperationException($"Setting '{key}' is {field.KindName}, not string, choice or text.");
            }

            return (string)field.Value;
        }
    }

    /// <summary>
    /// Changes a setting from the module side. FaceBridge stores and displays the new value; elsewhere it is saved
    /// to the JSON file. Does not raise <see cref="Changed"/>. Out-of-range numbers are clamped. For a text field
    /// this is the same as <see cref="SetText"/>.
    /// </summary>
    /// <exception cref="ArgumentException">The value does not fit the setting's type.</exception>
    /// <exception cref="InvalidOperationException">The setting is a button.</exception>
    public void Set(string key, object value) => Write(key, value, textOnly: false);

    /// <summary>
    /// Changes the text of a text field (see <see cref="AddText"/>). FaceBridge shows the new text right away.
    /// </summary>
    /// <exception cref="InvalidOperationException">The setting is not a text field.</exception>
    public void SetText(string key, string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        Write(key, text, textOnly: true);
    }

    /// <summary>The schema JSON FaceBridge receives (see spec/ipc-protocol.md, "Settings schema JSON").</summary>
    public string BuildSchemaJson()
    {
        lock (_gate)
        {
            var stream = new MemoryStream();
            using (var w = new Utf8JsonWriter(stream))
            {
                w.WriteStartObject();
                w.WriteStartArray("fields");
                foreach (SettingField f in _fields)
                {
                    w.WriteStartObject();
                    w.WriteString("key", f.Key);
                    w.WriteString("label", f.Label);
                    if (f.Description != null)
                    {
                        w.WriteString("description", f.Description);
                    }

                    w.WriteString("type", f.KindName);
                    if (f.Kind != SettingKind.Button)
                    {
                        // A text field's "default" is the text it shows now.
                        w.WritePropertyName("default");
                        WriteValue(w, f.Kind == SettingKind.Text ? f.Value : f.Default);
                    }

                    if (f.Min is { } min)
                    {
                        w.WritePropertyName("min");
                        WriteNumber(w, f, min);
                    }

                    if (f.Max is { } max)
                    {
                        w.WritePropertyName("max");
                        WriteNumber(w, f, max);
                    }

                    if (f.Step is { } step)
                    {
                        w.WritePropertyName("step");
                        WriteNumber(w, f, step);
                    }

                    if (f.Options != null)
                    {
                        w.WriteStartArray("options");
                        foreach (ChoiceOption o in f.Options)
                        {
                            w.WriteStartObject();
                            w.WriteString("value", o.Value);
                            w.WriteString("label", o.Label);
                            w.WriteEndObject();
                        }

                        w.WriteEndArray();
                    }

                    w.WriteEndObject();
                }

                w.WriteEndArray();
                w.WriteEndObject();
            }

            return Encoding.UTF8.GetString(stream.GetBuffer(), 0, (int)stream.Length);
        }
    }

    /// <summary>
    /// Applies an object of key to value. Unknown keys and invalid values are ignored, and so are text and button
    /// fields (the module owns those).
    /// </summary>
    internal void ApplyJson(string valuesJson, bool raiseChanged)
    {
        List<SettingChangedEventArgs>? changes = null;
        using (JsonDocument doc = JsonDocument.Parse(valuesJson))
        {
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return;
            }

            lock (_gate)
            {
                foreach (JsonProperty property in doc.RootElement.EnumerateObject())
                {
                    if (!_byKey.TryGetValue(property.Name, out SettingField? field)
                        || !field.IsStored
                        || !field.TryCoerce(ToClr(property.Value), out object coerced)
                        || coerced.Equals(field.Value))
                    {
                        continue;
                    }

                    field.Value = coerced;
                    (changes ??= new List<SettingChangedEventArgs>()).Add(new SettingChangedEventArgs(field.Key, coerced));
                }
            }
        }

        if (!raiseChanged || changes == null)
        {
            return;
        }

        foreach (SettingChangedEventArgs change in changes)
        {
            Changed?.Invoke(this, change);
        }
    }

    /// <summary>The handler of button <paramref name="key"/>; null if no such button is declared.</summary>
    internal Action? ButtonHandler(string key)
    {
        lock (_gate)
        {
            return _byKey.TryGetValue(key, out SettingField? field) && field.Kind == SettingKind.Button ? field.OnPressed : null;
        }
    }

    private void Write(string key, object value, bool textOnly)
    {
        string json;
        bool stored;
        lock (_gate)
        {
            SettingField field = Field(key);
            if (textOnly && field.Kind != SettingKind.Text)
            {
                throw new InvalidOperationException($"Setting '{key}' is {field.KindName}, not text.");
            }

            if (field.Kind == SettingKind.Button)
            {
                throw new InvalidOperationException($"Setting '{key}' is a button and has no value.");
            }

            if (!field.TryCoerce(value, out object coerced))
            {
                throw new ArgumentException($"Value {value} is not valid for setting '{key}' ({field.KindName}).", nameof(value));
            }

            field.Value = coerced;
            json = ValuesJson(only: field);
            stored = field.IsStored;
        }

        IModuleHostBridge? bridge = ModuleHostRegistration.Bridge;
        if (bridge != null)
        {
            bridge.WriteSettingValues(json);
        }
        else if (stored)
        {
            // Text is runtime state: not saved to the file.
            SaveFile();
        }
    }

    private ModuleSettings Add(SettingField field)
    {
        ArgumentException.ThrowIfNullOrEmpty(field.Key);
        ArgumentNullException.ThrowIfNull(field.Label);
        lock (_gate)
        {
            if (_byKey.ContainsKey(field.Key))
            {
                throw new ArgumentException($"Setting key '{field.Key}' is already declared.", nameof(field));
            }

            _byKey.Add(field.Key, field);
            _fields.Add(field);
        }

        return this;
    }

    private SettingField Field(string key)
    {
        if (!_byKey.TryGetValue(key, out SettingField? field))
        {
            throw new KeyNotFoundException($"Setting '{key}' is not declared.");
        }

        return field;
    }

    private object Get(string key, SettingKind kind)
    {
        lock (_gate)
        {
            SettingField field = Field(key);
            if (field.Kind != kind)
            {
                throw new InvalidOperationException($"Setting '{key}' is {field.KindName}, not {kind.ToString().ToLowerInvariant()}.");
            }

            return field.Value;
        }
    }

    /// <summary>Values as an object of key to value: just <paramref name="only"/>, or every stored field.</summary>
    private string ValuesJson(SettingField? only)
    {
        var stream = new MemoryStream();
        using (var w = new Utf8JsonWriter(stream))
        {
            w.WriteStartObject();
            foreach (SettingField f in only != null ? (IEnumerable<SettingField>)new[] { only } : _fields)
            {
                if (only == null && !f.IsStored)
                {
                    continue;
                }

                w.WritePropertyName(f.Key);
                WriteValue(w, f.Value);
            }

            w.WriteEndObject();
        }

        return Encoding.UTF8.GetString(stream.GetBuffer(), 0, (int)stream.Length);
    }

    private static void WriteValue(Utf8JsonWriter w, object value)
    {
        switch (value)
        {
            case int i: w.WriteNumberValue(i); break;
            case float f: w.WriteNumberValue(FloatToDouble(f)); break;
            case bool b: w.WriteBooleanValue(b); break;
            default: w.WriteStringValue((string)value); break;
        }
    }

    private static void WriteNumber(Utf8JsonWriter w, SettingField f, double value)
    {
        if (f.Kind == SettingKind.Int)
        {
            w.WriteNumberValue((long)value);
        }
        else
        {
            w.WriteNumberValue(FloatToDouble((float)value));
        }
    }

    // 0.05f -> 0.05 rather than 0.05000000074505806.
    private static double FloatToDouble(float value) =>
        double.Parse(value.ToString("R", CultureInfo.InvariantCulture), CultureInfo.InvariantCulture);

    private static int ClampInt(int value, int? min, int? max) => Math.Clamp(value, min ?? int.MinValue, max ?? int.MaxValue);

    private static object? ToClr(JsonElement element) => element.ValueKind switch
    {
        JsonValueKind.Number => element.TryGetInt64(out long l) ? l : element.GetDouble(),
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        JsonValueKind.String => element.GetString(),
        _ => null,
    };

    private void LoadFile()
    {
        foreach (string? path in new[] { _primaryPath, _fallbackPath })
        {
            if (path == null || !File.Exists(path))
            {
                continue;
            }

            try
            {
                ApplyJson(File.ReadAllText(path, Encoding.UTF8), raiseChanged: false);
                return;
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
            {
                // Unreadable or corrupt file: try the next location, else keep defaults.
            }
        }
    }

    private void SaveFile()
    {
        string json;
        lock (_gate)
        {
            json = ValuesJson(only: null);
        }

        foreach (string? path in new[] { _primaryPath, _fallbackPath })
        {
            if (path == null)
            {
                continue;
            }

            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                string temp = path + ".tmp";
                File.WriteAllText(temp, json, new UTF8Encoding(false));
                File.Move(temp, path, overwrite: true);
                return;
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                // Folder not writable: try the next location.
            }
        }
    }
}
