//! Dependency-free syntax highlighter for the languages this monorepo documents.
//!
//! Off-the-shelf highlighters (syntect, highlight.js) have no grammar for KCL or Nushell, which
//! are first-class here, so fenced blocks are tokenised by one generic lexer driven by a
//! [`LangSpec`] table. It is deliberately approximate: the output is documentation, not a
//! compiler front end, and an unknown language degrades to escaped plain text.

use std::fmt::Write as _;

/// Token classes; the `&str` is the CSS class suffix emitted as `tok-<class>`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tok {
    Plain,
    Comment,
    Str,
    Num,
    Keyword,
    Type,
    Literal,
    Func,
    Attr,
    Key,
    Flag,
    Var,
    Punct,
}

impl Tok {
    fn class(self) -> Option<&'static str> {
        match self {
            Tok::Plain => None,
            Tok::Comment => Some("com"),
            Tok::Str => Some("str"),
            Tok::Num => Some("num"),
            Tok::Keyword => Some("kw"),
            Tok::Type => Some("ty"),
            Tok::Literal => Some("lit"),
            Tok::Func => Some("fn"),
            Tok::Attr => Some("attr"),
            Tok::Key => Some("key"),
            Tok::Flag => Some("flag"),
            Tok::Var => Some("var"),
            Tok::Punct => Some("punct"),
        }
    }
}

/// Per-language lexer configuration.
struct LangSpec {
    /// Canonical id, also the fence label shown in the rendered output.
    id: &'static str,
    /// Fence info strings that select this spec (`id` is matched implicitly).
    aliases: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters that open a string literal.
    quotes: &'static [char],
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    literals: &'static [&'static str],
    /// Match keywords case-insensitively (SQL).
    fold_case: bool,
    /// Extra characters that continue an identifier (`-` in Nushell/KCL-style names).
    ident_extra: &'static [char],
    /// `#[attr]` / `#![attr]` runs (Rust).
    rust_attrs: bool,
    /// `name!` macro invocations (Rust).
    macro_bang: bool,
    /// `ident(` is a call site.
    call_syntax: bool,
    /// `$var` sigils (shell, Nushell, Make).
    dollar_vars: bool,
    /// `--flag` / `-f` command-line flags.
    cli_flags: bool,
    /// `key:` (or `key =`, see `key_separators`) at line start is a mapping key.
    mapping_keys: bool,
    /// Characters that may follow a mapping key.
    key_separators: &'static [char],
    /// `[section]` headers at the start of a line (TOML).
    section_headers: bool,
}

const DEFAULT: LangSpec = LangSpec {
    id: "text",
    aliases: &[],
    line_comments: &[],
    block_comment: None,
    quotes: &[],
    keywords: &[],
    types: &[],
    literals: &[],
    fold_case: false,
    ident_extra: &[],
    rust_attrs: false,
    macro_bang: false,
    call_syntax: false,
    dollar_vars: false,
    cli_flags: false,
    mapping_keys: false,
    key_separators: &[':'],
    section_headers: false,
};

