using System.Text.RegularExpressions;

namespace Wisp.ProjectBrowser;

/// <summary>Dependency-free token classification for transcript code blocks.
/// Unknown languages and oversized input stay plain; only saved spans are
/// colored by the platform renderer. No code is ever executed.</summary>
public static class NativeCodeHighlight
{
    public sealed record Token(int Start, int Length, string Kind);
    public const string Comment = "comment";
    public const string StringToken = "string";
    public const string Number = "number";
    public const string Keyword = "keyword";

    private static readonly Dictionary<string, string[]> Keywords = new()
    {
        ["python"] = ["def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as", "with", "try", "except", "finally", "raise", "lambda", "yield", "in", "not", "and", "or", "is", "None", "True", "False", "pass", "break", "continue", "global", "async", "await", "assert", "del"],
        ["r"] = ["function", "if", "else", "for", "while", "repeat", "break", "next", "return", "TRUE", "FALSE", "NULL", "NA", "Inf", "NaN", "library", "require", "in", "stop", "warning"],
        ["bash"] = ["if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "function", "return", "export", "local", "echo", "cd", "set", "source", "in"],
        ["c"] = ["int", "char", "float", "double", "void", "long", "short", "unsigned", "signed", "struct", "union", "enum", "typedef", "const", "static", "extern", "return", "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "sizeof", "goto"],
        ["cpp"] = ["int", "char", "float", "double", "void", "long", "short", "unsigned", "struct", "class", "enum", "typedef", "const", "static", "return", "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "sizeof", "new", "delete", "namespace", "using", "template", "typename", "public", "private", "protected", "virtual", "override", "auto", "bool", "true", "false", "nullptr", "constexpr"],
        ["rust"] = ["fn", "let", "mut", "const", "static", "struct", "enum", "trait", "impl", "match", "if", "else", "for", "while", "loop", "return", "use", "mod", "pub", "crate", "self", "Self", "where", "async", "await", "move", "ref", "dyn", "true", "false", "unsafe", "as", "in", "break", "continue"],
        ["js"] = ["function", "const", "let", "var", "return", "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "class", "extends", "new", "this", "typeof", "instanceof", "await", "async", "yield", "import", "export", "from", "true", "false", "null", "undefined", "try", "catch", "finally", "throw"],
        ["ts"] = ["function", "const", "let", "var", "return", "if", "else", "for", "while", "switch", "case", "default", "break", "continue", "class", "extends", "interface", "type", "enum", "implements", "new", "this", "typeof", "keyof", "await", "async", "import", "export", "from", "true", "false", "null", "undefined", "try", "catch", "finally", "throw", "readonly", "public", "private", "protected", "as"],
        ["cs"] = ["using", "namespace", "class", "struct", "interface", "enum", "record", "public", "private", "protected", "internal", "static", "readonly", "const", "void", "int", "string", "bool", "double", "float", "long", "var", "new", "return", "if", "else", "for", "foreach", "while", "switch", "case", "default", "break", "continue", "true", "false", "null", "async", "await", "try", "catch", "finally", "throw", "this", "base"],
        ["go"] = ["func", "package", "import", "var", "const", "type", "struct", "interface", "map", "chan", "go", "defer", "return", "if", "else", "for", "range", "switch", "case", "default", "break", "continue", "true", "false", "nil", "select"],
        ["java"] = ["public", "private", "protected", "static", "final", "class", "interface", "enum", "record", "void", "int", "long", "double", "float", "boolean", "char", "String", "var", "new", "return", "if", "else", "for", "while", "switch", "case", "default", "break", "continue", "true", "false", "null", "try", "catch", "finally", "throw", "throws", "extends", "implements", "this", "super", "package", "import"],
        ["sql"] = ["select", "from", "where", "group", "by", "order", "having", "limit", "offset", "insert", "into", "values", "update", "set", "delete", "create", "table", "alter", "drop", "index", "view", "join", "left", "right", "inner", "outer", "on", "as", "and", "or", "not", "null", "is", "in", "between", "like", "distinct", "count", "sum", "avg", "min", "max", "case", "when", "then", "else", "end", "with", "union", "all", "primary", "key", "foreign", "references"],
        ["toml"] = ["true", "false"],
        ["yaml"] = ["true", "false", "null", "yes", "no", "on", "off"],
        ["json"] = [],
        ["diff"] = [],
        ["xml"] = [],
        ["html"] = [],
    };

    public static string? NormalizeLanguage(string? info)
    {
        var value = (info ?? "").Trim().ToLowerInvariant();
        var first = value.Split([' ', '\t', ':'], 2)[0].Trim();
        return first switch
        {
            "" => null,
            "py" or "python3" or "ipython" => "python",
            "rscript" => "r",
            "sh" or "shell" or "zsh" or "console" or "terminal" or "powershell" or "ps1" => "bash",
            "c++" or "cxx" or "cc" or "hpp" or "h" or "h++" => "cpp",
            "rs" => "rust",
            "javascript" or "jsx" or "node" or "nodejs" => "js",
            "typescript" or "tsx" => "ts",
            "csharp" or "c#" => "cs",
            "golang" => "go",
            "yml" => "yaml",
            "xhtml" or "svg" => "xml",
            _ => Keywords.ContainsKey(first) ? first : null,
        };
    }

    public static bool Supported(string? language) => NormalizeLanguage(language) != null;

    public static IReadOnlyList<Token> Tokenize(string code, string? language)
    {
        var lang = NormalizeLanguage(language);
        if (lang is null || code.Length == 0 || code.Length > 200_000) return [];
        var pattern = PatternFor(lang);
        var tokens = new List<Token>();
        foreach (Match match in Regex.Matches(code, pattern, RegexOptions.Multiline | RegexOptions.CultureInvariant))
        {
            var kind = match.Groups["comment"].Success ? Comment
                : match.Groups["string"].Success ? StringToken
                : match.Groups["number"].Success ? Number
                : Keyword;
            // Identifier-based patterns match every name; keep only real keywords.
            if (kind == Keyword && IdentifierKeywordLanguages.Contains(lang) && !Keywords[lang].Contains(match.Value)) continue;
            tokens.Add(new Token(match.Index, match.Length, kind));
        }
        return tokens;
    }

    private static readonly HashSet<string> IdentifierKeywordLanguages =
        ["python", "r", "bash", "c", "cpp", "rust", "js", "ts", "cs", "go", "java", "sql", "toml", "yaml"];

    // Order matters: comments first, then strings, so a quote inside a comment
    // and a comment marker inside a string both resolve to the earlier opener.
    private const string Hash =
        @"(?<comment>\#[^\n]*)|(?<string>""(?:\\.|[^""\\\n])*""|'(?:\\.|[^'\\\n])*')|(?<number>\b(?:0[xX][0-9a-fA-F_]+|\d[\d_]*(?:\.[\d_]+)?(?:[eE][+-]?\d+)?)\b)|(?<keyword>\b[A-Za-z_][A-Za-z0-9_]*\b)";
    private const string Python =
        @"(?<comment>\#[^\n]*)|(?<string>""""""[\s\S]*?""""""|'''[\s\S]*?'''|""(?:\\.|[^""\\\n])*""|'(?:\\.|[^'\\\n])*')|(?<number>\b(?:0[xX][0-9a-fA-F_]+|\d[\d_]*(?:\.[\d_]+)?(?:[eE][+-]?\d+)?)\b)|(?<keyword>\b[A-Za-z_][A-Za-z0-9_]*\b)";
    private const string Slash =
        @"(?<comment>//[^\n]*|/\*[\s\S]*?\*/)|(?<string>""(?:\\.|[^""\\\n])*""|'(?:\\.|[^'\\\n])*')|(?<number>\b(?:0[xX][0-9a-fA-F_]+|\d[\d_]*(?:\.[\d_]+)?(?:[eE][+-]?\d+)?)\b)|(?<keyword>\b[A-Za-z_][A-Za-z0-9_]*\b)";
    private const string Dash =
        @"(?<comment>--[^\n]*)|(?<string>'(?:''|[^'\n])*'|""(?:\\.|[^""\\\n])*"")|(?<number>\b\d[\d_]*(?:\.[\d_]+)?\b)|(?<keyword>\b[A-Za-z_][A-Za-z0-9_]*\b)";
    private const string Json =
        @"(?<string>""(?:\\.|[^""\\])*"")|(?<number>-?\b(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?\b)|(?<keyword>\b(?:true|false|null)\b)";
    private const string Markup =
        @"(?<comment><!--[\s\S]*?-->)|(?<string>""[^""\n]*""|'[^'\n]*')|(?<keyword></?[a-zA-Z][^\n>]*>)";
    private const string Diff =
        @"(?<keyword>^\+[^\n]*|^-[^\n]*)|(?<comment>^@@[^\n]*)";

    private static string PatternFor(string lang) => lang switch
    {
        "json" => Json,
        "xml" or "html" => Markup,
        "diff" => Diff,
        "sql" => Dash,
        "c" or "cpp" or "rust" or "js" or "ts" or "cs" or "go" or "java" => Slash,
        "python" => Python,
        _ => Hash,
    };
}
