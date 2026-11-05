using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>The WebView's error row and ordered provider-hint classification.</summary>
public static class NativeErrorPresentation
{
    public const string AcpPrefix = "ACP turn failed: ";
    public static string? Message(ConversationItem item) => item.Role == "assistant"
        && item.Text.StartsWith("Error: ", StringComparison.Ordinal) ? item.Text[7..] : null;

    public static ConversationSnapshot WithLiveError(ConversationSnapshot page)
    {
        if (page.Error is not { } error || page.Running || page.Stopping
            || page.Items.LastOrDefault(row => row.Role != "usage") is { } last && Message(last) == error) return page;
        return page with { Items = [.. page.Items, new("assistant", "Error: " + error, null, null, null, null, Kind: "native_turn_error")] };
    }

    public static string? HintKey(string? message)
    {
        if (message == null) return null;
        if (message.StartsWith(AcpPrefix, StringComparison.Ordinal)) return "err.hint.acp_no_fallback";
        if (!message.Contains("api: ", StringComparison.Ordinal) && !message.Contains("http: ", StringComparison.Ordinal)) return null;
        var text = message.ToLowerInvariant();
        bool Has(params string[] parts) => parts.Any(text.Contains);
        if (Has("insufficient balance", "insufficient_quota", "api: 402")) return "err.hint.balance";
        if (Has("maximum context length", "context_length_exceeded", "context length exceeded", "context window exceeded",
            "exceeds the context window", "prompt is too long", "input is too long", "too many tokens", "reduce the length")) return "err.hint.context";
        if (Has("image_url", "does not support image")) return "err.hint.image";
        if (Has("model names are", "model_not_found", "unknown model") || Has("model") && Has("does not exist")) return "err.hint.model_name";
        if (Has("api: 401", "invalid api key", "authentication_error")) return "err.hint.auth";
        if (Has("api: 429", "rate limit", "rate_limit")) return "err.hint.rate";
        if (Has("api: 5", "overloaded")) return "err.hint.server";
        if (Has("http: ") && Has("connect", "timed out", "deadline has elapsed", "dns", "refused", "via leftover",
            "via model api proxy", "via system proxy", "socks", "tunnel", "unreachable")) return "err.hint.network";
        if (Has("no tool output found for tool call", "no tool call found for function call output")) return "err.hint.tool_pairing";
        if (Has("x-opencode-session")) return "err.hint.opencode_session";
        return Has("api: 400") ? "err.hint.bad_request" : null;
    }
}