const LANGS: &[LangSpec] = &[
    LangSpec {
        id: "rust",
        aliases: &["rs"],
        line_comments: &["///", "//!", "//"],
        block_comment: Some(("/*", "*/")),
        quotes: &['"', '\''],
        keywords: &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "unsafe", "use", "where", "while",
        ],
        types: &[
            "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8",
            "u16", "u32", "u64", "u128", "usize", "String", "Vec", "Option", "Result", "Box",
            "Path", "PathBuf", "BTreeMap", "HashMap", "Arc", "Rc", "Cow",
        ],
        literals: &["None", "Some", "Ok", "Err"],
        call_syntax: true,
        rust_attrs: true,
        macro_bang: true,
        ..DEFAULT
    },
    LangSpec {
        id: "typescript",
        aliases: &["ts", "tsx", "js", "jsx", "javascript"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: &['"', '\'', '`'],
        keywords: &[
            "abstract",
            "as",
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "declare",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "export",
            "extends",
            "finally",
            "for",
            "from",
            "function",
            "get",
            "if",
            "implements",
            "import",
            "in",
            "infer",
            "instanceof",
            "interface",
            "keyof",
            "let",
            "namespace",
            "new",
            "of",
            "private",
            "protected",
            "public",
            "readonly",
            "return",
            "satisfies",
            "set",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "try",
            "type",
            "typeof",
            "var",
            "void",
            "while",
            "yield",
        ],
        types: &[
            "any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown",
            "Array", "Promise", "Record", "Partial", "Readonly", "Map", "Set",
        ],
        literals: &["true", "false", "null", "undefined", "NaN"],
        call_syntax: true,
        dollar_vars: false,
        ..DEFAULT
    },
    LangSpec {
        id: "kcl",
        aliases: &["k"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &[
            "all", "and", "any", "as", "assert", "check", "elif", "else", "filter", "for", "if",
            "import", "in", "is", "lambda", "map", "mixin", "not", "or", "protocol", "rule",
            "schema", "type",
        ],
        types: &["bool", "float", "int", "str", "any"],
        literals: &["None", "Undefined", "True", "False"],
        call_syntax: true,
        mapping_keys: true,
        ..DEFAULT
    },
    LangSpec {
        id: "nushell",
        aliases: &["nu"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\'', '`'],
        keywords: &[
            "alias", "and", "break", "catch", "const", "continue", "def", "do", "else", "export",
            "extern", "for", "hide", "if", "in", "let", "loop", "match", "module", "mut", "not",
            "or", "overlay", "return", "source", "try", "use", "where", "while", "xor",
        ],
        types: &[
            "any",
            "binary",
            "bool",
            "cell-path",
            "closure",
            "datetime",
            "duration",
            "filesize",
            "float",
            "int",
            "list",
            "nothing",
            "path",
            "range",
            "record",
            "string",
            "table",
        ],
        literals: &["true", "false", "null"],
        ident_extra: &['-'],
        dollar_vars: true,
        cli_flags: true,
        ..DEFAULT
    },
    LangSpec {
        id: "bash",
        aliases: &["sh", "shell", "zsh", "console", "just", "justfile"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &[
            "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if",
            "in", "local", "return", "set", "then", "until", "while",
        ],
        types: &[],
        literals: &[],
        ident_extra: &['-'],
        dollar_vars: true,
        cli_flags: true,
        ..DEFAULT
    },
    LangSpec {
        id: "yaml",
        aliases: &["yml"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &[],
        types: &[],
        literals: &["true", "false", "null", "yes", "no", "~"],
        mapping_keys: true,
        ..DEFAULT
    },
    LangSpec {
        id: "toml",
        aliases: &[],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &[],
        types: &[],
        literals: &["true", "false"],
        ident_extra: &['-'],
        mapping_keys: true,
        key_separators: &['=', ':'],
        section_headers: true,
        ..DEFAULT
    },
    LangSpec {
        id: "json",
        aliases: &["jsonc", "nuon"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: &['"'],
        keywords: &[],
        types: &[],
        literals: &["true", "false", "null"],
        mapping_keys: true,
        ..DEFAULT
    },
    LangSpec {
        id: "sql",
        aliases: &["postgres", "psql"],
        line_comments: &["--"],
        block_comment: Some(("/*", "*/")),
        quotes: &['\'', '"'],
        keywords: &[
            "add",
            "all",
            "alter",
            "and",
            "as",
            "asc",
            "begin",
            "between",
            "by",
            "cascade",
            "check",
            "column",
            "commit",
            "constraint",
            "create",
            "default",
            "delete",
            "desc",
            "distinct",
            "drop",
            "exists",
            "foreign",
            "from",
            "grant",
            "group",
            "having",
            "index",
            "inner",
            "insert",
            "into",
            "join",
            "key",
            "left",
            "limit",
            "not",
            "null",
            "on",
            "or",
            "order",
            "primary",
            "references",
            "returning",
            "right",
            "rollback",
            "select",
            "set",
            "table",
            "then",
            "unique",
            "update",
            "using",
            "values",
            "view",
            "where",
            "with",
        ],
        types: &[
            "bigint",
            "bigserial",
            "boolean",
            "bytea",
            "date",
            "double",
            "integer",
            "jsonb",
            "numeric",
            "real",
            "serial",
            "smallint",
            "text",
            "timestamp",
            "timestamptz",
            "uuid",
            "varchar",
        ],
        literals: &["true", "false", "null"],
        fold_case: true,
        ..DEFAULT
    },
];

/// Resolve a fence info string (`rust,no_run` / `ts` / `nu`) to a language spec.
fn spec_for(lang: &str) -> Option<&'static LangSpec> {
    let name = lang
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    LANGS
        .iter()
        .find(|spec| spec.id == name || spec.aliases.contains(&name.as_str()))
}

/// Display label for a fence, e.g. `ts` -> `typescript`. `None` when the fence has no language.
pub fn label_for(lang: &str) -> Option<&'static str> {
    spec_for(lang).map(|spec| spec.id)
}

/// Every fence language the renderer can tokenise, in declaration order. Aliases are omitted:
/// this is the list a lint message offers as the set of accepted spellings.
pub fn known_languages() -> impl Iterator<Item = &'static str> {
    LANGS.iter().map(|spec| spec.id)
}

/// Fence languages that deliberately ask for *no* highlighting. `highlight` already escapes
/// them verbatim; naming them keeps the linter from calling a deliberate choice a typo.
pub fn is_plain(lang: &str) -> bool {
    let name = lang
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(name.as_str(), "text" | "txt" | "plain" | "plaintext")
}

/// HTML-escape `text` into `out`.
pub fn escape_into(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

/// HTML-escape `text`.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    escape_into(&mut out, text);
    out
}

/// Tokenise `code` and return HTML with `<span class="tok-…">` markup.
///
/// Unknown languages are escaped verbatim, so the caller can always use the result.
pub fn highlight(lang: &str, code: &str) -> String {
    let Some(spec) = spec_for(lang) else {
        return escape(code);
    };
    Lexer::new(spec, code).run()
}

struct Lexer {
    spec: &'static LangSpec,
    src: Vec<char>,
    pos: usize,
    out: String,
}

impl Lexer {
    fn new(spec: &'static LangSpec, code: &str) -> Self {
        Self {
            spec,
            src: code.chars().collect(),
            pos: 0,
            out: String::with_capacity(code.len() * 2),
        }
    }

    fn at(&self, offset: usize) -> Option<char> {
        self.src.get(self.pos + offset).copied()
    }

    fn starts_with(&self, needle: &str) -> bool {
        needle
            .chars()
            .enumerate()
            .all(|(i, want)| self.at(i) == Some(want))
    }

    /// True when only whitespace precedes the cursor on the current line.
    fn at_line_start(&self) -> bool {
        self.src[..self.pos]
            .iter()
            .rev()
            .take_while(|c| **c != '\n')
            .all(|c| c.is_whitespace())
    }

    fn push(&mut self, tok: Tok, text: &str) {
        match tok.class() {
            Some(class) => {
                let _ = write!(self.out, "<span class=\"tok-{class}\">");
                escape_into(&mut self.out, text);
                self.out.push_str("</span>");
            }
            None => escape_into(&mut self.out, text),
        }
    }

    fn is_ident_char(&self, ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_' || self.spec.ident_extra.contains(&ch)
    }

    fn run(mut self) -> String {
        while self.pos < self.src.len() {
            if self.comment() || self.string() || self.attribute() || self.header() || self.flag() {
                continue;
            }
            let ch = self.src[self.pos];
            if self.spec.dollar_vars && ch == '$' {
                let start = self.pos;
                self.pos += 1;
                while self.at(0).is_some_and(|c| self.is_ident_char(c)) {
                    self.pos += 1;
                }
                let text: String = self.src[start..self.pos].iter().collect();
                self.push(Tok::Var, &text);
                continue;
            }
            if ch.is_ascii_digit() {
                let start = self.pos;
                while self
                    .at(0)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
                {
                    self.pos += 1;
                }
                let text: String = self.src[start..self.pos].iter().collect();
                self.push(Tok::Num, &text);
                continue;
            }
            if ch.is_alphabetic() || ch == '_' {
                self.word();
                continue;
            }
            self.pos += 1;
            let tok = if "{}[]()<>=+-*/%|&!?:;,.".contains(ch) {
                Tok::Punct
            } else {
                Tok::Plain
            };
            self.push(tok, &ch.to_string());
            continue;
        }
        self.out
    }

    fn comment(&mut self) -> bool {
        for marker in self.spec.line_comments {
            if self.starts_with(marker) {
                let start = self.pos;
                while self.at(0).is_some_and(|c| c != '\n') {
                    self.pos += 1;
                }
                let text: String = self.src[start..self.pos].iter().collect();
                self.push(Tok::Comment, &text);
                return true;
            }
        }
        if let Some((open, close)) = self.spec.block_comment
            && self.starts_with(open)
        {
            let start = self.pos;
            self.pos += open.chars().count();
            while self.pos < self.src.len() && !self.starts_with(close) {
                self.pos += 1;
            }
            self.pos = (self.pos + close.chars().count()).min(self.src.len());
            let text: String = self.src[start..self.pos].iter().collect();
            self.push(Tok::Comment, &text);
            return true;
        }
        false
    }

    fn string(&mut self) -> bool {
        let Some(quote) = self.at(0).filter(|c| self.spec.quotes.contains(c)) else {
            return false;
        };
        // A lone apostrophe inside prose (`don't`) must not open a string.
        if quote == '\''
            && self.at(1).is_some_and(|c| c.is_alphanumeric())
            && self.at(2) != Some('\'')
        {
            let rest_has_close = self.src[self.pos + 1..].contains(&'\'');
            if !rest_has_close {
                return false;
            }
        }
        let start = self.pos;
        self.pos += 1;
        while let Some(ch) = self.at(0) {
            self.pos += 1;
            match ch {
                '\\' => self.pos += 1,
                c if c == quote => break,
                '\n' if quote != '`' => break,
                _ => {}
            }
        }
        self.pos = self.pos.min(self.src.len());
        let text: String = self.src[start..self.pos].iter().collect();
        self.push(Tok::Str, &text);
        true
    }

    fn attribute(&mut self) -> bool {
        if !self.spec.rust_attrs || self.at(0) != Some('#') {
            return false;
        }
        let bracket = if self.at(1) == Some('!') { 2 } else { 1 };
        if self.at(bracket) != Some('[') {
            return false;
        }
        let start = self.pos;
        let mut depth = 0usize;
        while let Some(ch) = self.at(0) {
            self.pos += 1;
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        let text: String = self.src[start..self.pos].iter().collect();
        self.push(Tok::Attr, &text);
        true
    }

    fn header(&mut self) -> bool {
        if !self.spec.section_headers || self.at(0) != Some('[') || !self.at_line_start() {
            return false;
        }
        let start = self.pos;
        while self.at(0).is_some_and(|c| c != '\n') {
            self.pos += 1;
            if self.src[self.pos - 1] == ']' {
                break;
            }
        }
        let text: String = self.src[start..self.pos].iter().collect();
        self.push(Tok::Attr, &text);
        true
    }

    fn flag(&mut self) -> bool {
        if !self.spec.cli_flags || self.at(0) != Some('-') {
            return false;
        }
        let prev_is_word = self
            .pos
            .checked_sub(1)
            .and_then(|i| self.src.get(i))
            .is_some_and(|c| c.is_alphanumeric());
        if prev_is_word || !self.at(1).is_some_and(|c| c.is_alphabetic() || c == '-') {
            return false;
        }
        let start = self.pos;
        while self
            .at(0)
            .is_some_and(|c| c == '-' || c.is_alphanumeric() || c == '_')
        {
            self.pos += 1;
        }
        let text: String = self.src[start..self.pos].iter().collect();
        self.push(Tok::Flag, &text);
        true
    }

    fn word(&mut self) {
        let start = self.pos;
        while self.at(0).is_some_and(|c| self.is_ident_char(c)) {
            self.pos += 1;
        }
        let word: String = self.src[start..self.pos].iter().collect();
        let probe = if self.spec.fold_case {
            word.to_ascii_lowercase()
        } else {
            word.clone()
        };
        let probe = probe.as_str();

        if self.spec.macro_bang && self.at(0) == Some('!') && self.at(1) != Some('=') {
            self.pos += 1;
            let text = format!("{word}!");
            self.push(Tok::Func, &text);
            return;
        }

        let tok = if self.spec.keywords.contains(&probe) {
            Tok::Keyword
        } else if self.spec.literals.contains(&probe) {
            Tok::Literal
        } else if self.spec.types.contains(&probe) {
            Tok::Type
        } else if self.spec.mapping_keys && self.at_line_start_before(start) && self.key_ahead() {
            Tok::Key
        } else if self.spec.call_syntax && self.at(0) == Some('(') {
            Tok::Func
        } else if word
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() && word.chars().any(|c| c.is_lowercase()))
        {
            Tok::Type
        } else {
            Tok::Plain
        };
        self.push(tok, &word);
    }

    /// Whether only whitespace (or list markers) precede `index` on its line.
    fn at_line_start_before(&self, index: usize) -> bool {
        self.src[..index]
            .iter()
            .rev()
            .take_while(|c| **c != '\n')
            .all(|c| c.is_whitespace() || *c == '-')
    }

    /// Whether the identifier just consumed is followed by a key separator.
    fn key_ahead(&self) -> bool {
        let mut i = self.pos;
        while self.src.get(i).is_some_and(|c| *c == ' ') {
            i += 1;
        }
        self.src
            .get(i)
            .is_some_and(|c| self.spec.key_separators.contains(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(html: &str) -> Vec<&str> {
        html.match_indices("<span class=\"tok-")
            .map(|(i, _)| {
                let rest = &html[i + 17..];
                &rest[..rest.find('"').unwrap()]
            })
            .collect()
    }

    #[test]
    fn unknown_language_is_escaped_not_dropped() {
        let html = highlight("brainfuck", "a < b && c > d");
        assert_eq!(html, "a &lt; b &amp;&amp; c &gt; d");
        assert!(classes(&html).is_empty());
    }

    #[test]
    fn fence_aliases_resolve_to_one_spec() {
        assert_eq!(label_for("ts"), Some("typescript"));
        assert_eq!(label_for("tsx"), Some("typescript"));
        assert_eq!(label_for("nu"), Some("nushell"));
        assert_eq!(label_for("rust,no_run"), Some("rust"));
        assert_eq!(label_for(""), None);
    }

    #[test]
    fn rust_attributes_macros_and_strings() {
        let html = highlight("rust", "#[derive(Debug)]\nfn main() { println!(\"hi\"); }");
        assert!(html.contains("<span class=\"tok-attr\">#[derive(Debug)]</span>"));
        assert!(html.contains("<span class=\"tok-fn\">println!</span>"));
        assert!(html.contains("<span class=\"tok-str\">&quot;hi&quot;</span>"));
        assert!(html.contains("<span class=\"tok-kw\">fn</span>"));
    }

    #[test]
    fn kcl_schema_keywords_and_keys() {
        let html = highlight(
            "kcl",
            "schema Server:\n    name: str = \"web\"\n    # comment",
        );
        assert!(html.contains("<span class=\"tok-kw\">schema</span>"));
        assert!(html.contains("<span class=\"tok-key\">name</span>"));
        assert!(html.contains("<span class=\"tok-ty\">str</span>"));
        assert!(html.contains("<span class=\"tok-com\"># comment</span>"));
    }

    #[test]
    fn nushell_variables_and_flags() {
        let html = highlight(
            "nu",
            "def main [] { ls | where size > 1mb --long $env.PWD }",
        );
        assert!(html.contains("<span class=\"tok-kw\">def</span>"));
        assert!(html.contains("<span class=\"tok-flag\">--long</span>"));
        assert!(html.contains("<span class=\"tok-var\">$env</span>"));
    }

    #[test]
    fn typescript_types_and_template_strings() {
        let html = highlight("ts", "export const x: Promise<string> = `a ${b}`;");
        assert!(html.contains("<span class=\"tok-kw\">export</span>"));
        assert!(html.contains("<span class=\"tok-ty\">Promise</span>"));
        assert!(html.contains("<span class=\"tok-str\">`a ${b}`</span>"));
    }

    #[test]
    fn sql_keywords_are_case_insensitive() {
        let upper = highlight("sql", "SELECT id FROM users");
        let lower = highlight("sql", "select id from users");
        assert_eq!(classes(&upper), classes(&lower));
        assert!(upper.contains("<span class=\"tok-kw\">SELECT</span>"));
    }

    #[test]
    fn yaml_keys_and_toml_sections() {
        let yaml = highlight("yaml", "kind: ConfigMap\ndata:\n  schema.sql: |\n");
        assert!(yaml.contains("<span class=\"tok-key\">kind</span>"));
        let toml = highlight("toml", "[package]\nname = \"pg-cli\"");
        assert!(toml.contains("<span class=\"tok-attr\">[package]</span>"));
        assert!(toml.contains("<span class=\"tok-key\">name</span>"));
    }

    #[test]
    fn unterminated_string_does_not_run_past_end() {
        // Regression guard: the lexer must terminate on malformed input.
        let html = highlight("rust", "let s = \"oops");
        assert!(html.contains("<span class=\"tok-str\">&quot;oops</span>"));
    }
}
