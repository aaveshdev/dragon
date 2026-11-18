using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Windows;

namespace DragonRuntime
{
    public sealed class PluginContext
    {
        public string DocsDir { get; }
        public System.Windows.Threading.Dispatcher Dispatcher { get; }
        public PluginContext(string docsDir, System.Windows.Threading.Dispatcher dispatcher)
        {
            DocsDir = docsDir;
            Dispatcher = dispatcher;
        }
    }

    public interface IDragonPlugin
    {
        string Namespace { get; }
        bool Handles(string method);
        bool Handle(string method, JsonElement args, Action<object> reply);
        UIElement CreateView(string kind, string id, JsonElement parameters);
        void ReleaseView(string kind, string id);
    }

    public abstract class DragonPluginBase : IDragonPlugin
    {
        public abstract string Namespace { get; }
        public virtual bool Handles(string method) => method.StartsWith(Namespace + ".", StringComparison.Ordinal);
        public virtual bool Handle(string method, JsonElement args, Action<object> reply) => false;
        public virtual UIElement CreateView(string kind, string id, JsonElement parameters) => null;
        public virtual void ReleaseView(string kind, string id) { }
    }
}
