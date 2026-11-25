//! Opdeler kildetekst i farvede stykker. Ren tekstbehandling uden Win32, så den kan testes.
//!
//! Det er ikke en parser: den kender ord, strenge, tal og kommentarer, og gætter på
//! typer og funktionskald ud fra hvordan navnet ser ud og hvad der står efter det.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    JavaScript,
    TypeScript,
    Sql,
    Json,
    Html,
    Css,
    Python,
    Rust,
    Markdown,
    CSharp,
    Php,
    Yaml,
    Go,
    Java,
    Xml,
    C,
    Cpp,
    Bash,
    Kotlin,
    Swift,
    Ruby,
    PowerShell,
    Lua,
    Toml,
    Dockerfile,
    Ini,
    Batch,
    Env,
    Vue,
    Svelte,
    Scss,
}

impl Lang {
    pub fn of(path: &Path) -> Option<Lang> {
        // Skallens opsætningsfiler har intet efternavn.
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(name, ".bashrc" | ".bash_profile" | ".bash_aliases" | ".profile" | ".zshrc" | ".zprofile") {
            return Some(Lang::Bash);
        }
        if matches!(name, "Gemfile" | "Rakefile" | "Vagrantfile" | "Podfile" | "Brewfile") {
            return Some(Lang::Ruby);
        }
        if matches!(name, "Cargo.lock" | "poetry.lock" | "uv.lock") {
            return Some(Lang::Toml);
        }
        if matches!(name, ".editorconfig" | ".gitconfig" | ".npmrc") {
            return Some(Lang::Ini);
        }
        // .env, .env.local, .env.production ...
        if name == ".env" || name.starts_with(".env.") {
            return Some(Lang::Env);
        }
        // Dockerfile, Dockerfile.dev og Containerfile
        let lower = name.to_ascii_lowercase();
        if lower == "containerfile" || lower == "dockerfile" || lower.starts_with("dockerfile.") {
            return Some(Lang::Dockerfile);
        }
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "js" | "mjs" | "cjs" | "jsx" => Some(Lang::JavaScript),
            "ts" | "mts" | "cts" | "tsx" => Some(Lang::TypeScript),
            "sql" => Some(Lang::Sql),
            "json" | "jsonc" | "json5" => Some(Lang::Json),
            "html" | "htm" => Some(Lang::Html),
            "css" => Some(Lang::Css),
            "py" | "pyw" | "pyi" => Some(Lang::Python),
            "rs" => Some(Lang::Rust),
            "md" | "markdown" => Some(Lang::Markdown),
            "cs" | "csx" => Some(Lang::CSharp),
            "php" | "phtml" => Some(Lang::Php),
            "yml" | "yaml" => Some(Lang::Yaml),
            "go" => Some(Lang::Go),
            "c" | "h" => Some(Lang::C),
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" => Some(Lang::Cpp),
            "sh" | "bash" | "zsh" | "ksh" | "shell" => Some(Lang::Bash),
            "kt" | "kts" | "kotlin" => Some(Lang::Kotlin),
            "swift" => Some(Lang::Swift),
            "rb" | "rake" | "gemspec" | "ru" | "ruby" => Some(Lang::Ruby),
            "ps1" | "psm1" | "psd1" | "powershell" | "pwsh" => Some(Lang::PowerShell),
            "lua" => Some(Lang::Lua),
            "toml" => Some(Lang::Toml),
            "dockerfile" | "docker" => Some(Lang::Dockerfile),
            "ini" | "cfg" | "conf" | "properties" | "inf" => Some(Lang::Ini),
            "bat" | "cmd" | "batch" => Some(Lang::Batch),
            "env" | "dotenv" => Some(Lang::Env),
            "vue" => Some(Lang::Vue),
            "svelte" => Some(Lang::Svelte),
            "scss" | "less" => Some(Lang::Scss),
            "java" => Some(Lang::Java),
            "xml" | "svg" | "xsd" | "xsl" | "xslt" | "xaml" | "csproj" | "props" | "targets" | "config" | "resx" | "plist" => {
                Some(Lang::Xml)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Keyword,
    Str,
    Number,
    Comment,
    Type,
    Function,
}

/// Et farvet stykke: tegnpositioner i UTF-16-enheder, `end` er ikke med.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

pub fn tokenize(lang: Lang, text: &[u16]) -> Vec<Token> {
    let mut scanner = Scanner { text, at: 0, out: Vec::with_capacity(text.len() / 6) };
    scanner.run(lang);
    scanner.out
}

const CR: u16 = 13;

fn is(c: u16, ch: u8) -> bool {
    c == ch as u16
}

fn is_space(c: u16) -> bool {
    c == 32 || c == 9 || c == CR || c == 10 || c == 11
}

fn is_digit(c: u16) -> bool {
    (b'0' as u16..=b'9' as u16).contains(&c)
}

fn is_upper(c: u16) -> bool {
    (b'A' as u16..=b'Z' as u16).contains(&c)
}

fn is_word_start(c: u16) -> bool {
    is_upper(c) || (b'a' as u16..=b'z' as u16).contains(&c) || is(c, b'_') || is(c, b'$') || c >= 0x80
}

fn is_word(c: u16) -> bool {
    is_word_start(c) || is_digit(c)
}

/// Hvad der sidst stod før det aktuelle sted; afgør om `/` starter et regulært udtryk.
#[derive(Clone, Copy, PartialEq)]
enum Before {
    /// Starten af teksten, en operator eller et nøgleord: her kan et udtryk begynde.
    Operator,
    /// Et navn, et tal, en streng eller en lukkende parentes: her er `/` division.
    Value,
}

/// Hvilken slags opmærkning `Scanner::markup` læser.
#[derive(Clone, Copy, PartialEq)]
enum Markup {
    Html,
    Xml,
    Vue,
    Svelte,
}

struct Scanner<'a> {
    text: &'a [u16],
    at: usize,
    out: Vec<Token>,
}

impl Scanner<'_> {
    fn peek(&self, ahead: usize) -> u16 {
        self.text.get(self.at + ahead).copied().unwrap_or(0)
    }

    fn push(&mut self, start: usize, kind: Kind) {
        if self.at > start {
            self.out.push(Token { start, end: self.at, kind });
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        s.bytes().enumerate().all(|(i, b)| is(self.peek(i), b))
    }

    /// Første tegn efter mellemrum fra `from`.
    fn next_visible(&self, from: usize) -> u16 {
        self.text[from..].iter().copied().find(|&c| !is_space(c)).unwrap_or(0)
    }

    fn skip_line(&mut self) {
        while self.at < self.text.len() && self.text[self.at] != CR {
            self.at += 1;
        }
    }

    fn block_comment(&mut self) {
        let start = self.at;
        self.at += 2;
        while self.at < self.text.len() && !self.starts_with("*/") {
            self.at += 1;
        }
        self.at = (self.at + 2).min(self.text.len());
        self.push(start, Kind::Comment);
    }

    fn line_comment(&mut self) {
        let start = self.at;
        self.skip_line();
        self.push(start, Kind::Comment);
    }

    /// Ordet der starter her, i små bogstaver hvis det er kort og rent ASCII.
    fn word(&mut self, buffer: &mut [u8; 24]) -> (usize, usize) {
        let start = self.at;
        while self.at < self.text.len() && is_word(self.text[self.at]) {
            self.at += 1;
        }
        let word = &self.text[start..self.at];
        if word.len() > buffer.len() || word.iter().any(|&c| c >= 0x80) {
            return (start, 0);
        }
        for (slot, &c) in buffer.iter_mut().zip(word) {
            *slot = c as u8;
        }
        (start, word.len())
    }

    fn number(&mut self) {
        let start = self.at;
        let radix_prefix = is(self.peek(0), b'0') && matches!(self.peek(1) as u8 | 0x20, b'x' | b'b' | b'o');
        let mut seen_dot = false;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let exponent = !radix_prefix && self.at > start && matches!(self.text[self.at - 1] as u8 | 0x20, b'e');
            if is_digit(c) || is(c, b'_') || (c < 0x80 && (c as u8).is_ascii_alphabetic()) {
                self.at += 1;
            } else if is(c, b'.') && !seen_dot && !radix_prefix && is_digit(self.peek(1)) {
                seen_dot = true;
                self.at += 1;
            } else if is(c, b'.') && !seen_dot && !radix_prefix && !is(self.peek(1), b'.') && !is_word_start(self.peek(1)) {
                // "1." er et tal; "1..toFixed" og "1.toFixed" er ikke.
                seen_dot = true;
                self.at += 1;
            } else if (is(c, b'+') || is(c, b'-')) && exponent && is_digit(self.peek(1)) {
                self.at += 1;
            } else {
                break;
            }
        }
        self.push(start, Kind::Number);
    }

    // ---------- JavaScript og TypeScript ----------

    fn script(&mut self, typescript: bool) {
        let mut before = Before::Operator;
        // Åbne `${`-udtryk i template-strenge: antal åbne `{` inde i hvert.
        let mut templates: Vec<u32> = Vec::new();
        if self.starts_with("#!") {
            self.line_comment();
        }
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if is(c, b'"') || is(c, b'\'') {
                self.quoted(c, true);
                before = Before::Value;
            } else if is(c, b'`') {
                let start = self.at;
                self.at += 1;
                self.template(start, &mut templates);
                before = Before::Value;
            } else if is(c, b'}') && templates.last() == Some(&0) {
                // Slut på et `${ ... }`: resten af template-strengen fortsætter.
                templates.pop();
                let start = self.at;
                self.at += 1;
                self.push(start, Kind::Keyword);
                let rest = self.at;
                self.template(rest, &mut templates);
                before = Before::Value;
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
                before = Before::Value;
            } else if is_word_start(c) {
                before = self.script_word(typescript);
            } else if is(c, b'/') && before == Before::Operator && self.regex() {
                before = Before::Value;
            } else {
                if is(c, b'{') {
                    if let Some(depth) = templates.last_mut() {
                        *depth += 1;
                    }
                } else if is(c, b'}') {
                    if let Some(depth) = templates.last_mut() {
                        *depth -= 1;
                    }
                }
                before = if is(c, b')') || is(c, b']') { Before::Value } else { Before::Operator };
                self.at += 1;
            }
        }
    }

    /// En streng i enkelte eller dobbelte anførselstegn. Slutter senest ved linjens ende.
    fn quoted(&mut self, quote: u16, backslash_escapes: bool) {
        let start = self.at;
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if c == CR {
                break;
            }
            self.at += 1;
            if backslash_escapes && is(c, b'\\') && self.at < self.text.len() && self.text[self.at] != CR {
                self.at += 1;
            } else if c == quote {
                break;
            }
        }
        self.push(start, Kind::Str);
    }

    /// Indholdet af en template-streng fra `start`, til den slutter eller et `${` åbner.
    fn template(&mut self, start: usize, templates: &mut Vec<u32>) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is(c, b'\\') {
                self.at = (self.at + 2).min(self.text.len());
            } else if is(c, b'`') {
                self.at += 1;
                self.push(start, Kind::Str);
                return;
            } else if self.starts_with("${") {
                self.push(start, Kind::Str);
                let open = self.at;
                self.at += 2;
                self.push(open, Kind::Keyword);
                templates.push(0);
                return;
            } else {
                self.at += 1;
            }
        }
        self.push(start, Kind::Str);
    }

    /// Et regulært udtryk som `/ab+c/gi`. Returnerer false, hvis det ikke er et.
    fn regex(&mut self) -> bool {
        let start = self.at;
        let mut at = self.at + 1;
        let mut in_class = false;
        loop {
            let Some(&c) = self.text.get(at) else { return false };
            if c == CR {
                return false;
            }
            at += 1;
            if is(c, b'\\') {
                at += 1;
            } else if is(c, b'[') {
                in_class = true;
            } else if is(c, b']') {
                in_class = false;
            } else if is(c, b'/') && !in_class {
                break;
            }
        }
        while self.text.get(at).is_some_and(|&c| is_word(c)) {
            at += 1;
        }
        self.at = at.min(self.text.len());
        self.push(start, Kind::Str);
        true
    }

    fn script_word(&mut self, typescript: bool) -> Before {
        // Efter et punktum er ordet en egenskab ("x.default"), men "...this" er ikke.
        let member = self.at > 0
            && is(self.text[self.at - 1], b'.')
            && !(self.at > 1 && is(self.text[self.at - 2], b'.'));
        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
        let next = self.next_visible(self.at);
        let called = is(next, b'(');

        if !member {
            // Ord som "from" og "type" er kun nøgleord, når der følger noget efter dem,
            // der passer; ellers er de almindelige navne.
            let introduces = is_word_start(next) || [b'{', b'[', b'*', b'"', b'\'', b'`'].iter().any(|&b| is(next, b));
            let keyword = script_keyword(word)
                || (typescript && typescript_keyword(word))
                || (introduces && (contextual_keyword(word) || (typescript && typescript_contextual(word))));
            if keyword {
                self.push(start, Kind::Keyword);
                // Efter "this" og "super" er `/` division; efter "return" starter et udtryk.
                return if matches!(word, "this" | "super") { Before::Value } else { Before::Operator };
            }
            if matches!(word, "true" | "false" | "null" | "undefined" | "NaN" | "Infinity") {
                self.push(start, Kind::Number);
                return Before::Value;
            }
            if typescript && typescript_type(word) {
                self.push(start, Kind::Type);
                return Before::Value;
            }
        }

        let name = &self.text[start..self.at];
        let constant = name.len() > 1 && name.iter().all(|&c| is_upper(c) || is_digit(c) || is(c, b'_'));
        if is_upper(name[0]) && !constant {
            self.push(start, Kind::Type);
        } else if called {
            self.push(start, Kind::Function);
        }
        Before::Value
    }

    // ---------- SQL ----------

    fn sql(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("--") {
                self.line_comment();
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if is(c, b'\'') {
                self.sql_string();
            } else if is(c, b'"') || is(c, b'`') {
                // Navne i anførselstegn farves ikke, men ordene i dem er heller ikke nøgleord.
                self.skip_to(c);
            } else if is(c, b'[') {
                self.skip_to(b']' as u16);
            } else if is(c, b'$') && self.dollar_quote() {
                // Mærket er farvet; fortsæt med koden efter det.
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is_word_start(c) {
                self.sql_word();
            } else if (is(c, b'@') || is(c, b':') || is(c, b'#'))
                && is_word_start(self.peek(1))
                && !(self.at > 0 && self.text[self.at - 1] == c)
            {
                // Variabler og parametre (@id, :navn, #temp) er navne, ikke nøgleord.
                // Efter "::" står der derimod en type, som skal farves.
                self.at += 1;
                while self.at < self.text.len() && is_word(self.text[self.at]) {
                    self.at += 1;
                }
            } else {
                self.at += 1;
            }
        }
    }

    /// En SQL-streng. Et dobbelt `''` er et anførselstegn i strengen, og den må gå over
    /// flere linjer.
    fn sql_string(&mut self) {
        let start = self.at;
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            self.at += 1;
            if is(c, b'\'') {
                if is(self.peek(0), b'\'') {
                    self.at += 1;
                } else {
                    break;
                }
            }
        }
        self.push(start, Kind::Str);
    }

    fn skip_to(&mut self, close: u16) {
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if c == CR {
                return;
            }
            self.at += 1;
            if c == close {
                return;
            }
        }
    }

    /// Postgres' `$$` og `$navn$` omkring funktionskroppe. Kun selve mærket farves,
    /// så koden imellem stadig bliver farvet som SQL.
    fn dollar_quote(&mut self) -> bool {
        let mut at = self.at + 1;
        while self.text.get(at).is_some_and(|&c| is_word(c) && !is(c, b'$')) {
            at += 1;
        }
        if !self.text.get(at).is_some_and(|&c| is(c, b'$')) || is_digit(self.peek(1)) {
            return false;
        }
        let start = self.at;
        self.at = at + 1;
        self.push(start, Kind::Str);
        true
    }

    fn sql_word(&mut self) {
        // N'tekst' og E'tekst' er strenge med et præfiks.
        if matches!(self.peek(0) as u8 | 0x20, b'n' | b'e' | b'b' | b'x') && is(self.peek(1), b'\'') {
            let start = self.at;
            self.at += 1;
            self.sql_string();
            let string = self.out.pop().map_or(start, |t| t.end);
            self.out.push(Token { start, end: string, kind: Kind::Str });
            return;
        }
        let member = self.at > 0 && is(self.text[self.at - 1], b'.');
        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        buffer[..len].make_ascii_lowercase();
        let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
        if !member && sql_keyword(word) {
            self.push(start, Kind::Keyword);
        } else if !member && sql_type(word) {
            self.push(start, Kind::Type);
        } else if is(self.next_visible(self.at), b'(') {
            self.push(start, Kind::Function);
        }
    }

    fn run(&mut self, lang: Lang) {
        match lang {
            Lang::JavaScript => self.script(false),
            Lang::TypeScript => self.script(true),
            Lang::Sql => self.sql(),
            Lang::Json => self.json(),
            Lang::Html => self.html(),
            Lang::Css => self.css(),
            Lang::Python => self.python(),
            Lang::Rust => self.rust(),
            Lang::Markdown => self.markdown(),
            Lang::CSharp => self.csharp(),
            Lang::Php => self.php(),
            Lang::Yaml => self.yaml(),
            Lang::Go => self.go(),
            Lang::Java => self.java(),
            Lang::Xml => self.markup(Markup::Xml),
            Lang::C => self.c_like(false),
            Lang::Cpp => self.c_like(true),
            Lang::Bash => self.bash(),
            Lang::Kotlin => self.curly(&KOTLIN),
            Lang::Swift => self.curly(&SWIFT),
            Lang::Ruby => self.ruby(),
            Lang::PowerShell => self.powershell(),
            Lang::Lua => self.lua(),
            Lang::Toml => self.toml(),
            Lang::Dockerfile => self.dockerfile(),
            Lang::Ini => self.ini(),
            Lang::Batch => self.batch(),
            Lang::Env => self.env(),
            Lang::Vue => self.markup(Markup::Vue),
            Lang::Svelte => self.markup(Markup::Svelte),
            Lang::Scss => self.stylesheet(true),
        }
    }

    // ---------- Python ----------

    fn python(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is_space(c) {
                self.at += 1;
            } else if is(c, b'#') {
                self.line_comment();
            } else if is(c, b'"') || is(c, b'\'') {
                self.python_string(self.at);
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is(c, b'@') && is_word_start(self.peek(1)) && self.first_on_line() {
                // En dekorator: @app.route
                let start = self.at;
                self.at += 1;
                while self.at < self.text.len() && (is_word(self.text[self.at]) || is(self.text[self.at], b'.')) {
                    self.at += 1;
                }
                self.push(start, Kind::Function);
            } else if is_word_start(c) {
                self.python_word();
            } else {
                self.at += 1;
            }
        }
    }

    /// Om der kun står mellemrum mellem linjens start og her.
    fn first_on_line(&self) -> bool {
        self.text[..self.at].iter().rev().find(|&&c| c != 32 && c != 9).is_none_or(|&c| c == CR)
    }

    /// En streng der starter med anførselstegnet ved `self.at`; `start` er før et
    /// eventuelt præfiks som `f` eller `rb`. Tre anførselstegn må gå over flere linjer.
    fn python_string(&mut self, start: usize) {
        let quote = self.text[self.at];
        let triple = self.peek(1) == quote && self.peek(2) == quote;
        self.at += if triple { 3 } else { 1 };
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is(c, b'\\') {
                self.at += 2;
            } else if triple && c == quote && self.peek(1) == quote && self.peek(2) == quote {
                self.at += 3;
                break;
            } else if !triple && (c == quote || c == CR) {
                self.at += (c == quote) as usize;
                break;
            } else {
                self.at += 1;
            }
        }
        self.at = self.at.min(self.text.len());
        self.push(start, Kind::Str);
    }

    fn python_word(&mut self) {
        // r"...", f'...', rb"..." og lignende er strenge med et præfiks.
        let mut prefix = 0;
        while prefix < 2 && matches!(self.peek(prefix) as u8 | 0x20, b'r' | b'b' | b'f' | b'u') && self.peek(prefix) < 0x80 {
            prefix += 1;
        }
        if prefix > 0 && (is(self.peek(prefix), b'"') || is(self.peek(prefix), b'\'')) {
            let start = self.at;
            self.at += prefix;
            self.python_string(start);
            return;
        }

        let member = self.at > 0 && is(self.text[self.at - 1], b'.');
        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
        let next = self.next_visible(self.at);
        if !member {
            // "match", "case" og "type" er kun nøgleord, når der følger et udtryk efter dem.
            let introduces = is_word_start(next) || is_digit(next) || [b'[', b'{', b'"', b'\''].iter().any(|&b| is(next, b));
            if python_keyword(word) || (introduces && matches!(word, "match" | "case" | "type")) {
                self.push(start, Kind::Keyword);
                return;
            }
            if matches!(word, "True" | "False" | "None") {
                self.push(start, Kind::Number);
                return;
            }
            if python_type(word) {
                self.push(start, Kind::Type);
                return;
            }
        }
        self.name(start, is(next, b'('));
    }

    /// Farver et almindeligt navn: `Stort` begyndelsesbogstav er en type, `KONSTANTER`
    /// får ingen farve, og et navn fulgt af en parentes er et funktionskald.
    fn name(&mut self, start: usize, called: bool) {
        let name = &self.text[start..self.at];
        let constant = name.len() > 1 && name.iter().all(|&c| is_upper(c) || is_digit(c) || is(c, b'_'));
        if is_upper(name[0]) && !constant {
            self.push(start, Kind::Type);
        } else if called {
            self.push(start, Kind::Function);
        }
    }

    // ---------- Rust ----------

    fn rust(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                // Rusts blokkommentarer kan ligge inden i hinanden.
                let start = self.at;
                let mut depth = 0u32;
                while self.at < self.text.len() {
                    if self.starts_with("/*") {
                        depth += 1;
                        self.at += 2;
                    } else if self.starts_with("*/") {
                        depth -= 1;
                        self.at += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        self.at += 1;
                    }
                }
                self.push(start, Kind::Comment);
            } else if is(c, b'"') {
                self.rust_string(self.at);
            } else if is(c, b'\'') {
                self.rust_quote(self.at);
            } else if self.starts_with("#[") || self.starts_with("#![") {
                // En attribut som #[derive(Debug)]. Den farves til sin `]` på samme linje.
                let start = self.at;
                let mut depth = 0u32;
                while self.at < self.text.len() && self.text[self.at] != CR {
                    let c = self.text[self.at];
                    self.at += 1;
                    if is(c, b'[') {
                        depth += 1;
                    } else if is(c, b']') {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                self.push(start, Kind::Type);
            } else if is_digit(c) {
                self.number();
            } else if is_word_start(c) {
                self.rust_word();
            } else {
                self.at += 1;
            }
        }
    }

    /// En streng fra anførselstegnet ved `self.at`. Den må gå over flere linjer.
    fn rust_string(&mut self, start: usize) {
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            self.at += if is(c, b'\\') { 2 } else { 1 };
            if is(c, b'"') {
                break;
            }
        }
        self.at = self.at.min(self.text.len());
        self.push(start, Kind::Str);
    }

    /// `'a'` er et tegn, `'a` er en levetid.
    fn rust_quote(&mut self, start: usize) {
        let (first, second) = (self.peek(1), self.peek(2));
        if is(first, b'\\') {
            // '\n', '\'' og '\u{1F600}'
            let mut at = self.at + 3;
            while at < self.text.len() && at < self.at + 14 && self.text[at] != CR && !is(self.text[at], b'\'') {
                at += 1;
            }
            if self.text.get(at).is_some_and(|&c| is(c, b'\'')) {
                self.at = at + 1;
                self.push(start, Kind::Str);
            } else {
                self.at += 1;
            }
        } else if first != 0 && first != CR && is(second, b'\'') {
            self.at += 3;
            self.push(start, Kind::Str);
        } else if (0xD800..0xDC00).contains(&first) && is(self.peek(3), b'\'') {
            // Et tegn uden for BMP fylder to enheder.
            self.at += 4;
            self.push(start, Kind::Str);
        } else if is_word_start(first) {
            self.at += 1;
            while self.at < self.text.len() && is_word(self.text[self.at]) {
                self.at += 1;
            }
            self.push(start, Kind::Type);
        } else {
            self.at += 1;
        }
    }

    fn rust_word(&mut self) {
        let c = self.text[self.at];
        let start = self.at;
        // b"bytes", b'x', r"rå", r#"rå"# og br#"..."#
        if is(c, b'b') && is(self.peek(1), b'"') {
            self.at += 1;
            self.rust_string(start);
            return;
        }
        if is(c, b'b') && is(self.peek(1), b'\'') {
            self.at += 1;
            self.rust_quote(start);
            return;
        }
        let raw = if is(c, b'r') { 1 } else if is(c, b'b') && is(self.peek(1), b'r') { 2 } else { 0 };
        if raw > 0 {
            let mut hashes = 0;
            while is(self.peek(raw + hashes), b'#') {
                hashes += 1;
            }
            if is(self.peek(raw + hashes), b'"') {
                self.at += raw + hashes + 1;
                // Slutter ved et anførselstegn fulgt af lige så mange #.
                while self.at < self.text.len() {
                    let closes = is(self.text[self.at], b'"') && (1..=hashes).all(|i| is(self.peek(i), b'#'));
                    if closes {
                        self.at += 1 + hashes;
                        break;
                    }
                    self.at += 1;
                }
                self.push(start, Kind::Str);
                return;
            }
        }

        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
        if rust_keyword(word) {
            self.push(start, Kind::Keyword);
        } else if matches!(word, "true" | "false") {
            self.push(start, Kind::Number);
        } else if rust_type(word) {
            self.push(start, Kind::Type);
        } else if is(self.peek(0), b'!') && !is(self.peek(1), b'=') {
            // En makro: println!
            self.at += 1;
            self.push(start, Kind::Function);
        } else {
            // Navnet efter "fn" er en funktion, også når der følger <T> og ikke en parentes.
            let declared = self.preceded_by(start, "fn");
            let called = is(self.next_visible(self.at), b'(');
            self.name(start, called || declared);
        }
    }

    /// Om ordet lige før `start` (med mellemrum imellem) er `word`.
    fn preceded_by(&self, start: usize, word: &str) -> bool {
        let before = self.text[..start].iter().rposition(|&c| !is_space(c)).map_or(0, |i| i + 1);
        let len = word.len();
        before < start
            && before >= len
            && self.text[before - len..before].iter().zip(word.bytes()).all(|(&c, b)| is(c, b))
            && (before == len || !is_word(self.text[before - len - 1]))
    }

    /// En streng fra anførselstegnet ved `self.at` til det næste af samme slags, også
    /// over flere linjer. `\` foran et tegn tager det med.
    fn string_until(&mut self, start: usize, quote: u16) {
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            self.at += if is(c, b'\\') { 2 } else { 1 };
            if c == quote {
                break;
            }
        }
        self.at = self.at.min(self.text.len());
        self.push(start, Kind::Str);
    }

    // ---------- Dockerfile ----------

    fn dockerfile(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is_space(c) {
                self.at += 1;
                continue;
            }
            if is(c, b'#') {
                self.line_comment();
                continue;
            }
            // Første ord på linjen er instruktionen: FROM, RUN, COPY ...
            let mut buffer = [0u8; 24];
            let (start, len) = self.word(&mut buffer);
            buffer[..len].make_ascii_lowercase();
            let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
            if dockerfile_instruction(word) {
                self.push(start, Kind::Keyword);
            }
            if word == "onbuild" {
                // ONBUILD står foran en anden instruktion.
                continue;
            }

            // Instruktionen fortsætter over de linjer, der ender med `\`. Kommentarer og
            // tomme linjer midt i den hører med.
            let mut line = self.at;
            let mut end = self.line_end(line);
            loop {
                let content = &self.text[line..end];
                let first = content.iter().find(|&&c| c != 32 && c != 9);
                let last = content.iter().rfind(|&&c| c != 32 && c != 9);
                let continues = last.is_some_and(|&c| is(c, b'\\'));
                let passes = line != self.at && first.is_none_or(|&c| is(c, b'#'));
                if end >= self.text.len() || !(continues || passes) {
                    break;
                }
                line = end + 1;
                end = self.line_end(line);
            }

            if word == "run" {
                // Det der køres er skalkommandoer.
                self.embedded(end, |inner| inner.bash());
                continue;
            }
            while self.at < end {
                let c = self.text[self.at];
                let start = self.at;
                if is_space(c) || is(c, b'\\') {
                    self.at += 1;
                } else if is(c, b'"') {
                    self.bash_double_quoted();
                } else if is(c, b'\'') {
                    self.at = self.find_before(start + 1, end, "'").map_or(end, |close| close + 1);
                    self.push(start, Kind::Str);
                } else if is(c, b'$') {
                    if !self.bash_variable() {
                        self.at += 1;
                    }
                } else {
                    while self.at < end && !is_space(self.text[self.at]) && ![b'"', b'\'', b'$'].iter().any(|&b| is(self.text[self.at], b)) {
                        self.at += 1;
                    }
                    let value = &self.text[start..self.at];
                    let is_as = value.len() == 2 && matches!(value[0] as u8 | 0x20, b'a') && matches!(value[1] as u8 | 0x20, b's');
                    if word == "from" && is_as {
                        self.push(start, Kind::Keyword);
                    } else if value.iter().all(|&c| is_digit(c)) {
                        self.push(start, Kind::Number);
                    }
                }
            }
            self.at = self.at.max(end);
        }
    }

    // ---------- .env ----------

    fn env(&mut self) {
        let blank = |c: u16| c == 32 || c == 9;
        while self.at < self.text.len() {
            let mut end = self.line_end(self.at);
            while self.at < end && blank(self.text[self.at]) {
                self.at += 1;
            }
            let start = self.at;
            if self.at < end && is(self.text[self.at], b'#') {
                self.at = end;
                self.push(start, Kind::Comment);
            } else if self.at < end {
                // "export NAVN=værdi" er tilladt, så filen også kan læses af en skal.
                if self.starts_with("export") && blank(self.peek(6)) {
                    self.at += 6;
                    self.push(start, Kind::Keyword);
                    while self.at < end && blank(self.text[self.at]) {
                        self.at += 1;
                    }
                }
                if let Some(equals) = self.find_before(self.at, end, "=") {
                    let key = self.at;
                    self.at = equals;
                    while self.at > key && blank(self.text[self.at - 1]) {
                        self.at -= 1;
                    }
                    self.push(key, Kind::Keyword);
                    self.at = equals + 1;
                    while self.at < end && blank(self.text[self.at]) {
                        self.at += 1;
                    }
                    end = self.env_value(end);
                    // En kommentar efter værdien.
                    while self.at < end && blank(self.text[self.at]) {
                        self.at += 1;
                    }
                    if self.at < end && is(self.text[self.at], b'#') {
                        let comment = self.at;
                        self.at = end;
                        self.push(comment, Kind::Comment);
                    }
                }
            }
            self.at = (self.at.max(end) + 1).min(self.text.len());
        }
    }

    /// Værdien efter `=`. Returnerer linjens ende, som kan være flyttet, hvis en værdi
    /// i anførselstegn går over flere linjer.
    fn env_value(&mut self, end: usize) -> usize {
        let start = self.at;
        if start >= end {
            return end;
        }
        let c = self.text[start];
        if is(c, b'"') {
            // Variabler i dobbelte anførselstegn bliver sat ind, så de får deres egen farve.
            self.bash_double_quoted();
            return self.line_end(self.at);
        }
        if is(c, b'\'') {
            self.at = self.find_before(start + 1, self.text.len(), "'").map_or(self.text.len(), |close| close + 1);
            self.push(start, Kind::Str);
            return self.line_end(self.at);
        }
        // Uden anførselstegn slutter værdien ved linjens ende eller ved " #".
        let mut stop = start;
        while stop < end && !(is(self.text[stop], b'#') && (self.text[stop - 1] == 32 || self.text[stop - 1] == 9)) {
            stop += 1;
        }
        while stop > start && (self.text[stop - 1] == 32 || self.text[stop - 1] == 9) {
            stop -= 1;
        }
        if yaml_literal(&self.text[start..stop]) {
            self.at = stop;
            self.push(start, Kind::Number);
            return end;
        }
        let mut piece = start;
        while self.at < stop {
            if is(self.text[self.at], b'$') {
                let dollar = self.at;
                self.push(piece, Kind::Str);
                if self.bash_variable() {
                    piece = self.at;
                } else {
                    self.at += 1;
                    piece = dollar;
                }
            } else {
                self.at += 1;
            }
        }
        self.at = self.at.max(stop);
        let rest = self.at;
        self.at = stop.max(piece);
        self.push(piece, Kind::Str);
        self.at = rest;
        end
    }

    // ---------- INI ----------

    fn ini(&mut self) {
        while self.at < self.text.len() {
            let end = self.line_end(self.at);
            let mut at = self.at;
            while at < end && (self.text[at] == 32 || self.text[at] == 9) {
                at += 1;
            }
            let c = if at < end { self.text[at] } else { 0 };
            self.at = end;
            if is(c, b';') || is(c, b'#') {
                self.push(at, Kind::Comment);
            } else if is(c, b'[') {
                // [sektion]
                let close = self.find_before(at, end, "]").map_or(end, |found| found + 1);
                self.at = close;
                self.push(at, Kind::Function);
            } else if let Some(equals) = self.find_before(at, end, "=") {
                // nøgle = værdi
                let mut stop = equals;
                while stop > at && (self.text[stop - 1] == 32 || self.text[stop - 1] == 9) {
                    stop -= 1;
                }
                self.at = stop;
                self.push(at, Kind::Keyword);

                let mut value = equals + 1;
                while value < end && (self.text[value] == 32 || self.text[value] == 9) {
                    value += 1;
                }
                let mut stop = end;
                while stop > value && (self.text[stop - 1] == 32 || self.text[stop - 1] == 9) {
                    stop -= 1;
                }
                self.at = stop;
                let literal = yaml_literal(&self.text[value..stop]);
                self.push(value, if literal { Kind::Number } else { Kind::Str });
            }
            self.at = (end + 1).min(self.text.len());
        }
    }

    // ---------- Batch ----------

    fn batch(&mut self) {
        // Om det næste ord står på en kommandos plads (først på linjen, efter `&`, `do` ...).
        let mut command = true;
        // Efter "echo" og efter "=" i "set" er resten af linjen tekst, ikke kommandoer.
        let mut text = false;
        let mut in_set = false;
        let mut after_in = false;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if c == CR {
                self.at += 1;
                command = true;
                text = false;
                in_set = false;
                after_in = false;
            } else if is_space(c) {
                self.at += 1;
            } else if is(c, b'^') {
                self.at = (self.at + 2).min(self.text.len());
            } else if is(c, b'%') || is(c, b'!') {
                if !self.batch_variable() {
                    self.at += 1;
                }
                command = false;
            } else if text {
                if [b'&', b'|', b'<', b'>'].iter().any(|&b| is(c, b)) {
                    text = false;
                    command = is(c, b'&') || is(c, b'|');
                }
                self.at += 1;
            } else if is(c, b'"') {
                self.batch_string();
                command = false;
            } else if is(c, b'@') && command {
                self.at += 1;
            } else if is(c, b':') && command {
                // ":: tekst" er en kommentar, ":navn" er et mærke man kan hoppe til.
                let comment = is(self.peek(1), b':');
                self.skip_line();
                self.push(start, if comment { Kind::Comment } else { Kind::Function });
            } else if is(c, b':') && is_word_start(self.peek(1)) && is_space(self.text[start - 1]) {
                // call :mærke
                self.at += 1;
                while self.at < self.text.len() && is_word(self.text[self.at]) {
                    self.at += 1;
                }
                self.push(start, Kind::Function);
            } else if is(c, b'=') && in_set {
                self.at += 1;
                in_set = false;
                text = true;
            } else if [b'&', b'|', b'('].iter().any(|&b| is(c, b)) {
                self.at += 1;
                // Parentesen efter "in" er en liste, ikke kommandoer.
                command = !(is(c, b'(') && after_in);
                after_in = false;
            } else if [b')', b'<', b'>', b'=', b',', b';'].iter().any(|&b| is(c, b)) {
                self.at += 1;
                command = false;
            } else {
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    if is_space(c) || b"&|()<>=\"%!,;".iter().any(|&b| is(c, b)) {
                        break;
                    }
                    self.at += 1;
                }
                let word: String = self.text[start..self.at].iter().take(16).map(|&c| if c < 0x80 { (c as u8).to_ascii_lowercase() as char } else { '?' }).collect();
                let word = if self.at - start <= 16 { word.trim_end_matches('.') } else { "" };
                if word == "rem" && command {
                    self.skip_line();
                    self.push(start, Kind::Comment);
                } else if batch_keyword(word) {
                    self.push(start, Kind::Keyword);
                    command = matches!(word, "do" | "else");
                    in_set = word == "set";
                    after_in = word == "in";
                } else if command || batch_command(word) {
                    self.push(start, Kind::Function);
                    command = false;
                    text = matches!(word, "echo" | "title");
                } else if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                    self.push(start, Kind::Number);
                }
            }
        }
    }

    /// `%NAVN%`, `%1`, `%~dp0`, `%%i`, `%%~nxi` og `!NAVN!`.
    fn batch_variable(&mut self) -> bool {
        let start = self.at;
        let end = self.line_end(start);
        let letter = |c: u16| c < 0x80 && (c as u8).is_ascii_alphabetic();
        let mut at = start + 1;
        if is(self.text[start], b'!') {
            while at < end && (is_word(self.text[at]) || is(self.text[at], b':') || is(self.text[at], b'~')) {
                at += 1;
            }
            if at == start + 1 || at >= end || !is(self.text[at], b'!') {
                return false;
            }
            at += 1;
        } else if is(self.peek(1), b'%') || is(self.peek(1), b'~') || is_digit(self.peek(1)) || is(self.peek(1), b'*') {
            // Løkkevariabler og argumenter: %%i, %%~nxi, %1, %~dp0, %*
            at += is(self.peek(1), b'%') as usize;
            if at < end && is(self.text[at], b'~') {
                at += 1;
                while at + 1 < end && letter(self.text[at]) && (letter(self.text[at + 1]) || is_digit(self.text[at + 1])) {
                    at += 1;
                }
            }
            if at >= end || is_space(self.text[at]) {
                return false;
            }
            at += 1;
        } else {
            while at < end && !is(self.text[at], b'%') && !is_space(self.text[at]) && !is(self.text[at], b'"') {
                at += 1;
            }
            if at == start + 1 || at >= end || !is(self.text[at], b'%') {
                return false;
            }
            at += 1;
        }
        self.at = at;
        self.push(start, Kind::Type);
        true
    }

    /// En streng i anførselstegn. Variablerne i den får deres egen farve.
    fn batch_string(&mut self) {
        let mut piece = self.at;
        self.at += 1;
        while self.at < self.text.len() && self.text[self.at] != CR {
            let c = self.text[self.at];
            if is(c, b'"') {
                self.at += 1;
                break;
            } else if is(c, b'%') || is(c, b'!') {
                let mark = self.at;
                self.push(piece, Kind::Str);
                if self.batch_variable() {
                    piece = self.at;
                } else {
                    self.at += 1;
                    piece = mark;
                }
            } else {
                self.at += 1;
            }
        }
        self.push(piece, Kind::Str);
    }

    // ---------- PowerShell ----------

    fn powershell(&mut self) {
        // Om det næste ord står på en kommandos plads (først på linjen, efter `|`, `=` ...).
        let mut command = true;
        // Navnet efter "function" er en funktion, efter "class" og "enum" en type.
        let mut names: Option<Kind> = None;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            let previous = if start > 0 { self.text[start - 1] } else { 32 };
            if c == CR {
                self.at += 1;
                command = true;
            } else if is_space(c) {
                self.at += 1;
            } else if self.starts_with("<#") {
                self.at = self.find_before(start, self.text.len(), "#>").map_or(self.text.len(), |close| close + 2);
                self.push(start, Kind::Comment);
            } else if is(c, b'#') && (is_space(previous) || [b';', b'(', b'{', b'|'].iter().any(|&b| is(previous, b))) {
                self.line_comment();
            } else if (self.starts_with("@'") || self.starts_with("@\"")) && (self.peek(2) == CR || self.peek(2) == 0) {
                // Her-streng: alt frem til '@ eller "@ først på en linje.
                let close = if is(self.peek(1), b'"') { "\r\"@" } else { "\r'@" };
                self.at = self.find_before(start, self.text.len(), close).map_or(self.text.len(), |found| found + 3);
                self.push(start, Kind::Str);
                command = false;
            } else if is(c, b'\'') {
                // Kun '' har særlig betydning mellem enkelte anførselstegn.
                self.at += 1;
                while self.at < self.text.len() {
                    let quote = is(self.text[self.at], b'\'');
                    self.at += 1;
                    if quote {
                        if is(self.peek(0), b'\'') {
                            self.at += 1;
                        } else {
                            break;
                        }
                    }
                }
                self.push(start, Kind::Str);
                command = false;
            } else if is(c, b'"') {
                self.powershell_double_quoted();
                command = false;
            } else if is(c, b'$') {
                if is(self.peek(1), b'(') {
                    // $( ... ): parentesen starter en ny kommando.
                    self.at += 1;
                } else {
                    if !self.powershell_variable() {
                        self.at += 1;
                    }
                    command = false;
                }
            } else if is(c, b'[') {
                // [string], [System.IO.Path] og [Parameter(...)], men ikke $liste[0].
                self.at += 1;
                let indexing = is_word(previous) || [b')', b']', b'}', b'"', b'\''].iter().any(|&b| is(previous, b));
                if !indexing && is_word_start(self.peek(0)) && !is(self.peek(0), b'$') {
                    let name = self.at;
                    while self.at < self.text.len() && (is_word(self.text[self.at]) || is(self.text[self.at], b'.')) {
                        self.at += 1;
                    }
                    if [b']', b'(', b'['].iter().any(|&b| is(self.peek(0), b)) {
                        self.push(name, Kind::Type);
                    }
                    // I [Parameter(Mandatory)] er det i parentesen ikke kommandoer.
                    if is(self.peek(0), b'(') {
                        self.at += 1;
                        command = false;
                    }
                }
            } else if is(c, b'-') && self.peek(1) < 0x80 && (self.peek(1) as u8).is_ascii_alphabetic() && !is_word(previous) {
                // -eq, -and, -match er operatorer; -Path og andre parametre får ingen farve.
                self.at += 1;
                let mut buffer = [0u8; 24];
                let (_, len) = self.word(&mut buffer);
                buffer[..len].make_ascii_lowercase();
                if powershell_operator(std::str::from_utf8(&buffer[..len]).unwrap_or("")) {
                    self.push(start, Kind::Keyword);
                }
                command = false;
            } else if is_digit(c) {
                self.number();
                command = false;
            } else if [b'|', b';', b'{', b'(', b'=', b'&'].iter().any(|&b| is(c, b)) {
                self.at += 1;
                command = true;
            } else if is_word_start(c) {
                // Et ord må indeholde bindestreger: Get-ChildItem.
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    let hyphen = is(c, b'-') && self.peek(1) < 0x80 && (self.peek(1) as u8).is_ascii_alphabetic();
                    if !(is_word(c) && !is(c, b'$')) && !hyphen {
                        break;
                    }
                    self.at += 1;
                }
                let word: String = self.text[start..self.at].iter().take(24).map(|&c| if c < 0x80 { (c as u8).to_ascii_lowercase() as char } else { '?' }).collect();
                let word = if self.at - start <= 24 { word.as_str() } else { "" };
                let member = is(previous, b'.') || (start > 1 && is(previous, b':') && is(self.text[start - 2], b':'));
                let called = is(self.peek(0), b'(');
                if let Some(kind) = names.take() {
                    self.push(start, kind);
                } else if member {
                    if called {
                        self.push(start, Kind::Function);
                    }
                } else if powershell_keyword(word) {
                    names = match word {
                        "function" | "filter" => Some(Kind::Function),
                        "class" | "enum" => Some(Kind::Type),
                        _ => None,
                    };
                    self.push(start, Kind::Keyword);
                } else if command || called || word.contains('-') {
                    self.push(start, Kind::Function);
                }
                command = false;
            } else {
                self.at += 1;
            }
        }
    }

    /// `$navn`, `$env:PATH`, `${navn}`, `$_` og `$?`. `$true`, `$false` og `$null` er værdier.
    fn powershell_variable(&mut self) -> bool {
        let start = self.at;
        let next = self.peek(1);
        if is(next, b'{') {
            let end = self.line_end(start);
            match self.find_before(start, end, "}") {
                Some(close) => self.at = close + 1,
                None => return false,
            }
        } else if is(next, b'?') || is(next, b'$') || is(next, b'^') {
            self.at += 2;
        } else if is_word_start(next) {
            self.at += 1;
            while self.at < self.text.len() {
                let c = self.text[self.at];
                // Et kolon hører med, når der følger et navn: $env:PATH, $script:antal.
                let scope = is(c, b':') && is_word_start(self.peek(1)) && !is(self.peek(1), b'$');
                if !(is_word(c) && !is(c, b'$')) && !scope {
                    break;
                }
                self.at += 1;
            }
        } else {
            return false;
        }
        let name: String = self.text[start + 1..self.at].iter().take(6).map(|&c| (c as u8).to_ascii_lowercase() as char).collect();
        let literal = self.at - start <= 6 && matches!(name.as_str(), "true" | "false" | "null");
        self.push(start, if literal { Kind::Number } else { Kind::Type });
        true
    }

    /// En streng i dobbelte anførselstegn. Variablerne i den får deres egen farve.
    fn powershell_double_quoted(&mut self) {
        let mut piece = self.at;
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is(c, b'`') {
                self.at += 2;
            } else if is(c, b'"') {
                self.at += 1;
                if is(self.peek(0), b'"') {
                    self.at += 1;
                } else {
                    break;
                }
            } else if is(c, b'$') && is(self.peek(1), b'(') {
                let mut depth = 0u32;
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    self.at += 1;
                    if is(c, b'(') {
                        depth += 1;
                    } else if is(c, b')') {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            } else if is(c, b'$') {
                let dollar = self.at;
                self.push(piece, Kind::Str);
                if self.powershell_variable() {
                    piece = self.at;
                } else {
                    self.at += 1;
                    piece = dollar;
                }
            } else {
                self.at += 1;
            }
        }
        self.at = self.at.min(self.text.len());
        self.push(piece, Kind::Str);
    }

    // ---------- Lua ----------

    fn lua(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("--") {
                self.at += 2;
                // --[[ ... ]] er en kommentar over flere linjer.
                if !self.lua_long(start, Kind::Comment) {
                    self.skip_line();
                    self.push(start, Kind::Comment);
                }
            } else if is(c, b'[') && self.lua_long(start, Kind::Str) {
                // [[ ... ]]: en streng over flere linjer.
            } else if is(c, b'"') || is(c, b'\'') {
                self.quoted(c, true);
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is_word_start(c) {
                let member = start > 0 && (is(self.text[start - 1], b'.') || is(self.text[start - 1], b':'));
                let mut buffer = [0u8; 24];
                let (start, len) = self.word(&mut buffer);
                let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
                if !member && lua_keyword(word) {
                    self.push(start, Kind::Keyword);
                } else if !member && matches!(word, "true" | "false" | "nil") {
                    self.push(start, Kind::Number);
                } else {
                    let called = is(self.next_visible(self.at), b'(');
                    self.name(start, called);
                }
            } else {
                self.at += 1;
            }
        }
    }

    /// `[[ ... ]]` eller `[==[ ... ]==]` fra `self.at`. Returnerer false, hvis der ikke
    /// står sådan en her.
    fn lua_long(&mut self, start: usize, kind: Kind) -> bool {
        if !is(self.peek(0), b'[') {
            return false;
        }
        let mut level = 0;
        while is(self.peek(1 + level), b'=') {
            level += 1;
        }
        if !is(self.peek(1 + level), b'[') {
            return false;
        }
        let closing = format!("]{}]", "=".repeat(level));
        self.at = self.find_before(self.at, self.text.len(), &closing).map_or(self.text.len(), |close| close + closing.len());
        self.push(start, kind);
        true
    }

    // ---------- TOML ----------

    fn toml(&mut self) {
        // Åbne [ og { fra værdier som `tags = ["a", "b"]` og `x = { a = 1 }`.
        let mut open: Vec<u16> = Vec::new();
        // Om det næste ord er en nøgle (før `=`) og ikke en værdi.
        let mut key = true;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if c == CR {
                self.at += 1;
                key = key || open.is_empty();
            } else if is_space(c) {
                self.at += 1;
            } else if is(c, b'#') {
                self.line_comment();
            } else if is(c, b'[') && key && open.is_empty() {
                // [tabel] og [[liste]]
                while self.at < self.text.len() && self.text[self.at] != CR && !is(self.text[self.at], b'#') {
                    self.at += 1;
                }
                while self.at > start && self.text[self.at - 1] == 32 {
                    self.at -= 1;
                }
                self.push(start, Kind::Function);
                self.skip_line();
            } else if is(c, b'"') || is(c, b'\'') {
                if self.peek(1) == c && self.peek(2) == c {
                    // Tre anførselstegn: tekst over flere linjer.
                    self.at += 3;
                    while self.at < self.text.len() && !(self.text[self.at] == c && self.peek(1) == c && self.peek(2) == c) {
                        self.at += if is(self.text[self.at], b'\\') && is(c, b'"') { 2 } else { 1 };
                    }
                    self.at = (self.at + 3).min(self.text.len());
                    self.push(start, Kind::Str);
                } else {
                    self.quoted(c, is(c, b'"'));
                }
                if key {
                    if let Some(token) = self.out.last_mut() {
                        token.kind = Kind::Keyword;
                    }
                }
            } else if is(c, b'=') {
                self.at += 1;
                key = false;
            } else if is(c, b'[') || is(c, b'{') {
                open.push(c);
                key = is(c, b'{');
                self.at += 1;
            } else if is(c, b']') || is(c, b'}') {
                open.pop();
                key = false;
                self.at += 1;
            } else if is(c, b',') {
                key = open.last().is_some_and(|&o| is(o, b'{'));
                self.at += 1;
            } else {
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    if is_space(c) || [b'=', b',', b']', b'}', b'#'].iter().any(|&b| is(c, b)) {
                        break;
                    }
                    self.at += 1;
                }
                if self.at == start {
                    self.at += 1;
                } else if key {
                    self.push(start, Kind::Keyword);
                } else {
                    // Tal, datoer, true/false, inf og nan.
                    let value = &self.text[start..self.at];
                    let sign = is(value[0], b'+') || is(value[0], b'-');
                    let first = value.get(sign as usize).copied().unwrap_or(0);
                    let word = value.iter().map(|&c| c as u8).skip(sign as usize);
                    if is_digit(first) || word.clone().eq(*b"true") || word.clone().eq(*b"false") || word.clone().eq(*b"inf") || word.eq(*b"nan") {
                        self.push(start, Kind::Number);
                    }
                }
            }
        }
    }

    // ---------- Kotlin og Swift ----------

    /// Sprog med krøllede parenteser, der ligner hinanden nok til at dele regler.
    fn curly(&mut self, spec: &Curly) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                // Blokkommentarer kan ligge inden i hinanden.
                let mut depth = 0u32;
                while self.at < self.text.len() {
                    if self.starts_with("/*") {
                        depth += 1;
                        self.at += 2;
                    } else if self.starts_with("*/") {
                        depth -= 1;
                        self.at += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        self.at += 1;
                    }
                }
                self.push(start, Kind::Comment);
            } else if self.starts_with("\"\"\"") {
                // Tekst over flere linjer.
                self.at += 3;
                while self.at < self.text.len() && !self.starts_with("\"\"\"") {
                    self.at += 1;
                }
                self.at = (self.at + 3).min(self.text.len());
                self.push(start, Kind::Str);
            } else if is(c, b'"') || (spec.char_literals && is(c, b'\'')) {
                self.quoted(c, true);
            } else if (is(c, b'@') || (spec.hash_words && is(c, b'#'))) && is_word_start(self.peek(1)) {
                // @Composable, @objc og #if, #selector
                self.at += 1;
                while self.at < self.text.len() && is_word(self.text[self.at]) {
                    self.at += 1;
                }
                self.push(start, if is(c, b'@') { Kind::Function } else { Kind::Keyword });
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is_word_start(c) {
                let member = start > 0 && is(self.text[start - 1], b'.');
                let mut buffer = [0u8; 24];
                let (start, len) = self.word(&mut buffer);
                let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
                let called = is(self.next_visible(self.at), b'(');
                if !member && ((spec.keyword)(word) || (!called && (spec.soft)(word))) {
                    self.push(start, Kind::Keyword);
                } else if !member && (spec.literal)(word) {
                    self.push(start, Kind::Number);
                } else {
                    self.name(start, called);
                }
            } else {
                self.at += 1;
            }
        }
    }

    // ---------- Ruby ----------

    fn ruby(&mut self) {
        let mut before = Before::Operator;
        let mut heredoc: Option<(usize, usize)> = None;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            let previous = if start > 0 { self.text[start - 1] } else { CR };
            if c == CR {
                self.at += 1;
                before = Before::Operator;
                if let Some((from, to)) = heredoc.take() {
                    self.bash_heredoc(from, to);
                }
            } else if is_space(c) {
                self.at += 1;
            } else if is(c, b'#') {
                self.line_comment();
            } else if previous == CR && self.starts_with("=begin") {
                // =begin ... =end: en kommentar over flere linjer.
                let close = self.find_before(start, self.text.len(), "\r=end").map_or(self.text.len(), |found| found + 1);
                self.at = self.line_end(close);
                self.push(start, Kind::Comment);
            } else if is(c, b'"') || is(c, b'`') {
                self.ruby_string(c);
                before = Before::Value;
            } else if is(c, b'\'') {
                self.string_until(start, c);
                before = Before::Value;
            } else if is(c, b'%') && before == Before::Operator && self.ruby_percent() {
                before = Before::Value;
            } else if self.starts_with("<<") && (matches!(self.peek(2) as u8, b'~' | b'-') || is_upper(self.peek(2))) && self.peek(2) < 0x80 {
                // <<~SLUT: teksten på de næste linjer frem til mærket er en streng.
                self.at += 2;
                while [b'~', b'-', b'"', b'\''].iter().any(|&b| is(self.peek(0), b)) {
                    self.at += 1;
                }
                let label = self.at;
                while self.at < self.text.len() && is_word(self.text[self.at]) {
                    self.at += 1;
                }
                if self.at > label {
                    heredoc = Some((label, self.at));
                }
                before = Before::Value;
            } else if is(c, b':') {
                if is(self.peek(1), b':') {
                    self.at += 2;
                } else if is_word_start(self.peek(1)) && !is_word(previous) {
                    // Et symbol: :navn
                    self.at += 1;
                    self.ruby_name();
                    self.push(start, Kind::Number);
                    before = Before::Value;
                } else {
                    self.at += 1;
                    before = Before::Operator;
                }
            } else if (is(c, b'@') || is(c, b'$')) && (is_word_start(self.peek(1)) || is(self.peek(1), b'@')) {
                // @instans, @@klasse og $global
                self.at += 1 + is(self.peek(1), b'@') as usize;
                self.ruby_name();
                self.push(start, Kind::Type);
                before = Before::Value;
            } else if is_digit(c) {
                self.number();
                before = Before::Value;
            } else if is_word_start(c) && !is(c, b'$') {
                before = self.ruby_word();
            } else if is(c, b'/') && before == Before::Operator && self.regex() {
                before = Before::Value;
            } else {
                before = if [b')', b']', b'}'].iter().any(|&b| is(c, b)) { Before::Value } else { Before::Operator };
                self.at += 1;
            }
        }
    }

    /// Et navn, med `?` eller `!` til sidst hvis det er en del af det (`empty?`, `save!`).
    fn ruby_name(&mut self) {
        while self.at < self.text.len() && is_word(self.text[self.at]) && !is(self.text[self.at], b'$') {
            self.at += 1;
        }
        if (is(self.peek(0), b'?') || is(self.peek(0), b'!')) && !is(self.peek(1), b'=') {
            self.at += 1;
        }
    }

    /// En streng i dobbelte anførselstegn. Kode i `#{...}` må selv indeholde anførselstegn.
    fn ruby_string(&mut self, quote: u16) {
        let start = self.at;
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is(c, b'\\') {
                self.at += 2;
            } else if c == quote {
                self.at += 1;
                break;
            } else if self.starts_with("#{") {
                let mut depth = 0u32;
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    self.at += 1;
                    if is(c, b'{') {
                        depth += 1;
                    } else if is(c, b'}') {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            } else {
                self.at += 1;
            }
        }
        self.at = self.at.min(self.text.len());
        self.push(start, Kind::Str);
    }

    /// `%w[a b]`, `%i(a b)`, `%q{...}` og `%r{...}`.
    fn ruby_percent(&mut self) -> bool {
        let letter = matches!(self.peek(1) as u8, b'w' | b'W' | b'i' | b'I' | b'q' | b'Q' | b'r') && self.peek(1) < 0x80;
        let open = self.peek(1 + letter as usize);
        let close = match open as u8 {
            b'[' if open < 0x80 => b']',
            b'(' if open < 0x80 => b')',
            b'{' if open < 0x80 => b'}',
            _ => return false,
        };
        let start = self.at;
        self.at += 2 + letter as usize;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            self.at += if is(c, b'\\') { 2 } else { 1 };
            if is(c, close) {
                break;
            }
        }
        self.at = self.at.min(self.text.len());
        self.push(start, Kind::Str);
        true
    }

    fn ruby_word(&mut self) -> Before {
        let member = self.at > 0 && is(self.text[self.at - 1], b'.');
        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
        if (is(self.peek(0), b'?') || is(self.peek(0), b'!')) && !is(self.peek(1), b'=') {
            self.at += 1;
        }
        // "def navn" og "def self.navn"
        let on_self = member && start >= 5 && self.text[start - 5..start - 1].iter().map(|&c| c as u8).eq(*b"self");
        let declared = (self.preceded_by(start, "def") && word != "self") || (on_self && self.preceded_by(start - 5, "def"));
        if declared {
            self.push(start, Kind::Function);
        } else if !member && ruby_keyword(word) {
            self.push(start, Kind::Keyword);
            // Efter "end" og "self" er `/` division; efter "if" og "return" starter et udtryk.
            return if matches!(word, "end" | "self" | "super") { Before::Value } else { Before::Operator };
        } else if !member && matches!(word, "true" | "false" | "nil") {
            self.push(start, Kind::Number);
        } else {
            let called = is(self.peek(0), b'(');
            self.name(start, called);
        }
        Before::Value
    }

    // ---------- C og C++ ----------

    fn c_like(&mut self, cpp: bool) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if is(c, b'"') || is(c, b'\'') {
                self.quoted(c, true);
            } else if is(c, b'#') && self.first_on_line() {
                // #include, #define, #ifdef ...
                self.at += 1;
                while self.peek(0) == 32 || self.peek(0) == 9 {
                    self.at += 1;
                }
                let word = self.at;
                while self.at < self.text.len() && is_word(self.text[self.at]) {
                    self.at += 1;
                }
                let include = self.text[word..self.at].iter().map(|&c| c as u8).eq(*b"include");
                self.push(start, Kind::Keyword);
                while self.peek(0) == 32 || self.peek(0) == 9 {
                    self.at += 1;
                }
                if include && is(self.peek(0), b'<') {
                    // #include <stdio.h>: filnavnet er en streng.
                    let name = self.at;
                    while self.at < self.text.len() && self.text[self.at] != CR {
                        self.at += 1;
                        if is(self.text[self.at - 1], b'>') {
                            break;
                        }
                    }
                    self.push(name, Kind::Str);
                }
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
                // C++ tillader ' som tusindtalsskilletegn: 1'000'000.
                while cpp && is(self.peek(0), b'\'') && is_word(self.peek(1)) {
                    self.at += 1;
                    while self.at < self.text.len() && is_word(self.text[self.at]) {
                        self.at += 1;
                    }
                    if let Some(number) = self.out.last_mut() {
                        number.end = self.at;
                    }
                }
            } else if is_word_start(c) {
                self.c_word(cpp);
            } else {
                self.at += 1;
            }
        }
    }

    fn c_word(&mut self, cpp: bool) {
        let start = self.at;
        // Strenge med præfiks: L"...", u8"...", og i C++ den rå R"mærke(...)mærke".
        let prefix = if self.starts_with("u8") {
            2
        } else if [b'L', b'u', b'U'].iter().any(|&b| is(self.peek(0), b)) {
            1
        } else {
            0
        };
        if cpp && is(self.peek(prefix), b'R') && is(self.peek(prefix + 1), b'"') {
            self.at += prefix + 2;
            let mut closing: Vec<u16> = vec![b')' as u16];
            while self.at < self.text.len() && closing.len() <= 16 && !is(self.text[self.at], b'(') && self.text[self.at] != CR {
                closing.push(self.text[self.at]);
                self.at += 1;
            }
            closing.push(b'"' as u16);
            let from = self.at.min(self.text.len());
            self.at = self.text[from..]
                .windows(closing.len())
                .position(|w| w == closing.as_slice())
                .map_or(self.text.len(), |found| from + found + closing.len());
            self.push(start, Kind::Str);
            return;
        }
        if prefix > 0 && (is(self.peek(prefix), b'"') || is(self.peek(prefix), b'\'')) {
            self.at += prefix;
            self.quoted(self.text[self.at], true);
            if let Some(string) = self.out.last_mut() {
                string.start = start;
            }
            return;
        }

        let member = start > 0
            && (is(self.text[start - 1], b'.') || (start > 1 && is(self.text[start - 1], b'>') && is(self.text[start - 2], b'-')));
        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
        let called = is(self.next_visible(self.at), b'(');
        if !member {
            if c_keyword(word) || (cpp && cpp_keyword(word)) {
                self.push(start, Kind::Keyword);
                return;
            }
            if matches!(word, "NULL" | "true" | "false") || (cpp && word == "nullptr") {
                self.push(start, Kind::Number);
                return;
            }
            // Navne der ender på _t er typer efter fast skik: size_t, uint8_t.
            if c_type(word) || (cpp && cpp_type(word)) || (word.len() > 2 && word.ends_with("_t")) {
                self.push(start, Kind::Type);
                return;
            }
        }
        // Funktioner skrives ofte med stort (CreateWindow), så kaldet afgør farven.
        if called && !self.preceded_by(start, "new") {
            self.push(start, Kind::Function);
        } else if ["struct", "enum", "union", "class"].iter().any(|k| self.preceded_by(start, k)) {
            self.push(start, Kind::Type);
        } else if !member || called {
            self.name(start, false);
        }
    }

    // ---------- Bash ----------

    fn bash(&mut self) {
        // Om det næste ord står på en kommandos plads (først på linjen, efter `|`, `;` ...).
        let mut command = true;
        let mut names_function = false;
        let mut heredoc: Option<(usize, usize)> = None;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            let before = if start > 0 { self.text[start - 1] } else { 32 };
            if c == CR {
                self.at += 1;
                command = true;
                if let Some((from, to)) = heredoc.take() {
                    self.bash_heredoc(from, to);
                }
            } else if is_space(c) {
                self.at += 1;
            } else if is(c, b'#') && (is_space(before) || [b';', b'(', b'|', b'&'].iter().any(|&b| is(before, b))) {
                self.line_comment();
            } else if is(c, b'\\') {
                self.at = (self.at + 2).min(self.text.len());
            } else if is(c, b'\'') || is(c, b'`') {
                // Ingen tegn har særlig betydning mellem enkelte anførselstegn.
                self.at = self.find_before(start + 1, self.text.len(), if is(c, b'`') { "`" } else { "'" }).map_or(self.text.len(), |close| close + 1);
                self.push(start, Kind::Str);
                command = false;
            } else if is(c, b'"') {
                self.bash_double_quoted();
                command = false;
            } else if is(c, b'$') {
                if is(self.peek(1), b'\'') {
                    self.at += 1;
                    self.string_until(start, b'\'' as u16);
                    command = false;
                } else if is(self.peek(1), b'(') {
                    // $( ... ): parentesen starter en ny kommando.
                    self.at += 1;
                } else {
                    if !self.bash_variable() {
                        self.at += 1;
                    }
                    command = false;
                }
            } else if self.starts_with("<<") && !self.starts_with("<<<") {
                // <<SLUT: teksten på de næste linjer frem til mærket er en streng.
                self.at += 2;
                while [b'-', b' ', b'"', b'\'', b'\\'].iter().any(|&b| is(self.peek(0), b)) {
                    self.at += 1;
                }
                let label = self.at;
                while self.at < self.text.len() && is_word(self.text[self.at]) {
                    self.at += 1;
                }
                if self.at > label {
                    heredoc = Some((label, self.at));
                }
            } else if [b';', b'|', b'&', b'(', b')'].iter().any(|&b| is(c, b)) {
                self.at += 1;
                command = true;
            } else if [b'<', b'>', b'='].iter().any(|&b| is(c, b)) {
                self.at += 1;
            } else {
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    if is_space(c) || b";|&()<>\"'`$=".iter().any(|&b| is(c, b)) {
                        break;
                    }
                    self.at += 1;
                }
                let word: String = self.text[start..self.at].iter().take(16).map(|&c| if c < 0x80 { c as u8 as char } else { '?' }).collect();
                let word = if self.at - start <= 16 { word.as_str() } else { "" };
                let assigns = command && (is(self.peek(0), b'=') || (word.ends_with('+') && is(self.peek(0), b'=')));
                if names_function {
                    names_function = false;
                    command = false;
                    self.push(start, Kind::Function);
                } else if assigns {
                    // NAVN=værdi: det der følger er værdien, ikke en kommando.
                    command = false;
                } else if command {
                    match word {
                        "if" | "then" | "else" | "elif" | "do" | "while" | "until" | "time" => self.push(start, Kind::Keyword),
                        "fi" | "done" | "esac" | "for" | "case" | "select" | "function" | "in" => {
                            names_function = word == "function";
                            command = false;
                            self.push(start, Kind::Keyword);
                        }
                        "return" | "break" | "continue" | "exit" | "export" | "local" | "readonly" | "declare"
                        | "typeset" | "unset" | "shift" | "source" | "alias" | "eval" | "exec" | "set" | "trap" => {
                            command = false;
                            self.push(start, Kind::Keyword);
                        }
                        "{" | "}" | "!" => {}
                        "[" | "[[" | "." | ":" => command = false,
                        _ => {
                            command = false;
                            self.push(start, Kind::Function);
                        }
                    }
                } else if word == "in" {
                    self.push(start, Kind::Keyword);
                } else if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                    self.push(start, Kind::Number);
                }
            }
        }
    }

    /// `$navn`, `${navn:-x}`, `$1`, `$@`, `$?`. Returnerer false, hvis `$` står alene.
    fn bash_variable(&mut self) -> bool {
        let start = self.at;
        let next = self.peek(1);
        if is(next, b'{') {
            let end = self.line_end(start);
            match self.find_before(start, end, "}") {
                Some(close) => self.at = close + 1,
                None => return false,
            }
        } else if is_digit(next) || [b'@', b'*', b'#', b'?', b'-', b'$', b'!'].iter().any(|&b| is(next, b)) {
            self.at += 2;
        } else if is_word_start(next) {
            self.at += 1;
            while self.at < self.text.len() && is_word(self.text[self.at]) && !is(self.text[self.at], b'$') {
                self.at += 1;
            }
        } else {
            return false;
        }
        self.push(start, Kind::Type);
        true
    }

    /// En streng i dobbelte anførselstegn. Variablerne i den får deres egen farve.
    fn bash_double_quoted(&mut self) {
        let mut piece = self.at;
        self.at += 1;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is(c, b'\\') {
                self.at += 2;
            } else if is(c, b'"') {
                self.at += 1;
                break;
            } else if is(c, b'$') && is(self.peek(1), b'(') {
                // "$(kommando "med" anførselstegn)": spring til den parentes der lukker.
                let mut depth = 0u32;
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    self.at += 1;
                    if is(c, b'(') {
                        depth += 1;
                    } else if is(c, b')') {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
            } else if is(c, b'$') {
                let dollar = self.at;
                self.push(piece, Kind::Str);
                if !self.bash_variable() {
                    self.at += 1;
                    piece = dollar;
                } else {
                    piece = self.at;
                }
            } else {
                self.at += 1;
            }
        }
        self.at = self.at.min(self.text.len());
        self.push(piece, Kind::Str);
    }

    /// Teksten efter `<<MÆRKE`, fra linjen efter og til og med linjen med mærket.
    fn bash_heredoc(&mut self, from: usize, to: usize) {
        let start = self.at;
        let mut line = start;
        let mut end = self.text.len();
        while line < self.text.len() {
            let line_end = self.line_end(line);
            let mut at = line;
            while at < line_end && (self.text[at] == 32 || self.text[at] == 9) {
                at += 1;
            }
            if self.text[at..line_end] == self.text[from..to] {
                end = line_end;
                break;
            }
            line = line_end + 1;
        }
        self.at = end;
        self.push(start, Kind::Str);
    }

    // ---------- Go ----------

    fn go(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if is(c, b'"') || is(c, b'\'') {
                self.quoted(c, true);
            } else if is(c, b'`') {
                // Rå streng: alt frem til næste `, også over flere linjer.
                self.at = self.find_before(start + 1, self.text.len(), "`").map_or(self.text.len(), |close| close + 1);
                self.push(start, Kind::Str);
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is_word_start(c) {
                let member = start > 0 && is(self.text[start - 1], b'.');
                let mut buffer = [0u8; 24];
                let (start, len) = self.word(&mut buffer);
                let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
                if !member && go_keyword(word) {
                    self.push(start, Kind::Keyword);
                } else if !member && matches!(word, "true" | "false" | "nil" | "iota") {
                    self.push(start, Kind::Number);
                } else if !member && go_type(word) {
                    self.push(start, Kind::Type);
                } else if is(self.next_visible(self.at), b'(') || self.preceded_by(start, "func") {
                    // Eksporterede funktioner skrives med stort, så kaldet afgør farven.
                    self.push(start, Kind::Function);
                } else if self.preceded_by(start, "type") {
                    self.push(start, Kind::Type);
                } else if !member {
                    self.name(start, false);
                }
            } else {
                self.at += 1;
            }
        }
    }

    // ---------- Java ----------

    fn java(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if self.starts_with("\"\"\"") {
                // Tekstblok over flere linjer.
                self.at += 3;
                while self.at < self.text.len() && !self.starts_with("\"\"\"") {
                    self.at += if is(self.text[self.at], b'\\') { 2 } else { 1 };
                }
                self.at = (self.at + 3).min(self.text.len());
                self.push(start, Kind::Str);
            } else if is(c, b'"') || is(c, b'\'') {
                self.quoted(c, true);
            } else if is(c, b'@') && is_word_start(self.peek(1)) {
                // En annotation som @Override; "@interface" erklærer en.
                self.at += 1;
                while self.at < self.text.len() && (is_word(self.text[self.at]) || is(self.text[self.at], b'.')) {
                    self.at += 1;
                }
                let declares = self.at - start == 10 && self.text[start + 1..self.at].iter().zip(b"interface").all(|(&c, &b)| is(c, b));
                self.push(start, if declares { Kind::Keyword } else { Kind::Function });
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is_word_start(c) {
                let member = start > 0 && is(self.text[start - 1], b'.');
                let mut buffer = [0u8; 24];
                let (start, len) = self.word(&mut buffer);
                let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
                let called = is(self.next_visible(self.at), b'(');
                if !member && (java_keyword(word) || (!called && matches!(word, "var" | "record" | "sealed" | "permits" | "yield"))) {
                    self.push(start, Kind::Keyword);
                } else if !member && matches!(word, "true" | "false" | "null") {
                    self.push(start, Kind::Number);
                } else if !member && java_type(word) {
                    self.push(start, Kind::Type);
                } else {
                    self.name(start, called);
                }
            } else {
                self.at += 1;
            }
        }
    }

    // ---------- C# ----------

    fn csharp(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if is(c, b'"') {
                self.csharp_string(start, false);
            } else if is(c, b'$') || is(c, b'@') {
                // $"...", @"..." og $@"..." er strenge; @navn er bare et navn.
                let mut marks = 0;
                while is(self.peek(marks), b'$') || is(self.peek(marks), b'@') {
                    marks += 1;
                }
                let verbatim = (0..marks).any(|i| is(self.peek(i), b'@'));
                self.at += marks;
                if is(self.peek(0), b'"') {
                    self.csharp_string(start, verbatim);
                }
            } else if is(c, b'\'') {
                self.quoted(c, true);
            } else if is(c, b'#') && self.first_on_line() {
                // #if, #region ...
                self.at += 1;
                while self.at < self.text.len() && is_word(self.text[self.at]) {
                    self.at += 1;
                }
                self.push(start, Kind::Keyword);
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is_word_start(c) {
                self.csharp_word();
            } else {
                self.at += 1;
            }
        }
    }

    /// Strengen der starter med anførselstegnet ved `self.at`. `"""` og `@"` går over
    /// flere linjer; i `@"` er `""` et anførselstegn og `\` et almindeligt tegn.
    fn csharp_string(&mut self, start: usize, verbatim: bool) {
        let quote = b'"' as u16;
        let quotes = self.text[self.at..].iter().take_while(|&&c| c == quote).count();
        if quotes >= 3 {
            self.at += quotes;
            while self.at < self.text.len() {
                if self.text[self.at] == quote && self.text[self.at..].iter().take(quotes).filter(|&&c| c == quote).count() == quotes {
                    self.at += quotes;
                    break;
                }
                self.at += 1;
            }
            self.push(start, Kind::Str);
        } else if verbatim {
            self.at += 1;
            while self.at < self.text.len() {
                let c = self.text[self.at];
                self.at += 1;
                if c == quote {
                    if self.peek(0) == quote {
                        self.at += 1;
                    } else {
                        break;
                    }
                }
            }
            self.push(start, Kind::Str);
        } else {
            self.quoted(quote, true);
            // `quoted` begynder ved anførselstegnet; præfikset ($) hører med.
            if let Some(string) = self.out.last_mut() {
                string.start = start;
            }
        }
    }

    fn csharp_word(&mut self) {
        let member = self.at > 0 && is(self.text[self.at - 1], b'.');
        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        let word = std::str::from_utf8(&buffer[..len]).unwrap_or("");
        let called = is(self.next_visible(self.at), b'(');
        if !member {
            if csharp_keyword(word) || (!called && csharp_contextual(word)) {
                self.push(start, Kind::Keyword);
                return;
            }
            if matches!(word, "true" | "false" | "null") {
                self.push(start, Kind::Number);
                return;
            }
            if csharp_type(word) {
                self.push(start, Kind::Type);
                return;
            }
        }
        // Metoder skrives med stort i C#, så et kald er en funktion, medmindre det er
        // "new Order(...)". Egenskaber efter et punktum får ingen farve.
        if called && !self.preceded_by(start, "new") {
            self.push(start, Kind::Function);
        } else if !member || called {
            self.name(start, false);
        }
    }

    // ---------- PHP ----------

    /// En PHP-fil er HTML med kode mellem `<?php` og `?>`.
    fn php(&mut self) {
        while self.at < self.text.len() {
            let open = self.find_before(self.at, self.text.len(), "<?").unwrap_or(self.text.len());
            if open > self.at {
                self.embedded(open, |inner| inner.html());
            }
            if self.at >= self.text.len() {
                break;
            }
            let start = self.at;
            self.at += if self.starts_with("<?php") { 5 } else if self.starts_with("<?=") { 3 } else { 2 };
            self.push(start, Kind::Keyword);
            self.php_code();
        }
    }

    /// PHP-kode frem til `?>` eller tekstens ende.
    fn php_code(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            let start = self.at;
            if self.starts_with("?>") {
                self.at += 2;
                self.push(start, Kind::Keyword);
                return;
            } else if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") || (is(c, b'#') && !is(self.peek(1), b'[')) {
                // En linjekommentar slutter også ved ?>.
                while self.at < self.text.len() && self.text[self.at] != CR && !self.starts_with("?>") {
                    self.at += 1;
                }
                self.push(start, Kind::Comment);
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if is(c, b'"') || is(c, b'\'') {
                self.string_until(start, c);
            } else if self.starts_with("<<<") {
                self.php_heredoc();
            } else if is_digit(c) || (is(c, b'.') && is_digit(self.peek(1))) {
                self.number();
            } else if is(c, b'$') {
                // Variabler får ingen farve.
                self.at += 1;
                while self.at < self.text.len() && is_word(self.text[self.at]) && !is(self.text[self.at], b'$') {
                    self.at += 1;
                }
            } else if is_word_start(c) {
                self.php_word();
            } else {
                self.at += 1;
            }
        }
    }

    /// `<<<SLUT ... SLUT`: en streng over flere linjer, der ender med sit eget mærke.
    fn php_heredoc(&mut self) {
        let start = self.at;
        self.at += 3;
        while [b' ', b'"', b'\''].iter().any(|&b| is(self.peek(0), b)) {
            self.at += 1;
        }
        let label_start = self.at;
        while self.at < self.text.len() && is_word(self.text[self.at]) {
            self.at += 1;
        }
        let label = &self.text[label_start..self.at];
        if label.is_empty() {
            return;
        }
        let mut line = (self.line_end(self.at) + 1).min(self.text.len());
        let mut end = self.text.len();
        while line < self.text.len() {
            let line_end = self.line_end(line);
            let mut at = line;
            while at < line_end && (self.text[at] == 32 || self.text[at] == 9) {
                at += 1;
            }
            let after = at + label.len();
            if self.text[at..line_end].starts_with(label) && !self.text.get(after).is_some_and(|&c| is_word(c)) {
                end = after;
                break;
            }
            line = line_end + 1;
        }
        self.at = end;
        self.push(start, Kind::Str);
    }

    fn php_word(&mut self) {
        let member = self.at > 1
            && ((is(self.text[self.at - 1], b'>') && is(self.text[self.at - 2], b'-'))
                || (is(self.text[self.at - 1], b':') && is(self.text[self.at - 2], b':')));
        let mut buffer = [0u8; 24];
        let (start, len) = self.word(&mut buffer);
        let called = is(self.next_visible(self.at), b'(');
        // Nøgleord i PHP er ens med store og små bogstaver.
        let mut lower = buffer;
        lower[..len].make_ascii_lowercase();
        let word = std::str::from_utf8(&lower[..len]).unwrap_or("");
        if !member {
            if php_keyword(word) {
                self.push(start, Kind::Keyword);
                return;
            }
            if matches!(word, "true" | "false" | "null") {
                self.push(start, Kind::Number);
                return;
            }
            if !called && php_type(word) {
                self.push(start, Kind::Type);
                return;
            }
        }
        if called && !self.preceded_by(start, "new") {
            self.push(start, Kind::Function);
        } else if !member || called {
            self.name(start, false);
        }
    }

    // ---------- YAML ----------

    fn yaml(&mut self) {
        // Antal åbne [ og { fra linjer som `tags: [a, b]`.
        let mut flow = 0u32;
        while self.at < self.text.len() {
            let line = self.at;
            let end = self.line_end(line);
            let mut at = line;
            while at < end && self.text[at] == 32 {
                at += 1;
            }
            let indent = at - line;
            self.at = at;
            if indent == 0 && (self.starts_with("---") || self.starts_with("...")) && (at + 3 == end || self.text[at + 3] == 32) {
                self.at += 3;
                self.push(at, Kind::Comment);
            }

            let mut block = false;
            while self.at < end {
                let c = self.text[self.at];
                let start = self.at;
                // Linjens ende tæller som et mellemrum.
                let next = if start + 1 < end { self.text[start + 1] } else { 32 };
                let spaced = next == 32 || next == 9;
                if c == 32 || c == 9 {
                    self.at += 1;
                } else if is(c, b'#') && (start == line || is_space(self.text[start - 1])) {
                    self.at = end;
                    self.push(start, Kind::Comment);
                } else if is(c, b'-') && flow == 0 && spaced {
                    self.at += 1;
                    self.push(start, Kind::Keyword);
                } else if is(c, b'[') || is(c, b'{') {
                    flow += 1;
                    self.at += 1;
                } else if is(c, b']') || is(c, b'}') {
                    flow = flow.saturating_sub(1);
                    self.at += 1;
                } else if (is(c, b',') && flow > 0) || ((is(c, b':') || is(c, b'?')) && spaced) {
                    self.at += 1;
                } else if is(c, b'"') || is(c, b'\'') {
                    self.quoted(c, is(c, b'"'));
                    let mut after = self.at;
                    while after < end && self.text[after] == 32 {
                        after += 1;
                    }
                    let key = after < end && is(self.text[after], b':') && (after + 1 == end || self.text[after + 1] == 32);
                    if key {
                        if let Some(token) = self.out.last_mut() {
                            token.kind = Kind::Keyword;
                        }
                    }
                } else if ((is(c, b'&') || is(c, b'*')) && !spaced) || is(c, b'!') {
                    // Ankre (&navn), henvisninger (*navn) og typemærker (!!str).
                    self.at += 1;
                    while self.at < end && !is_space(self.text[self.at]) && ![b',', b']', b'}'].iter().any(|&b| is(self.text[self.at], b)) {
                        self.at += 1;
                    }
                    self.push(start, Kind::Type);
                } else if (is(c, b'|') || is(c, b'>')) && flow == 0 && self.yaml_block_header(end) {
                    block = true;
                } else {
                    self.yaml_plain(end, flow > 0);
                }
            }
            self.at = (end + 1).min(self.text.len());

            if block {
                // Teksten under `|` og `>`: alle linjer der er rykket længere ind.
                let start = self.at;
                let mut stop = start;
                let mut line = start;
                while line < self.text.len() {
                    let line_end = self.line_end(line);
                    let mut at = line;
                    while at < line_end && self.text[at] == 32 {
                        at += 1;
                    }
                    if at < line_end && at - line <= indent {
                        break;
                    }
                    if at < line_end {
                        stop = line_end;
                    }
                    line = line_end + 1;
                }
                if stop > start {
                    self.at = stop;
                    self.push(start, Kind::Str);
                    self.at = (stop + 1).min(self.text.len());
                }
            }
        }
    }

    /// `|`, `>-`, `|2+` sidst på linjen: starten på en tekstblok.
    fn yaml_block_header(&mut self, end: usize) -> bool {
        let mut at = self.at + 1;
        while at < end && (is_digit(self.text[at]) || is(self.text[at], b'+') || is(self.text[at], b'-')) {
            at += 1;
        }
        let rest = &self.text[at..end];
        let blank = rest.iter().position(|&c| c != 32).is_none_or(|i| is(rest[i], b'#') && i > 0);
        if blank {
            self.at = at;
        }
        blank
    }

    /// En værdi eller nøgle uden anførselstegn. Nøgler ender ved `: `.
    fn yaml_plain(&mut self, end: usize, in_flow: bool) {
        let start = self.at;
        let mut at = start;
        let mut key = false;
        while at < end {
            let c = self.text[at];
            let spaced = at + 1 >= end || self.text[at + 1] == 32 || self.text[at + 1] == 9;
            if is(c, b':') && spaced {
                key = true;
                break;
            }
            if (is(c, b'#') && at > start && self.text[at - 1] == 32) || (in_flow && [b',', b']', b'}'].iter().any(|&b| is(c, b))) {
                break;
            }
            at += 1;
        }
        let mut stop = at;
        while stop > start && self.text[stop - 1] == 32 {
            stop -= 1;
        }
        let kind = if key {
            Kind::Keyword
        } else if yaml_literal(&self.text[start..stop]) {
            Kind::Number
        } else {
            Kind::Str
        };
        self.at = stop;
        self.push(start, kind);
        self.at = at.max(start + 1);
    }

    // ---------- Markdown ----------

    fn line_end(&self, from: usize) -> usize {
        self.text[from..].iter().position(|&c| c == CR).map_or(self.text.len(), |i| from + i)
    }

    /// Antal ens tegn i træk fra `from`, højst til `end`.
    fn run_length(&self, from: usize, end: usize) -> usize {
        self.text[from..end].iter().take_while(|&&c| c == self.text[from]).count()
    }

    fn markdown(&mut self) {
        while self.at < self.text.len() {
            let end = self.line_end(self.at);
            let mut at = self.at;
            while at < end && (self.text[at] == 32 || self.text[at] == 9) {
                at += 1;
            }
            let c = if at < end { self.text[at] } else { 0 };
            let run = if at < end { self.run_length(at, end) } else { 0 };
            let rule = [b'-', b'*', b'_'].iter().any(|&b| is(c, b))
                && self.text[at..end].iter().all(|&x| x == c || x == 32)
                && self.text[at..end].iter().filter(|&&x| x == c).count() >= 3;

            if (is(c, b'`') || is(c, b'~')) && run >= 3 {
                self.markdown_fence(at, end, c, run);
                continue;
            }
            self.at = at;
            if is(c, b'#') && run <= 6 && (at + run == end || self.text[at + run] == 32) {
                // Overskrift
                self.at = end;
                self.push(at, Kind::Keyword);
            } else if is(c, b'>') || rule {
                // Citat eller vandret streg
                self.at = end;
                self.push(at, Kind::Comment);
            } else {
                self.markdown_list_marker(end);
                self.markdown_inline(end);
            }
            // Videre til næste linje, medmindre en kommentar allerede er gået forbi den.
            if self.at <= end {
                self.at = (end + 1).min(self.text.len());
            }
        }
    }

    /// `- `, `* `, `1. ` og afkrydsningsfeltet `[ ]` forrest på en linje.
    fn markdown_list_marker(&mut self, end: usize) {
        let start = self.at;
        let mut at = start;
        if [b'-', b'*', b'+'].iter().any(|&b| is(self.text[at.min(end.saturating_sub(1))], b)) && at < end {
            at += 1;
        } else {
            while at < end && is_digit(self.text[at]) {
                at += 1;
            }
            if at == start || at >= end || !(is(self.text[at], b'.') || is(self.text[at], b')')) {
                return;
            }
            at += 1;
        }
        if at < end && self.text[at] != 32 {
            return;
        }
        self.at = at;
        self.push(start, Kind::Keyword);
        while self.at < end && self.text[self.at] == 32 {
            self.at += 1;
        }
        let checkbox = is(self.peek(0), b'[')
            && (self.peek(1) == 32 || matches!(self.peek(1) as u8 | 0x20, b'x'))
            && is(self.peek(2), b']')
            && self.at + 3 <= end;
        if checkbox {
            let start = self.at;
            self.at += 3;
            self.push(start, Kind::Number);
        }
    }

    /// En kodeblok mellem ``` eller ~~~. Står der et kendt sprog efter de første, farves
    /// koden som det sprog.
    fn markdown_fence(&mut self, marker: usize, end: usize, fence: u16, run: usize) {
        self.at = end;
        self.push(marker, Kind::Comment);

        let mut info = marker + run;
        while info < end && self.text[info] == 32 {
            info += 1;
        }
        let name: String = self.text[info..end]
            .iter()
            .take_while(|&&c| c < 0x80 && ((c as u8).is_ascii_alphanumeric() || is(c, b'+') || is(c, b'#')))
            .map(|&c| (c as u8).to_ascii_lowercase() as char)
            .take(16)
            .collect();
        let lang = match name.as_str() {
            "js" | "javascript" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
            "ts" | "typescript" | "tsx" => Some(Lang::TypeScript),
            "sql" | "postgres" | "postgresql" | "pgsql" => Some(Lang::Sql),
            "json" | "jsonc" | "json5" => Some(Lang::Json),
            "html" | "htm" => Some(Lang::Html),
            "css" => Some(Lang::Css),
            "py" | "python" => Some(Lang::Python),
            "rs" | "rust" => Some(Lang::Rust),
            "cs" | "csharp" | "c#" => Some(Lang::CSharp),
            "php" => Some(Lang::Php),
            "yml" | "yaml" => Some(Lang::Yaml),
            "go" => Some(Lang::Go),
            "c" | "h" => Some(Lang::C),
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" => Some(Lang::Cpp),
            "sh" | "bash" | "zsh" | "ksh" | "shell" => Some(Lang::Bash),
            "kt" | "kts" | "kotlin" => Some(Lang::Kotlin),
            "swift" => Some(Lang::Swift),
            "rb" | "rake" | "gemspec" | "ru" | "ruby" => Some(Lang::Ruby),
            "ps1" | "psm1" | "psd1" | "powershell" | "pwsh" => Some(Lang::PowerShell),
            "lua" => Some(Lang::Lua),
            "toml" => Some(Lang::Toml),
            "dockerfile" | "docker" => Some(Lang::Dockerfile),
            "ini" | "cfg" | "conf" | "properties" | "inf" => Some(Lang::Ini),
            "bat" | "cmd" | "batch" => Some(Lang::Batch),
            "env" | "dotenv" => Some(Lang::Env),
            "vue" => Some(Lang::Vue),
            "svelte" => Some(Lang::Svelte),
            "scss" | "less" => Some(Lang::Scss),
            "java" => Some(Lang::Java),
            "xml" | "svg" | "xsd" | "xsl" | "xslt" | "xaml" | "csproj" | "props" | "targets" | "config" | "resx" | "plist" => {
                Some(Lang::Xml)
            }
            _ => None,
        };

        // Find linjen der lukker blokken: samme tegn, mindst lige så mange.
        let content = (end + 1).min(self.text.len());
        let mut line = content;
        let mut closing = None;
        while line < self.text.len() {
            let line_end = self.line_end(line);
            let mut at = line;
            while at < line_end && self.text[at] == 32 {
                at += 1;
            }
            if at < line_end && self.text[at] == fence && self.run_length(at, line_end) >= run {
                closing = Some((at, line_end));
                break;
            }
            line = line_end + 1;
        }
        let content_end = closing.map_or(self.text.len(), |_| line);

        self.at = content;
        if content_end > content {
            match lang {
                // PHP-eksempler skrives ofte uden <?php foran.
                Some(Lang::Php) if self.find_before(content, content_end, "<?").is_none() => {
                    self.embedded(content_end, |inner| inner.php_code())
                }
                Some(lang) => self.embedded(content_end, |inner| inner.run(lang)),
                None => {
                    self.at = content_end;
                    self.push(content, Kind::Str);
                }
            }
        }
        match closing {
            Some((marker, line_end)) => {
                self.at = line_end;
                self.push(marker, Kind::Comment);
                self.at = (line_end + 1).min(self.text.len());
            }
            None => self.at = self.text.len(),
        }
    }

    /// Positionen af `needle` mellem `from` og `end`.
    fn find_before(&self, from: usize, end: usize, needle: &str) -> Option<usize> {
        let needle: Vec<u16> = needle.encode_utf16().collect();
        let from = from.min(end);
        self.text[from..end].windows(needle.len()).position(|w| w == needle.as_slice()).map(|i| from + i)
    }

    /// Det der står inde i en linje: `kode`, **fed**, [links](adresse) og kommentarer.
    fn markdown_inline(&mut self, end: usize) {
        while self.at < end {
            let c = self.text[self.at];
            let start = self.at;
            if is(c, b'\\') {
                self.at = (self.at + 2).min(end);
            } else if is(c, b'`') {
                let ticks = self.run_length(start, end);
                let fence = "`".repeat(ticks);
                match self.find_before(start + ticks, end, &fence) {
                    Some(close) => {
                        self.at = close + ticks;
                        self.push(start, Kind::Str);
                    }
                    None => self.at += ticks,
                }
            } else if self.starts_with("**") || self.starts_with("__") {
                let mark = if is(c, b'*') { "**" } else { "__" };
                match self.find_before(start + 2, end, mark) {
                    Some(close) if close > start + 2 && self.text[start + 2] != 32 => {
                        self.at = close + 2;
                        self.push(start, Kind::Function);
                    }
                    _ => self.at += 2,
                }
            } else if is(c, b'[') {
                // [tekst](adresse)
                let link = self
                    .find_before(start + 1, end, "](")
                    .and_then(|middle| self.find_before(middle + 2, end, ")").map(|close| (middle, close)));
                match link {
                    Some((middle, close)) => {
                        self.at = middle + 1;
                        self.push(start, Kind::Type);
                        self.at = close + 1;
                        self.push(middle + 1, Kind::Str);
                    }
                    None => self.at += 1,
                }
            } else if self.starts_with("<!--") {
                // En kommentar må gå over flere linjer.
                self.at = self.find_before(start, self.text.len(), "-->").map_or(self.text.len(), |close| close + 3);
                self.push(start, Kind::Comment);
            } else if self.starts_with("<http") || self.starts_with("<mailto:") {
                match self.find_before(start, end, ">") {
                    Some(close) => {
                        self.at = close + 1;
                        self.push(start, Kind::Str);
                    }
                    None => self.at += 1,
                }
            } else {
                self.at += 1;
            }
        }
    }

    // ---------- JSON ----------

    /// JSON, også med kommentarer (jsonc). Nøglerne får nøgleordsfarven, så de kan
    /// skelnes fra værdierne.
    fn json(&mut self) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is_space(c) {
                self.at += 1;
            } else if self.starts_with("//") {
                self.line_comment();
            } else if self.starts_with("/*") {
                self.block_comment();
            } else if is(c, b'"') || is(c, b'\'') {
                self.quoted(c, true);
                if is(self.next_visible(self.at), b':') {
                    if let Some(key) = self.out.last_mut() {
                        key.kind = Kind::Keyword;
                    }
                }
            } else if is_digit(c) {
                self.number();
            } else if is_word_start(c) {
                let mut buffer = [0u8; 24];
                let (start, len) = self.word(&mut buffer);
                if matches!(&buffer[..len], b"true" | b"false" | b"null") {
                    self.push(start, Kind::Number);
                }
            } else {
                self.at += 1;
            }
        }
    }

    // ---------- HTML ----------

    fn html(&mut self) {
        self.markup(Markup::Html);
    }

    /// HTML og dets slægtninge. I alt andet end XML farves indholdet af <script> og
    /// <style> i deres eget sprog; Vue og Svelte har desuden kode i selve opmærkningen.
    fn markup(&mut self, mode: Markup) {
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if self.starts_with("<!--") {
                let start = self.at;
                while self.at < self.text.len() && !self.starts_with("-->") {
                    self.at += 1;
                }
                self.at = (self.at + 3).min(self.text.len());
                self.push(start, Kind::Comment);
            } else if self.starts_with("<![CDATA[") {
                // Rå tekst i XML: intet herinde er tags.
                let start = self.at;
                self.at = self.find_before(start, self.text.len(), "]]>").map_or(self.text.len(), |close| close + 3);
                self.push(start, Kind::Str);
            } else if self.starts_with("<!") || self.starts_with("<?") {
                // <!DOCTYPE html> og <?xml ... ?>
                let start = self.at;
                while self.at < self.text.len() && !is(self.text[self.at], b'>') {
                    self.at += 1;
                }
                self.at = (self.at + 1).min(self.text.len());
                self.push(start, Kind::Keyword);
            } else if is(c, b'<') && (is_word_start(self.peek(1)) || is(self.peek(1), b'/')) {
                self.html_tag(mode);
            } else if is(c, b'&') {
                self.html_entity();
            } else if mode == Markup::Vue && self.starts_with("{{") {
                // {{ udtryk }} i Vue er JavaScript.
                let start = self.at;
                self.at += 2;
                self.push(start, Kind::Keyword);
                match self.find_before(self.at, self.text.len(), "}}") {
                    Some(close) => {
                        self.embedded(close, |inner| inner.script(true));
                        self.at += 2;
                        self.push(close, Kind::Keyword);
                    }
                    None => self.embedded(self.text.len(), |inner| inner.script(true)),
                }
            } else if mode == Markup::Svelte && is(c, b'{') {
                self.svelte_block();
            } else {
                self.at += 1;
            }
        }
    }

    /// `{udtryk}`, `{#if ...}`, `{:else}` og `{/if}` i Svelte: tuborgparenteserne og
    /// blokordet er nøgleord, resten er JavaScript.
    fn svelte_block(&mut self) {
        let start = self.at;
        // Find den parentes der lukker; strenge i udtrykket kan indeholde } uden at lukke.
        let mut depth = 0u32;
        let mut at = start;
        let mut close = None;
        while at < self.text.len() {
            let c = self.text[at];
            if is(c, b'"') || is(c, b'\'') || is(c, b'`') {
                at += 1;
                while at < self.text.len() && self.text[at] != c && (self.text[at] != CR || is(c, b'`')) {
                    at += if is(self.text[at], b'\\') { 2 } else { 1 };
                }
            } else if is(c, b'{') {
                depth += 1;
            } else if is(c, b'}') {
                depth -= 1;
                if depth == 0 {
                    close = Some(at);
                    break;
                }
            }
            at += 1;
        }
        let Some(close) = close else {
            self.at += 1;
            return;
        };
        self.at += 1;
        if [b'#', b':', b'/', b'@'].iter().any(|&b| is(self.peek(0), b)) {
            self.at += 1;
            while self.at < close && is_word(self.text[self.at]) {
                self.at += 1;
            }
        }
        self.push(start, Kind::Keyword);
        self.embedded(close, |inner| inner.script(true));
        self.at = close + 1;
        self.push(close, Kind::Keyword);
    }

    /// Et tag fra `<` til `>`. Efter `<script>` og `<style>` farves indholdet som
    /// JavaScript og CSS, eller som TypeScript og SCSS hvis tagget siger `lang="ts"`
    /// eller `lang="scss"`.
    fn html_tag(&mut self, mode: Markup) {
        let tag_start = self.at;
        let framework = mode == Markup::Vue || mode == Markup::Svelte;
        self.at += 1;
        let closing = is(self.peek(0), b'/');
        if closing {
            self.at += 1;
        }
        let name_char = |c: u16| is_word(c) || is(c, b'-') || is(c, b':') || is(c, b'.');
        let start = self.at;
        while self.at < self.text.len() && name_char(self.text[self.at]) {
            self.at += 1;
        }
        let name = (start, self.at);
        // Komponenter (<OrderList>) skrives med stort og får typefarven.
        let component = framework && self.text.get(start).is_some_and(|&c| is_upper(c));
        self.push(start, if component { Kind::Type } else { Kind::Keyword });

        let mut self_closing = false;
        // Om den seneste attribut er et Vue-direktiv (:prop, @click, v-if, #slot),
        // hvis værdi er JavaScript.
        let mut directive = false;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is(c, b'>') {
                self.at += 1;
                break;
            } else if is(c, b'<') {
                // Tagget blev aldrig lukket; det næste begynder her.
                return;
            } else if is_space(c) {
                self.at += 1;
            } else if is(c, b'/') {
                self_closing = true;
                self.at += 1;
            } else if mode == Markup::Svelte && is(c, b'{') {
                // {værdi} som attribut eller attributværdi.
                self.svelte_block();
            } else if is(c, b'=') {
                self.at += 1;
                while self.at < self.text.len() && is_space(self.text[self.at]) {
                    self.at += 1;
                }
                let start = self.at;
                let quote = self.peek(0);
                if mode == Markup::Svelte && is(quote, b'{') {
                    continue;
                }
                if is(quote, b'"') || is(quote, b'\'') {
                    // Værdier i anførselstegn må gerne gå over flere linjer.
                    self.at += 1;
                    while self.at < self.text.len() && self.text[self.at] != quote {
                        self.at += 1;
                    }
                    let close = self.at;
                    self.at = (self.at + 1).min(self.text.len());
                    if directive && mode == Markup::Vue {
                        self.at = start + 1;
                        self.embedded(close, |inner| inner.script(true));
                        self.at = (close + 1).min(self.text.len());
                        continue;
                    }
                } else {
                    while self.at < self.text.len() && !is_space(self.text[self.at]) && !is(self.text[self.at], b'>') {
                        self.at += 1;
                    }
                }
                self.push(start, Kind::Str);
            } else {
                self_closing = false;
                let start = self.at;
                while self.at < self.text.len() {
                    let c = self.text[self.at];
                    if is_space(c) || [b'=', b'>', b'/', b'<'].iter().any(|&b| is(c, b)) {
                        break;
                    }
                    self.at += 1;
                }
                let first = self.text[start];
                directive = [b':', b'@', b'#'].iter().any(|&b| is(first, b)) || (is(first, b'v') && is(self.text.get(start + 1).copied().unwrap_or(0), b'-'));
                self.push(start, Kind::Type);
            }
        }

        if closing || self_closing || mode == Markup::Xml {
            return;
        }
        let tag = (tag_start, self.at);
        let name = &self.text[name.0..name.1];
        let named = |tag: &str| name.len() == tag.len() && name.iter().zip(tag.bytes()).all(|(&c, b)| c < 0x80 && (c as u8).to_ascii_lowercase() == b);
        if named("script") {
            let typescript = ["ts", "tsx", "typescript"].iter().any(|lang| self.tag_lang(tag, lang));
            let end = self.find_closing("</script");
            self.embedded(end, |inner| inner.script(typescript));
        } else if named("style") {
            let scss = ["scss", "less", "sass"].iter().any(|lang| self.tag_lang(tag, lang));
            let end = self.find_closing("</style");
            self.embedded(end, |inner| inner.stylesheet(scss));
        }
    }

    /// Om tagget i `tag` har attributten `lang` sat til `lang`.
    fn tag_lang(&self, tag: (usize, usize), lang: &str) -> bool {
        ['"', '\''].iter().any(|quote| self.find_before(tag.0, tag.1, &format!("lang={quote}{lang}{quote}")).is_some())
    }

    /// Positionen af det næste `needle` (uanset store og små bogstaver), ellers tekstens ende.
    fn find_closing(&self, needle: &str) -> usize {
        let needle = needle.as_bytes();
        let matches = |window: &[u16]| window.iter().zip(needle).all(|(&c, &b)| c < 0x80 && (c as u8).to_ascii_lowercase() == b);
        self.text[self.at..]
            .windows(needle.len())
            .position(matches)
            .map_or(self.text.len(), |found| self.at + found)
    }

    /// Farver teksten herfra til `end` som et andet sprog.
    fn embedded(&mut self, end: usize, scan: impl FnOnce(&mut Scanner)) {
        let mut inner = Scanner { text: &self.text[..end], at: self.at, out: std::mem::take(&mut self.out) };
        scan(&mut inner);
        self.out = inner.out;
        self.at = end;
    }

    /// `&amp;`, `&#160;` og lignende.
    fn html_entity(&mut self) {
        let start = self.at;
        let mut at = self.at + 1;
        while at - start < 34 && self.text.get(at).is_some_and(|&c| c < 0x80 && ((c as u8).is_ascii_alphanumeric() || is(c, b'#'))) {
            at += 1;
        }
        if at > start + 1 && self.text.get(at).is_some_and(|&c| is(c, b';')) {
            self.at = at + 1;
            self.push(start, Kind::Number);
        } else {
            self.at += 1;
        }
    }

    // ---------- CSS ----------

    fn css(&mut self) {
        self.stylesheet(false);
    }

    /// CSS, og med `scss` også SCSS og Less: `//`-kommentarer, `$variabler` og `#{...}`.
    fn stylesheet(&mut self, scss: bool) {
        #[derive(PartialEq)]
        enum Mode {
            /// Starten af en regel eller en erklæring; hvilken af dem afgøres af det der følger.
            Start,
            Selector,
            Property,
            Value,
        }
        let mut mode = Mode::Start;
        let mut depth = 0u32;
        let mut in_brackets = false;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if is_space(c) {
                self.at += 1;
                continue;
            }
            if self.starts_with("/*") {
                self.block_comment();
                continue;
            }
            if scss && self.starts_with("//") {
                self.line_comment();
                continue;
            }
            if scss && self.starts_with("#{") {
                // #{$navn}: en variabel sat ind midt i et navn eller en værdi.
                let start = self.at;
                self.at += 2;
                self.push(start, Kind::Keyword);
                let end = self.line_end(self.at);
                let close = self.find_before(self.at, end, "}").unwrap_or(end);
                while self.at < close {
                    let inner = self.at;
                    if is(self.text[self.at], b'$') {
                        self.at += 1;
                        self.css_ident();
                        self.push(inner, Kind::Type);
                    } else {
                        self.at += 1;
                    }
                }
                self.at = close;
                if close < end {
                    self.at += 1;
                    self.push(close, Kind::Keyword);
                }
                if mode == Mode::Start {
                    mode = if self.css_block_follows(depth) { Mode::Selector } else { Mode::Property };
                }
                continue;
            }
            if is(c, b'{') || is(c, b'}') || is(c, b';') {
                depth = if is(c, b'{') { depth + 1 } else if is(c, b'}') { depth.saturating_sub(1) } else { depth };
                self.at += 1;
                mode = Mode::Start;
                continue;
            }
            if is(c, b'"') || is(c, b'\'') {
                self.quoted(c, true);
                continue;
            }
            if mode == Mode::Start {
                in_brackets = false;
                if is(c, b'@') {
                    // @media, @import ...: resten af linjen læses som værdier.
                    let start = self.at;
                    self.at += 1;
                    self.css_ident();
                    self.push(start, Kind::Keyword);
                    mode = Mode::Value;
                    continue;
                }
                mode = if self.css_block_follows(depth) { Mode::Selector } else { Mode::Property };
            }
            let start = self.at;
            if scss && is(c, b'$') && is_css_ident(self.peek(1)) {
                // $variabel, både hvor den erklæres og hvor den bruges.
                self.at += 1;
                self.css_ident();
                self.push(start, Kind::Type);
                continue;
            }
            match mode {
                Mode::Selector => {
                    if is(c, b'[') || is(c, b']') {
                        in_brackets = is(c, b'[');
                        self.at += 1;
                    } else if (is(c, b'.') || is(c, b'#') || (scss && is(c, b'%'))) && is_css_ident(self.peek(1)) {
                        self.at += 1;
                        self.css_ident();
                        self.push(start, Kind::Function);
                    } else if is(c, b':') {
                        while is(self.peek(0), b':') {
                            self.at += 1;
                        }
                        self.css_ident();
                        if self.at > start + 1 && !is(self.text[self.at - 1], b':') {
                            self.push(start, Kind::Type);
                        }
                    } else if is_digit(c) {
                        self.css_number();
                    } else if is_css_ident(c) {
                        self.css_ident();
                        self.push(start, if in_brackets { Kind::Type } else { Kind::Keyword });
                    } else {
                        self.at += 1;
                    }
                }
                Mode::Property => {
                    if is(c, b':') {
                        self.at += 1;
                        mode = Mode::Value;
                    } else if is_css_ident(c) {
                        self.css_ident();
                        self.push(start, Kind::Keyword);
                    } else {
                        self.at += 1;
                    }
                }
                _ => self.css_value(),
            }
        }
    }

    /// Ét stykke af en værdi: et tal med enhed, en farve, et funktionskald eller et ord.
    fn css_value(&mut self) {
        let start = self.at;
        let c = self.text[self.at];
        let next = self.peek(1);
        if is(c, b'#') && next < 0x80 && (next as u8).is_ascii_hexdigit() {
            self.at += 1;
            while self.at < self.text.len() && is_word(self.text[self.at]) {
                self.at += 1;
            }
            self.push(start, Kind::Number);
        } else if is_digit(c) || (is(c, b'.') && is_digit(next)) {
            self.css_number();
        } else if (is(c, b'-') || is(c, b'+')) && (is_digit(next) || is(next, b'.')) {
            self.at += 1;
            self.css_number();
            // Fortegnet hører med til tallet.
            if let Some(number) = self.out.last_mut() {
                number.start = start;
            }
        } else if is(c, b'!') {
            self.at += 1;
            self.css_ident();
            self.push(start, Kind::Keyword);
        } else if is_css_ident(c) {
            self.css_ident();
            if !is(self.peek(0), b'(') {
                return;
            }
            self.push(start, Kind::Function);
            let url = self.at - start == 3 && self.text[start..self.at].iter().zip(b"url").all(|(&c, &b)| c == b as u16);
            if url {
                // url(billede.png) uden anførselstegn: indholdet er en streng.
                self.at += 1;
                while self.at < self.text.len() && is_space(self.text[self.at]) && self.text[self.at] != CR {
                    self.at += 1;
                }
                let content = self.at;
                if !is(self.peek(0), b'"') && !is(self.peek(0), b'\'') {
                    while self.at < self.text.len() && !is(self.text[self.at], b')') && self.text[self.at] != CR {
                        self.at += 1;
                    }
                    self.push(content, Kind::Str);
                }
            }
        } else {
            self.at += 1;
        }
    }

    fn css_ident(&mut self) {
        while self.at < self.text.len() && is_css_ident(self.text[self.at]) {
            self.at += 1;
        }
    }

    /// Et tal med eventuel enhed: `12`, `1.5em`, `50%`.
    fn css_number(&mut self) {
        let start = self.at;
        while self.at < self.text.len() && (is_digit(self.text[self.at]) || (is(self.text[self.at], b'.') && is_digit(self.peek(1)))) {
            self.at += 1;
        }
        if is(self.peek(0), b'%') {
            self.at += 1;
        } else {
            while self.at < self.text.len() && self.text[self.at] < 0x80 && (self.text[self.at] as u8).is_ascii_alphabetic() {
                self.at += 1;
            }
        }
        self.push(start, Kind::Number);
    }

    /// Om det der starter her er en regel (`a:hover {`) og ikke en erklæring
    /// (`color: red;`): kommer der en `{` før næste `;` eller `}`?
    fn css_block_follows(&self, depth: u32) -> bool {
        let mut at = self.at;
        let mut parens = 0u32;
        while at < self.text.len() {
            let c = self.text[at];
            if is(c, b'"') || is(c, b'\'') {
                at += 1;
                while at < self.text.len() && self.text[at] != c && self.text[at] != CR {
                    at += if is(self.text[at], b'\\') { 2 } else { 1 };
                }
            } else if is(c, b'/') && self.text.get(at + 1).is_some_and(|&n| is(n, b'*')) {
                at += 2;
                while at < self.text.len() && !(is(self.text[at], b'*') && self.text.get(at + 1).is_some_and(|&n| is(n, b'/'))) {
                    at += 1;
                }
                at += 1;
            } else if is(c, b'#') && self.text.get(at + 1).is_some_and(|&n| is(n, b'{')) {
                // #{...} i SCSS åbner ikke en blok.
                while at < self.text.len() && !is(self.text[at], b'}') && self.text[at] != CR {
                    at += 1;
                }
            } else if is(c, b'(') {
                parens += 1;
            } else if is(c, b')') {
                parens = parens.saturating_sub(1);
            } else if parens == 0 && is(c, b'{') {
                return true;
            } else if parens == 0 && (is(c, b';') || is(c, b'}')) {
                return false;
            }
            at += 1;
        }
        // Ufærdig tekst: yderst er det en regel på vej, inde i en blok en erklæring.
        depth == 0
    }
}

/// Tal, sandt/falsk og tom værdi i YAML: `8080`, `-1.5`, `0x1F`, `true`, `no`, `~`.
fn yaml_literal(value: &[u16]) -> bool {
    if value.is_empty() || value.len() > 32 || value.iter().any(|&c| c >= 0x80) {
        return false;
    }
    let text: String = value.iter().map(|&c| (c as u8).to_ascii_lowercase() as char).filter(|&c| c != '_').collect();
    if matches!(text.as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~") {
        return true;
    }
    let digits = text.trim_start_matches(['+', '-']);
    if let Some(hex) = digits.strip_prefix("0x") {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    digits.starts_with(|c: char| c.is_ascii_digit() || c == '.') && digits.parse::<f64>().is_ok()
}

fn dockerfile_instruction(word: &str) -> bool {
    matches!(
        word,
        "add" | "arg" | "cmd" | "copy" | "entrypoint" | "env" | "expose" | "from" | "healthcheck"
            | "label" | "maintainer" | "onbuild" | "run" | "shell" | "stopsignal" | "user" | "volume"
            | "workdir"
    )
}

/// Nøgleord i batch-filer, i små bogstaver.
fn batch_keyword(word: &str) -> bool {
    matches!(
        word,
        "call" | "defined" | "do" | "else" | "endlocal" | "equ" | "errorlevel" | "exist" | "exit"
            | "for" | "geq" | "goto" | "gtr" | "if" | "in" | "leq" | "lss" | "neq" | "not" | "set"
            | "setlocal" | "shift"
    )
}

/// Almindelige kommandoer, der også genkendes midt på en linje (`if exist x del x`).
fn batch_command(word: &str) -> bool {
    matches!(
        word,
        "cd" | "chcp" | "chdir" | "cls" | "copy" | "del" | "dir" | "echo" | "erase" | "md" | "mkdir"
            | "move" | "pause" | "popd" | "pushd" | "rd" | "ren" | "rename" | "rmdir" | "robocopy"
            | "start" | "timeout" | "title" | "type" | "xcopy"
    )
}

/// Nøgleord i PowerShell, i små bogstaver; sproget skelner ikke mellem store og små.
fn powershell_keyword(word: &str) -> bool {
    matches!(
        word,
        "begin" | "break" | "catch" | "class" | "continue" | "data" | "do" | "dynamicparam" | "else"
            | "elseif" | "end" | "enum" | "exit" | "filter" | "finally" | "for" | "foreach"
            | "function" | "hidden" | "if" | "in" | "param" | "process" | "return" | "static"
            | "switch" | "throw" | "trap" | "try" | "until" | "using" | "while"
    )
}

fn powershell_operator(word: &str) -> bool {
    matches!(
        word,
        "and" | "as" | "band" | "bor" | "bxor" | "ccontains" | "ceq" | "clike" | "cmatch" | "cne"
            | "contains" | "eq" | "f" | "ge" | "gt" | "ieq" | "in" | "is" | "isnot" | "join" | "le"
            | "like" | "lt" | "match" | "ne" | "not" | "notcontains" | "notin" | "notlike"
            | "notmatch" | "or" | "replace" | "shl" | "shr" | "split" | "xor"
    )
}

fn lua_keyword(word: &str) -> bool {
    matches!(
        word,
        "and" | "break" | "do" | "else" | "elseif" | "end" | "for" | "function" | "goto" | "if" | "in"
            | "local" | "not" | "or" | "repeat" | "return" | "self" | "then" | "until" | "while"
    )
}

/// Det der adskiller sprogene i `Scanner::curly`.
struct Curly {
    keyword: fn(&str) -> bool,
    /// Ord der kun er nøgleord, når de ikke bruges som funktionsnavn.
    soft: fn(&str) -> bool,
    literal: fn(&str) -> bool,
    /// Om 'x' er et tegn.
    char_literals: bool,
    /// Om #ord er et direktiv.
    hash_words: bool,
}

const KOTLIN: Curly = Curly {
    keyword: |word| {
        matches!(
            word,
            "as" | "break" | "catch" | "class" | "constructor" | "continue" | "do" | "else" | "finally"
                | "for" | "fun" | "if" | "import" | "in" | "init" | "interface" | "is" | "object"
                | "package" | "return" | "super" | "this" | "throw" | "try" | "typealias" | "val"
                | "var" | "when" | "while"
        )
    },
    soft: |word| {
        matches!(
            word,
            "abstract" | "actual" | "annotation" | "by" | "companion" | "const" | "crossinline" | "data"
                | "enum" | "expect" | "external" | "final" | "get" | "infix" | "inline" | "inner"
                | "internal" | "lateinit" | "noinline" | "open" | "operator" | "out" | "override"
                | "private" | "protected" | "public" | "reified" | "sealed" | "set" | "suspend"
                | "tailrec" | "value" | "vararg" | "where"
        )
    },
    literal: |word| matches!(word, "true" | "false" | "null"),
    char_literals: true,
    hash_words: false,
};

const SWIFT: Curly = Curly {
    keyword: |word| {
        matches!(
            word,
            "Self" | "actor" | "any" | "as" | "associatedtype" | "async" | "await" | "break" | "case"
                | "catch" | "class" | "continue" | "default" | "defer" | "deinit" | "do" | "else"
                | "enum" | "extension" | "fallthrough" | "fileprivate" | "for" | "func" | "guard"
                | "if" | "import" | "in" | "init" | "inout" | "internal" | "is" | "let" | "open"
                | "operator" | "private" | "protocol" | "public" | "repeat" | "rethrows" | "return"
                | "self" | "some" | "static" | "struct" | "subscript" | "super" | "switch" | "throw"
                | "throws" | "try" | "typealias" | "var" | "where" | "while"
        )
    },
    soft: |word| {
        matches!(
            word,
            "convenience" | "didSet" | "dynamic" | "final" | "get" | "indirect" | "lazy" | "mutating"
                | "nonmutating" | "optional" | "override" | "required" | "set" | "unowned" | "weak"
                | "willSet"
        )
    },
    literal: |word| matches!(word, "true" | "false" | "nil"),
    char_literals: false,
    hash_words: true,
};

fn ruby_keyword(word: &str) -> bool {
    matches!(
        word,
        "BEGIN" | "END" | "alias" | "and" | "attr_accessor" | "attr_reader" | "attr_writer" | "begin"
            | "break" | "case" | "class" | "def" | "defined" | "do" | "else" | "elsif" | "end"
            | "ensure" | "extend" | "for" | "if" | "in" | "include" | "lambda" | "loop" | "module"
            | "next" | "not" | "or" | "private" | "proc" | "protected" | "public" | "raise" | "redo"
            | "require" | "require_relative" | "rescue" | "retry" | "return" | "self" | "super"
            | "then" | "undef" | "unless" | "until" | "when" | "while" | "yield"
    )
}

fn c_keyword(word: &str) -> bool {
    matches!(
        word,
        "auto" | "break" | "case" | "const" | "continue" | "default" | "do" | "else" | "enum" | "extern"
            | "for" | "goto" | "if" | "inline" | "register" | "restrict" | "return" | "sizeof"
            | "static" | "struct" | "switch" | "typedef" | "union" | "volatile" | "while"
    )
}

fn c_type(word: &str) -> bool {
    matches!(
        word,
        "bool" | "char" | "double" | "float" | "int" | "long" | "short" | "signed" | "unsigned" | "void"
    )
}

fn cpp_keyword(word: &str) -> bool {
    matches!(
        word,
        "alignas" | "alignof" | "and" | "catch" | "class" | "co_await" | "co_return" | "co_yield"
            | "concept" | "const_cast" | "consteval" | "constexpr" | "constinit" | "decltype"
            | "delete" | "dynamic_cast" | "explicit" | "export" | "final" | "friend" | "mutable"
            | "namespace" | "new" | "noexcept" | "not" | "operator" | "or" | "override" | "private"
            | "protected" | "public" | "reinterpret_cast" | "requires" | "static_assert"
            | "static_cast" | "template" | "this" | "thread_local" | "throw" | "try" | "typeid"
            | "typename" | "using" | "virtual" | "xor"
    )
}

fn cpp_type(word: &str) -> bool {
    matches!(word, "string" | "vector" | "map" | "set" | "optional" | "unique_ptr" | "shared_ptr" | "string_view")
}

fn go_keyword(word: &str) -> bool {
    matches!(
        word,
        "break" | "case" | "chan" | "const" | "continue" | "default" | "defer" | "else" | "fallthrough"
            | "for" | "func" | "go" | "goto" | "if" | "import" | "interface" | "map" | "package"
            | "range" | "return" | "select" | "struct" | "switch" | "type" | "var"
    )
}

fn go_type(word: &str) -> bool {
    matches!(
        word,
        "any" | "bool" | "byte" | "complex128" | "complex64" | "error" | "float32" | "float64" | "int"
            | "int16" | "int32" | "int64" | "int8" | "rune" | "string" | "uint" | "uint16" | "uint32"
            | "uint64" | "uint8" | "uintptr"
    )
}

fn java_keyword(word: &str) -> bool {
    matches!(
        word,
        "abstract" | "assert" | "break" | "case" | "catch" | "class" | "const" | "continue" | "default"
            | "do" | "else" | "enum" | "extends" | "final" | "finally" | "for" | "goto" | "if"
            | "implements" | "import" | "instanceof" | "interface" | "native" | "new" | "package"
            | "private" | "protected" | "public" | "return" | "static" | "strictfp" | "super"
            | "switch" | "synchronized" | "this" | "throw" | "throws" | "transient" | "try"
            | "volatile" | "while"
    )
}

fn java_type(word: &str) -> bool {
    matches!(word, "boolean" | "byte" | "char" | "double" | "float" | "int" | "long" | "short" | "void")
}

fn csharp_keyword(word: &str) -> bool {
    matches!(
        word,
        "abstract" | "as" | "base" | "break" | "case" | "catch" | "checked" | "class" | "const"
            | "continue" | "default" | "delegate" | "do" | "else" | "enum" | "event" | "explicit"
            | "extern" | "finally" | "fixed" | "for" | "foreach" | "goto" | "if" | "implicit" | "in"
            | "interface" | "internal" | "is" | "lock" | "namespace" | "new" | "operator" | "out"
            | "override" | "params" | "private" | "protected" | "public" | "readonly" | "ref"
            | "return" | "sealed" | "sizeof" | "stackalloc" | "static" | "struct" | "switch" | "this"
            | "throw" | "try" | "typeof" | "unchecked" | "unsafe" | "using" | "virtual" | "volatile"
            | "while"
    )
}

/// Ord der kun er nøgleord i deres sammenhæng; som metodenavne (`Get(...)`) er de ikke.
fn csharp_contextual(word: &str) -> bool {
    matches!(
        word,
        "and" | "async" | "await" | "get" | "global" | "init" | "nameof" | "not" | "or" | "partial"
            | "record" | "required" | "set" | "var" | "when" | "where" | "with" | "yield"
    )
}

fn csharp_type(word: &str) -> bool {
    matches!(
        word,
        "bool" | "byte" | "char" | "decimal" | "double" | "dynamic" | "float" | "int" | "long"
            | "nint" | "nuint" | "object" | "sbyte" | "short" | "string" | "uint" | "ulong"
            | "ushort" | "void"
    )
}

fn php_keyword(word: &str) -> bool {
    matches!(
        word,
        "abstract" | "and" | "array" | "as" | "break" | "case" | "catch" | "class" | "clone"
            | "const" | "continue" | "declare" | "default" | "do" | "echo" | "else" | "elseif"
            | "empty" | "enddeclare" | "endfor" | "endforeach" | "endif" | "endswitch" | "endwhile"
            | "enum" | "extends" | "final" | "finally" | "fn" | "for" | "foreach" | "function"
            | "global" | "goto" | "if" | "implements" | "include" | "include_once" | "instanceof"
            | "insteadof" | "interface" | "isset" | "list" | "match" | "namespace" | "new" | "or"
            | "parent" | "print" | "private" | "protected" | "public" | "readonly" | "require"
            | "require_once" | "return" | "self" | "static" | "switch" | "throw" | "trait" | "try"
            | "unset" | "use" | "var" | "while" | "xor" | "yield"
    )
}

fn php_type(word: &str) -> bool {
    matches!(
        word,
        "bool" | "callable" | "float" | "int" | "iterable" | "mixed" | "never" | "object" | "string" | "void"
    )
}

fn python_keyword(word: &str) -> bool {
    matches!(
        word,
        "and" | "as" | "assert" | "async" | "await" | "break" | "class" | "continue" | "def" | "del"
            | "elif" | "else" | "except" | "finally" | "for" | "from" | "global" | "if" | "import"
            | "in" | "is" | "lambda" | "nonlocal" | "not" | "or" | "pass" | "raise" | "return"
            | "try" | "while" | "with" | "yield"
    )
}

fn python_type(word: &str) -> bool {
    matches!(
        word,
        "bool" | "bytearray" | "bytes" | "complex" | "dict" | "float" | "frozenset" | "int" | "list"
            | "object" | "set" | "str" | "tuple"
    )
}

fn rust_keyword(word: &str) -> bool {
    matches!(
        word,
        "Self" | "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
            | "enum" | "extern" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match"
            | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self" | "static" | "struct"
            | "super" | "trait" | "type" | "unsafe" | "use" | "where" | "while"
    )
}

fn rust_type(word: &str) -> bool {
    matches!(
        word,
        "bool" | "char" | "f32" | "f64" | "i128" | "i16" | "i32" | "i64" | "i8" | "isize" | "str"
            | "u128" | "u16" | "u32" | "u64" | "u8" | "usize"
    )
}

fn is_css_ident(c: u16) -> bool {
    (is_word(c) && !is(c, b'$')) || is(c, b'-')
}

fn script_keyword(word: &str) -> bool {
    matches!(
        word,
        "async" | "await" | "break" | "case" | "catch" | "class" | "const" | "continue" | "debugger"
            | "default" | "delete" | "do" | "else" | "export" | "extends" | "finally" | "for"
            | "function" | "if" | "import" | "in" | "instanceof" | "let" | "new" | "of" | "return"
            | "super" | "switch" | "this" | "throw" | "try" | "typeof" | "var" | "void" | "while"
            | "with" | "yield"
    )
}

fn contextual_keyword(word: &str) -> bool {
    matches!(word, "as" | "from" | "get" | "set" | "static")
}

fn typescript_keyword(word: &str) -> bool {
    matches!(word, "enum" | "implements" | "interface" | "private" | "protected" | "public")
}

fn typescript_contextual(word: &str) -> bool {
    matches!(
        word,
        "abstract" | "accessor" | "asserts" | "declare" | "infer" | "is" | "keyof" | "module"
            | "namespace" | "override" | "readonly" | "satisfies" | "type" | "unique"
    )
}

fn typescript_type(word: &str) -> bool {
    matches!(
        word,
        "any" | "bigint" | "boolean" | "never" | "number" | "object" | "string" | "symbol" | "unknown"
    )
}

fn sql_keyword(word: &str) -> bool {
    matches!(
        word,
        "add" | "after" | "all" | "alter" | "always" | "analyze" | "and" | "any" | "as" | "asc"
            | "authorization" | "before" | "begin" | "between" | "by" | "cascade" | "case" | "check"
            | "column" | "comment" | "commit" | "conflict" | "constraint" | "create" | "cross"
            | "current_date" | "current_time" | "current_timestamp" | "current_user" | "cursor"
            | "database" | "declare" | "default" | "deferrable" | "definer" | "delete" | "desc"
            | "distinct" | "do" | "drop" | "each" | "else" | "elsif" | "enable" | "end" | "except"
            | "exception" | "exec" | "execute" | "exists" | "explain" | "extension" | "false"
            | "fetch" | "filter" | "first" | "for" | "foreign" | "from" | "full" | "function"
            | "generated" | "go" | "grant" | "group" | "having" | "identity" | "if" | "ilike"
            | "immutable" | "in" | "index" | "inner" | "insert" | "instead" | "intersect" | "into"
            | "invoker" | "is" | "join" | "key" | "language" | "last" | "lateral" | "left" | "level"
            | "like" | "limit" | "loop" | "materialized" | "merge" | "natural" | "new" | "no" | "not"
            | "nothing" | "notnull" | "null" | "nulls" | "of" | "offset" | "old" | "on" | "only"
            | "or" | "order" | "outer" | "over" | "owner" | "partition" | "perform" | "policy"
            | "primary" | "procedure" | "public" | "raise" | "recursive" | "references"
            | "refresh" | "rename" | "replace" | "restrict" | "return" | "returning" | "returns"
            | "revoke" | "right" | "role" | "rollback" | "row" | "rows" | "schema" | "security"
            | "select" | "sequence" | "set" | "setof" | "similar" | "stable" | "start" | "table"
            | "temp" | "temporary" | "then" | "to" | "top" | "transaction" | "trigger" | "true"
            | "truncate" | "union" | "unique" | "update" | "using" | "values" | "view" | "volatile"
            | "when" | "where" | "while" | "window" | "with" | "without"
    )
}

fn sql_type(word: &str) -> bool {
    matches!(
        word,
        "bigint" | "bigserial" | "binary" | "bit" | "blob" | "bool" | "boolean" | "bytea" | "char"
            | "character" | "citext" | "clob" | "date" | "datetime" | "datetime2" | "decimal"
            | "double" | "float" | "inet" | "int" | "int2" | "int4" | "int8" | "integer" | "interval"
            | "json" | "jsonb" | "money" | "nchar" | "ntext" | "numeric" | "nvarchar" | "precision"
            | "real" | "serial" | "smallint" | "smallserial" | "text" | "time" | "timestamp"
            | "timestamptz" | "tinyint" | "uuid" | "varbinary" | "varchar" | "varchar2" | "varying"
            | "void" | "xml" | "zone"
    )
}

#[cfg(test)]
mod tests {
    use super::Kind::*;
    use super::*;

    /// De farvede stykker som (tekst, slags), så testene kan læses.
    fn tokens(lang: Lang, source: &str) -> Vec<(String, Kind)> {
        let text: Vec<u16> = source.replace('\n', "\r").encode_utf16().collect();
        tokenize(lang, &text)
            .into_iter()
            .map(|t| (String::from_utf16(&text[t.start..t.end]).unwrap().replace('\r', "\n"), t.kind))
            .collect()
    }

    fn js(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::JavaScript, source)
    }

    fn ts(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::TypeScript, source)
    }

    fn sql(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Sql, source)
    }

    fn expect(actual: Vec<(String, Kind)>, expected: &[(&str, Kind)]) {
        let expected: Vec<(String, Kind)> = expected.iter().map(|&(s, k)| (s.to_string(), k)).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn language_follows_the_file_extension() {
        assert_eq!(Lang::of(Path::new("a/app.JS")), Some(Lang::JavaScript));
        assert_eq!(Lang::of(Path::new("page.tsx")), Some(Lang::TypeScript));
        assert_eq!(Lang::of(Path::new("001_init.sql")), Some(Lang::Sql));
        assert_eq!(Lang::of(Path::new(".env")), Some(Lang::Env));
        assert_eq!(Lang::of(Path::new("notes.txt")), None);
    }

    #[test]
    fn js_keywords_strings_numbers_and_calls() {
        expect(
            js("const total = sum(1, 0x1F) + 'kr';"),
            &[("const", Keyword), ("sum", Function), ("1", Number), ("0x1F", Number), ("'kr'", Str)],
        );
    }

    #[test]
    fn js_comments() {
        expect(
            js("// en linje\nlet a; /* flere\nlinjer */ let b"),
            &[("// en linje", Comment), ("let", Keyword), ("/* flere\nlinjer */", Comment), ("let", Keyword)],
        );
        expect(js("#!/usr/bin/env node\nlet a"), &[("#!/usr/bin/env node", Comment), ("let", Keyword)]);
        expect(js("/* aldrig lukket"), &[("/* aldrig lukket", Comment)]);
    }

    #[test]
    fn js_string_stops_at_the_end_of_the_line() {
        expect(js("a = 'åben\nlet b"), &[("'åben", Str), ("let", Keyword)]);
        expect(js(r#"a = "med \" i""#), &[(r#""med \" i""#, Str)]);
    }

    #[test]
    fn js_template_string_colors_the_code_inside_it() {
        expect(
            js("`a ${f(1)} b ${ {x: 2}.x } c`"),
            &[
                ("`a ", Str),
                ("${", Keyword),
                ("f", Function),
                ("1", Number),
                ("}", Keyword),
                (" b ", Str),
                ("${", Keyword),
                ("2", Number),
                ("}", Keyword),
                (" c`", Str),
            ],
        );
        expect(js("`flere\nlinjer` let"), &[("`flere\nlinjer`", Str), ("let", Keyword)]);
    }

    #[test]
    fn js_regex_is_told_apart_from_division() {
        expect(js("x = /a[/]b+/gi;"), &[("/a[/]b+/gi", Str)]);
        expect(js("return /\\d+/.test(s)"), &[("return", Keyword), ("/\\d+/", Str), ("test", Function)]);
        expect(js("a / b / c"), &[]);
        expect(js("f(a) / 2"), &[("f", Function), ("2", Number)]);
    }

    #[test]
    fn js_property_names_are_not_keywords() {
        expect(js("x.default.new"), &[]);
        expect(js("m.get(k)"), &[("get", Function)]);
        expect(js("[...this.items]"), &[("this", Keyword)]);
    }

    #[test]
    fn js_contextual_keywords_need_something_after_them() {
        expect(js("import x from 'y'"), &[("import", Keyword), ("from", Keyword), ("'y'", Str)]);
        expect(js("const from = 1"), &[("const", Keyword), ("1", Number)]);
        expect(js("static get size() {}"), &[("static", Keyword), ("get", Keyword), ("size", Function)]);
        expect(js("get(1)"), &[("get", Function), ("1", Number)]);
    }

    #[test]
    fn js_types_constants_and_literals() {
        expect(
            js("new Map(); MAX_SIZE; true; null"),
            &[("new", Keyword), ("Map", Type), ("true", Number), ("null", Number)],
        );
    }

    #[test]
    fn js_numbers() {
        expect(
            js("1_000 1.5e-3 .5 10n 0b101 1..toFixed()"),
            &[
                ("1_000", Number),
                ("1.5e-3", Number),
                (".5", Number),
                ("10n", Number),
                ("0b101", Number),
                ("1", Number),
                ("toFixed", Function),
            ],
        );
        expect(js("a = 1-2"), &[("1", Number), ("2", Number)]);
    }

    #[test]
    fn ts_adds_types_and_its_own_keywords() {
        expect(
            ts("interface User { readonly id: number; name: string }"),
            &[("interface", Keyword), ("User", Type), ("readonly", Keyword), ("number", Type), ("string", Type)],
        );
        expect(
            ts("type Id = string | number"),
            &[("type", Keyword), ("Id", Type), ("string", Type), ("number", Type)],
        );
        expect(ts("const type = 1"), &[("const", Keyword), ("1", Number)]);
        // I JavaScript er de samme ord bare navne.
        expect(js("interface; number"), &[]);
    }

    #[test]
    fn sql_keywords_are_case_insensitive() {
        expect(
            sql("SELECT id, Count(*) from users WHERE age >= 18"),
            &[("SELECT", Keyword), ("Count", Function), ("from", Keyword), ("WHERE", Keyword), ("18", Number)],
        );
    }

    #[test]
    fn sql_comments_and_strings() {
        expect(
            sql("-- note\nselect 'it''s' /* x\ny */, N'æ'"),
            &[
                ("-- note", Comment),
                ("select", Keyword),
                ("'it''s'", Str),
                ("/* x\ny */", Comment),
                ("N'æ'", Str),
            ],
        );
        expect(sql("'flere\nlinjer' and"), &[("'flere\nlinjer'", Str), ("and", Keyword)]);
    }

    #[test]
    fn sql_types_and_quoted_names() {
        expect(
            sql("create table \"order\" ([from] int, note varchar(50), t.select uuid)"),
            &[
                ("create", Keyword),
                ("table", Keyword),
                ("int", Type),
                ("varchar", Type),
                ("50", Number),
                ("uuid", Type),
            ],
        );
    }

    #[test]
    fn sql_variables_and_dollar_quotes() {
        expect(sql("where id = @from and x = :to and y = $1"), &[("where", Keyword), ("and", Keyword), ("and", Keyword)]);
        expect(sql("select now()::timestamptz, @@rowcount"), &[("select", Keyword), ("now", Function), ("timestamptz", Type)]);
        expect(
            sql("as $$ begin return 1; end $fn$"),
            &[
                ("as", Keyword),
                ("$$", Str),
                ("begin", Keyword),
                ("return", Keyword),
                ("1", Number),
                ("end", Keyword),
                ("$fn$", Str),
            ],
        );
    }

    fn json(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Json, source)
    }

    fn html(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Html, source)
    }

    fn css(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Css, source)
    }

    #[test]
    fn new_languages_follow_the_file_extension() {
        assert_eq!(Lang::of(Path::new("package.json")), Some(Lang::Json));
        assert_eq!(Lang::of(Path::new("tsconfig.JSONC")), Some(Lang::Json));
        assert_eq!(Lang::of(Path::new("index.html")), Some(Lang::Html));
        assert_eq!(Lang::of(Path::new("old.htm")), Some(Lang::Html));
        assert_eq!(Lang::of(Path::new("site.css")), Some(Lang::Css));
    }

    #[test]
    fn json_keys_are_told_apart_from_values() {
        expect(
            json("{ \"name\": \"sEdit\", \"n\" : -1.5e3, \"ok\": true, \"x\": null }"),
            &[
                ("\"name\"", Keyword),
                ("\"sEdit\"", Str),
                ("\"n\"", Keyword),
                ("1.5e3", Number),
                ("\"ok\"", Keyword),
                ("true", Number),
                ("\"x\"", Keyword),
                ("null", Number),
            ],
        );
        expect(json("[\"a:b\", \"c\"]"), &[("\"a:b\"", Str), ("\"c\"", Str)]);
    }

    #[test]
    fn json_allows_comments() {
        expect(
            json("// jsonc\n{ /* x */ \"a\": 1 }"),
            &[("// jsonc", Comment), ("/* x */", Comment), ("\"a\"", Keyword), ("1", Number)],
        );
    }

    #[test]
    fn html_tags_attributes_and_text() {
        expect(
            html("<!DOCTYPE html>\n<a href=\"x.html\" class=big hidden>Tom &amp; Jerry</a><br/>"),
            &[
                ("<!DOCTYPE html>", Keyword),
                ("a", Keyword),
                ("href", Type),
                ("\"x.html\"", Str),
                ("class", Type),
                ("big", Str),
                ("hidden", Type),
                ("&amp;", Number),
                ("a", Keyword),
                ("br", Keyword),
            ],
        );
        expect(html("fisk & chips, a < b"), &[]);
    }

    #[test]
    fn html_comments_and_values_span_lines() {
        expect(
            html("<!-- <b>ikke\net tag</b> -->\n<p title='to\nlinjer'>"),
            &[("<!-- <b>ikke\net tag</b> -->", Comment), ("p", Keyword), ("title", Type), ("'to\nlinjer'", Str)],
        );
        expect(html("<!-- aldrig lukket"), &[("<!-- aldrig lukket", Comment)]);
    }

    #[test]
    fn html_colors_script_and_style_in_their_own_language() {
        expect(
            html("<script>let a = '</p>';</SCRIPT><style>p { color: red }</style><p>let</p>"),
            &[
                ("script", Keyword),
                ("let", Keyword),
                ("'</p>'", Str),
                ("SCRIPT", Keyword),
                ("style", Keyword),
                ("p", Keyword),
                ("color", Keyword),
                ("style", Keyword),
                ("p", Keyword),
                ("p", Keyword),
            ],
        );
        // Et tag der lukker sig selv har intet indhold.
        expect(html("<script src=a.js /><p>let</p>"), &[("script", Keyword), ("src", Type), ("a.js", Str), ("p", Keyword), ("p", Keyword)]);
    }

    #[test]
    fn css_selectors_properties_and_values() {
        expect(
            css("a.btn:hover, #top > li[data-x=\"1\"] {\n  color: #fff;\n  margin: -1.5em 10% 0 auto !important;\n}"),
            &[
                ("a", Keyword),
                (".btn", Function),
                (":hover", Type),
                ("#top", Function),
                ("li", Keyword),
                ("data-x", Type),
                ("\"1\"", Str),
                ("color", Keyword),
                ("#fff", Number),
                ("margin", Keyword),
                ("-1.5em", Number),
                ("10%", Number),
                ("0", Number),
                ("!important", Keyword),
            ],
        );
    }

    #[test]
    fn css_functions_urls_and_comments() {
        expect(
            css("/* note */ p { background: url(a;b.png) rgb(0, 0, 0); --gap: calc(1px + var(--x)) }"),
            &[
                ("/* note */", Comment),
                ("p", Keyword),
                ("background", Keyword),
                ("url", Function),
                ("a;b.png", Str),
                ("rgb", Function),
                ("0", Number),
                ("0", Number),
                ("0", Number),
                ("--gap", Keyword),
                ("calc", Function),
                ("1px", Number),
                ("var", Function),
            ],
        );
    }

    #[test]
    fn css_at_rules_and_nested_rules() {
        expect(
            css("@media (min-width: 600px) { .a { top: 0 } }\n@import 'x.css';"),
            &[
                ("@media", Keyword),
                ("600px", Number),
                (".a", Function),
                ("top", Keyword),
                ("0", Number),
                ("@import", Keyword),
                ("'x.css'", Str),
            ],
        );
        // Indlejrede regler: "a:hover {" er en regel, "color: red" en erklæring.
        expect(
            css(".card { color: red; a:hover { top: 1px } }"),
            &[
                (".card", Function),
                ("color", Keyword),
                ("a", Keyword),
                (":hover", Type),
                ("top", Keyword),
                ("1px", Number),
            ],
        );
    }

    fn vue(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Vue, source)
    }

    fn svelte(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Svelte, source)
    }

    fn scss(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Scss, source)
    }

    #[test]
    fn vue_svelte_and_scss_follow_the_file_extension() {
        assert_eq!(Lang::of(Path::new("App.vue")), Some(Lang::Vue));
        assert_eq!(Lang::of(Path::new("+page.svelte")), Some(Lang::Svelte));
        assert_eq!(Lang::of(Path::new("main.scss")), Some(Lang::Scss));
        assert_eq!(Lang::of(Path::new("theme.less")), Some(Lang::Scss));
    }

    #[test]
    fn scss_variables_comments_and_nesting() {
        expect(
            scss("// note\n$gap: 8px;\n@mixin card($r: 4px) { border-radius: $r; }\n.btn {\n  &:hover { color: darken($c, 10%); }\n  @include card;\n  .icon-#{$name}, %base { width: calc(#{$gap} * 2); }\n}"),
            &[
                ("// note", Comment),
                ("$gap", Type),
                ("8px", Number),
                ("@mixin", Keyword),
                ("card", Function),
                ("$r", Type),
                ("4px", Number),
                ("border-radius", Keyword),
                ("$r", Type),
                (".btn", Function),
                (":hover", Type),
                ("color", Keyword),
                ("darken", Function),
                ("$c", Type),
                ("10%", Number),
                ("@include", Keyword),
                (".icon-", Function),
                ("#{", Keyword),
                ("$name", Type),
                ("}", Keyword),
                ("%base", Function),
                ("width", Keyword),
                ("calc", Function),
                ("#{", Keyword),
                ("$gap", Type),
                ("}", Keyword),
                ("2", Number),
            ],
        );
        // I ren CSS er // og $ ikke noget særligt.
        expect(css("a { b: url(//x.dk/y) } // c { d: 1 }"), &[("a", Keyword), ("b", Keyword), ("url", Function), ("//x.dk/y", Str), ("c", Keyword), ("d", Keyword), ("1", Number)]);
    }

    #[test]
    fn vue_template_script_and_style() {
        expect(
            vue("<script setup lang=\"ts\">\nconst n: number = 1\n</script>\n<template>\n  <OrderList :items=\"orders.filter(o => o.paid)\" @click=\"open('x')\" class=\"big\" v-if=\"n > 0\">{{ total + 1 }}</OrderList>\n</template>\n<style lang=\"scss\" scoped>\n$c: red; // x\n</style>"),
            &[
                ("script", Keyword),
                ("setup", Type),
                ("lang", Type),
                ("\"ts\"", Str),
                ("const", Keyword),
                ("number", Type),
                ("1", Number),
                ("script", Keyword),
                ("template", Keyword),
                ("OrderList", Type),
                (":items", Type),
                ("filter", Function),
                ("@click", Type),
                ("open", Function),
                ("'x'", Str),
                ("class", Type),
                ("\"big\"", Str),
                ("v-if", Type),
                ("0", Number),
                ("{{", Keyword),
                ("1", Number),
                ("}}", Keyword),
                ("OrderList", Type),
                ("template", Keyword),
                ("style", Keyword),
                ("lang", Type),
                ("\"scss\"", Str),
                ("scoped", Type),
                ("$c", Type),
                ("// x", Comment),
                ("style", Keyword),
            ],
        );
    }

    #[test]
    fn svelte_blocks_and_expressions() {
        expect(
            svelte("<script>\n  let count = 0;\n</script>\n{#if count > 1}\n  <Button on:click={() => count++} disabled={!ok} {title}>Antal: {count + '}'}</Button>\n{:else}\n  <p class=\"a\">{@html text}</p>\n{/if}\n<style>p { color: red }</style>"),
            &[
                ("script", Keyword),
                ("let", Keyword),
                ("0", Number),
                ("script", Keyword),
                ("{#if", Keyword),
                ("1", Number),
                ("}", Keyword),
                ("Button", Type),
                ("on:click", Type),
                ("{", Keyword),
                ("}", Keyword),
                ("disabled", Type),
                ("{", Keyword),
                ("}", Keyword),
                ("{", Keyword),
                ("}", Keyword),
                ("{", Keyword),
                ("'}'", Str),
                ("}", Keyword),
                ("Button", Type),
                ("{:else", Keyword),
                ("}", Keyword),
                ("p", Keyword),
                ("class", Type),
                ("\"a\"", Str),
                ("{@html", Keyword),
                ("}", Keyword),
                ("p", Keyword),
                ("{/if", Keyword),
                ("}", Keyword),
                ("style", Keyword),
                ("p", Keyword),
                ("color", Keyword),
                ("style", Keyword),
            ],
        );
        // En { uden makker er bare tekst, og i HTML er {x} også bare tekst.
        expect(svelte("a { b <i>"), &[("i", Keyword)]);
        expect(html("<p>{x} {{ y }}</p>"), &[("p", Keyword), ("p", Keyword)]);
    }

    fn env(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Env, source)
    }

    #[test]
    fn env_keys_values_and_comments() {
        expect(
            env("# database\nDATABASE_URL=postgres://user:pw@host:5432/db?ssl=true\nPORT=8080\nDEBUG = false # kun lokalt\nSECRET=abc#123==\nEMPTY=\n\nexport NODE_ENV=production\nløs linje"),
            &[
                ("# database", Comment),
                ("DATABASE_URL", Keyword),
                ("postgres://user:pw@host:5432/db?ssl=true", Str),
                ("PORT", Keyword),
                ("8080", Number),
                ("DEBUG", Keyword),
                ("false", Number),
                ("# kun lokalt", Comment),
                ("SECRET", Keyword),
                ("abc#123==", Str),
                ("EMPTY", Keyword),
                ("export", Keyword),
                ("NODE_ENV", Keyword),
                ("production", Str),
            ],
        );
    }

    #[test]
    fn env_quotes_and_variables() {
        expect(
            env("A=\"hej $NAME # ikke kommentar\" # men her\nB='rå $x'\nC=${HOME}/bin:$PATH\nD=\"to\nlinjer\"\nE=1"),
            &[
                ("A", Keyword),
                ("\"hej ", Str),
                ("$NAME", Type),
                (" # ikke kommentar\"", Str),
                ("# men her", Comment),
                ("B", Keyword),
                ("'rå $x'", Str),
                ("C", Keyword),
                ("${HOME}", Type),
                ("/bin:", Str),
                ("$PATH", Type),
                ("D", Keyword),
                ("\"to\nlinjer\"", Str),
                ("E", Keyword),
                ("1", Number),
            ],
        );
    }

    fn docker(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Dockerfile, source)
    }

    fn ini(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Ini, source)
    }

    fn bat(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Batch, source)
    }

    #[test]
    fn dockerfile_ini_and_batch_follow_the_file_name() {
        assert_eq!(Lang::of(Path::new("app/Dockerfile")), Some(Lang::Dockerfile));
        assert_eq!(Lang::of(Path::new("Dockerfile.dev")), Some(Lang::Dockerfile));
        assert_eq!(Lang::of(Path::new("api.dockerfile")), Some(Lang::Dockerfile));
        assert_eq!(Lang::of(Path::new("settings.ini")), Some(Lang::Ini));
        assert_eq!(Lang::of(Path::new(".editorconfig")), Some(Lang::Ini));
        assert_eq!(Lang::of(Path::new("build.BAT")), Some(Lang::Batch));
        assert_eq!(Lang::of(Path::new("run.cmd")), Some(Lang::Batch));
        // .env-filer har deres egne regler.
        assert_eq!(Lang::of(Path::new(".env")), Some(Lang::Env));
        assert_eq!(Lang::of(Path::new(".env.local")), Some(Lang::Env));
        assert_eq!(Lang::of(Path::new("prod.env")), Some(Lang::Env));
        assert_eq!(Lang::of(Path::new(".envrc")), None);
    }

    #[test]
    fn dockerfile_instructions_and_values() {
        expect(
            docker("# byg\nFROM node:20 AS build\nARG VERSION=1\nENV PORT=8080 NAME=\"app $VERSION\"\nexpose 8080 443\nCOPY --from=build /app ${DEST}/\nONBUILD CMD [\"node\", 'x.js']\nfrom_x y"),
            &[
                ("# byg", Comment),
                ("FROM", Keyword),
                ("AS", Keyword),
                ("ARG", Keyword),
                ("ENV", Keyword),
                ("\"app ", Str),
                ("$VERSION", Type),
                ("\"", Str),
                ("expose", Keyword),
                ("8080", Number),
                ("443", Number),
                ("COPY", Keyword),
                ("${DEST}", Type),
                ("ONBUILD", Keyword),
                ("CMD", Keyword),
                ("\"node\"", Str),
                ("'x.js'", Str),
            ],
        );
    }

    #[test]
    fn dockerfile_run_is_shell_code_across_continued_lines() {
        expect(
            docker("RUN apt-get update && \\\n    # ryd op\n    rm -rf \"$TMP\" \\\n    && echo done\nWORKDIR /app\nRUN if true; then ls; fi"),
            &[
                ("RUN", Keyword),
                ("apt-get", Function),
                ("# ryd op", Comment),
                ("rm", Function),
                ("\"", Str),
                ("$TMP", Type),
                ("\"", Str),
                ("echo", Function),
                ("WORKDIR", Keyword),
                ("RUN", Keyword),
                ("if", Keyword),
                ("true", Function),
                ("then", Keyword),
                ("ls", Function),
                ("fi", Keyword),
            ],
        );
    }

    #[test]
    fn ini_sections_keys_and_values() {
        expect(
            ini("; kommentar\n# også\n[core]\n  editor = code --wait\nport=8080\nenabled = yes\npath = C:\\x;y # z\nempty =\nbare linje\n[remote \"origin\"] ekstra"),
            &[
                ("; kommentar", Comment),
                ("# også", Comment),
                ("[core]", Function),
                ("editor", Keyword),
                ("code --wait", Str),
                ("port", Keyword),
                ("8080", Number),
                ("enabled", Keyword),
                ("yes", Number),
                ("path", Keyword),
                ("C:\\x;y # z", Str),
                ("empty", Keyword),
                ("[remote \"origin\"]", Function),
            ],
        );
    }

    #[test]
    fn batch_commands_keywords_and_variables() {
        expect(
            bat("@echo off\nREM note if\n:: også note\nsetlocal\nset NAME=world if 1\nif not exist \"%~dp0out\\%NAME%\" mkdir out\nfor %%f in (*.txt) do copy %%f !DEST! >nul\n:done\ngoto :eof"),
            &[
                ("echo", Function),
                ("REM note if", Comment),
                (":: også note", Comment),
                ("setlocal", Keyword),
                ("set", Keyword),
                ("if", Keyword),
                ("not", Keyword),
                ("exist", Keyword),
                ("\"", Str),
                ("%~dp0", Type),
                ("out\\", Str),
                ("%NAME%", Type),
                ("\"", Str),
                ("mkdir", Function),
                ("for", Keyword),
                ("%%f", Type),
                ("in", Keyword),
                ("do", Keyword),
                ("copy", Function),
                ("%%f", Type),
                ("!DEST!", Type),
                (":done", Function),
                ("goto", Keyword),
                (":eof", Function),
            ],
        );
    }

    #[test]
    fn batch_echo_text_is_not_code() {
        expect(
            bat("echo if you do 100% of %COUNT% & call :sub 5\nmyapp.exe --port 80 | findstr x\necho. & echo(done"),
            &[
                ("echo", Function),
                ("%COUNT%", Type),
                ("call", Keyword),
                (":sub", Function),
                ("5", Number),
                ("myapp.exe", Function),
                ("80", Number),
                ("findstr", Function),
                ("echo.", Function),
                ("echo", Function),
            ],
        );
    }

    fn ps(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::PowerShell, source)
    }

    fn lua(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Lua, source)
    }

    fn toml(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Toml, source)
    }

    #[test]
    fn powershell_lua_and_toml_follow_the_file_name() {
        assert_eq!(Lang::of(Path::new("install.ps1")), Some(Lang::PowerShell));
        assert_eq!(Lang::of(Path::new("Tools.PSM1")), Some(Lang::PowerShell));
        assert_eq!(Lang::of(Path::new("init.lua")), Some(Lang::Lua));
        assert_eq!(Lang::of(Path::new("Cargo.toml")), Some(Lang::Toml));
        assert_eq!(Lang::of(Path::new("Cargo.lock")), Some(Lang::Toml));
        assert_eq!(Lang::of(Path::new("yarn.lock")), None);
    }

    #[test]
    fn powershell_commands_variables_and_operators() {
        expect(
            ps("$files = Get-ChildItem -Path $env:TEMP -Recurse | Where-Object { $_.Length -gt 1mb -and $true }\nif (-not $files) { Write-Host 'ingen' } # slut"),
            &[
                ("$files", Type),
                ("Get-ChildItem", Function),
                ("$env:TEMP", Type),
                ("Where-Object", Function),
                ("$_", Type),
                ("-gt", Keyword),
                ("1mb", Number),
                ("-and", Keyword),
                ("$true", Number),
                ("if", Keyword),
                ("-not", Keyword),
                ("$files", Type),
                ("Write-Host", Function),
                ("'ingen'", Str),
                ("# slut", Comment),
            ],
        );
    }

    #[test]
    fn powershell_functions_types_and_members() {
        expect(
            ps("Function Get-Total([int]$n, [System.IO.FileInfo[]]$f) {\n  [Parameter(Mandatory)]\n  return [math]::Round($n * 1.25, 2) + $list[0].ToString().Length\n}\nclass Order { }"),
            &[
                ("Function", Keyword),
                ("Get-Total", Function),
                ("int", Type),
                ("$n", Type),
                ("System.IO.FileInfo", Type),
                ("$f", Type),
                ("Parameter", Type),
                ("return", Keyword),
                ("math", Type),
                ("Round", Function),
                ("$n", Type),
                ("1.25", Number),
                ("2", Number),
                ("$list", Type),
                ("0", Number),
                ("ToString", Function),
                ("class", Keyword),
                ("Order", Type),
            ],
        );
    }

    #[test]
    fn powershell_strings_and_comments() {
        expect(
            ps("<# blok\nif #>\n$a = \"hej $name`\" og \"\"x\"\" $(Get-Date)\" + 'it''s $no'\n$b = @'\nrå 'tekst' $x\n'@\nfoo#bar"),
            &[
                ("<# blok\nif #>", Comment),
                ("$a", Type),
                ("\"hej ", Str),
                ("$name", Type),
                ("`\" og \"\"x\"\" $(Get-Date)\"", Str),
                ("'it''s $no'", Str),
                ("$b", Type),
                ("@'\nrå 'tekst' $x\n'@", Str),
                ("foo", Function),
            ],
        );
    }

    #[test]
    fn lua_keywords_calls_and_literals() {
        expect(
            lua("local function total(lines, rate)\n  if not lines then return nil end\n  return obj:sum(lines) * (1 + rate) .. 'kr' -- moms\nend"),
            &[
                ("local", Keyword),
                ("function", Keyword),
                ("total", Function),
                ("if", Keyword),
                ("not", Keyword),
                ("then", Keyword),
                ("return", Keyword),
                ("nil", Number),
                ("end", Keyword),
                ("return", Keyword),
                ("sum", Function),
                ("1", Number),
                ("'kr'", Str),
                ("-- moms", Comment),
                ("end", Keyword),
            ],
        );
        expect(lua("t.end = M.new(0x1F, true)"), &[("M", Type), ("new", Function), ("0x1F", Number), ("true", Number)]);
    }

    #[test]
    fn lua_long_strings_and_comments() {
        expect(
            lua("--[[ blok\nif ]] x = [==[ rå ]] stadig\n]==] --[ ikke blok\ny = t[1]"),
            &[
                ("--[[ blok\nif ]]", Comment),
                ("[==[ rå ]] stadig\n]==]", Str),
                ("--[ ikke blok", Comment),
                ("1", Number),
            ],
        );
        expect(lua("s = [[aldrig lukket"), &[("[[aldrig lukket", Str)]);
    }

    #[test]
    fn toml_tables_keys_and_values() {
        expect(
            toml("# pakke\n[package]\nname = \"sedit\" # navn\nversion = '0.1.0'\nedition = 2021\ndebug = true\nbuilt = 2026-11-25T10:00:00Z\n\"quoted key\" = -1.5\n[[bin]]\npath.to = inf"),
            &[
                ("# pakke", Comment),
                ("[package]", Function),
                ("name", Keyword),
                ("\"sedit\"", Str),
                ("# navn", Comment),
                ("version", Keyword),
                ("'0.1.0'", Str),
                ("edition", Keyword),
                ("2021", Number),
                ("debug", Keyword),
                ("true", Number),
                ("built", Keyword),
                ("2026-11-25T10:00:00Z", Number),
                ("\"quoted key\"", Keyword),
                ("-1.5", Number),
                ("[[bin]]", Function),
                ("path.to", Keyword),
                ("inf", Number),
            ],
        );
    }

    #[test]
    fn toml_arrays_inline_tables_and_long_strings() {
        expect(
            toml("features = [\n  \"a\", # første\n  1,\n]\ndep = { version = \"1\", optional = false }\ntext = \"\"\"\nto \"linjer\"\nx = 1\"\"\"\nlast = 2"),
            &[
                ("features", Keyword),
                ("\"a\"", Str),
                ("# første", Comment),
                ("1", Number),
                ("dep", Keyword),
                ("version", Keyword),
                ("\"1\"", Str),
                ("optional", Keyword),
                ("false", Number),
                ("text", Keyword),
                ("\"\"\"\nto \"linjer\"\nx = 1\"\"\"", Str),
                ("last", Keyword),
                ("2", Number),
            ],
        );
    }

    fn kt(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Kotlin, source)
    }

    fn swift(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Swift, source)
    }

    fn rb(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Ruby, source)
    }

    #[test]
    fn kotlin_swift_and_ruby_follow_the_file_name() {
        assert_eq!(Lang::of(Path::new("Main.kt")), Some(Lang::Kotlin));
        assert_eq!(Lang::of(Path::new("build.gradle.kts")), Some(Lang::Kotlin));
        assert_eq!(Lang::of(Path::new("App.swift")), Some(Lang::Swift));
        assert_eq!(Lang::of(Path::new("user.rb")), Some(Lang::Ruby));
        assert_eq!(Lang::of(Path::new("app/Gemfile")), Some(Lang::Ruby));
    }

    #[test]
    fn kotlin_keywords_types_and_calls() {
        expect(
            kt("@Serializable\ndata class Order(val id: Int, private val lines: List<Double> = emptyList()) {\n  fun total(): Double = lines.sum() * 1.25 // moms\n}"),
            &[
                ("@Serializable", Function),
                ("data", Keyword),
                ("class", Keyword),
                ("Order", Type),
                ("val", Keyword),
                ("Int", Type),
                ("private", Keyword),
                ("val", Keyword),
                ("List", Type),
                ("Double", Type),
                ("emptyList", Function),
                ("fun", Keyword),
                ("total", Function),
                ("Double", Type),
                ("sum", Function),
                ("1.25", Number),
                ("// moms", Comment),
            ],
        );
        // "data" og "value" er kun nøgleord, når de ikke kaldes eller står efter et punktum.
        expect(kt("val x = data(1) + it.value ?: null"), &[("val", Keyword), ("data", Function), ("1", Number), ("null", Number)]);
    }

    #[test]
    fn kotlin_strings_and_nested_comments() {
        expect(
            kt("/* a /* b */ c */ val s = \"x $y\" + 'c' + \"\"\"\n  rå \"tekst\"\n\"\"\""),
            &[
                ("/* a /* b */ c */", Comment),
                ("val", Keyword),
                ("\"x $y\"", Str),
                ("'c'", Str),
                ("\"\"\"\n  rå \"tekst\"\n\"\"\"", Str),
            ],
        );
    }

    #[test]
    fn swift_keywords_types_and_attributes() {
        expect(
            swift("@MainActor\nfinal class Store: ObservableObject {\n  @Published var items: [Item] = []\n  func load(id: Int) async throws -> Item? { guard id > 0 else { return nil }; return try await api.fetch(id) }\n}"),
            &[
                ("@MainActor", Function),
                ("final", Keyword),
                ("class", Keyword),
                ("Store", Type),
                ("ObservableObject", Type),
                ("@Published", Function),
                ("var", Keyword),
                ("Item", Type),
                ("func", Keyword),
                ("load", Function),
                ("Int", Type),
                ("async", Keyword),
                ("throws", Keyword),
                ("Item", Type),
                ("guard", Keyword),
                ("0", Number),
                ("else", Keyword),
                ("return", Keyword),
                ("nil", Number),
                ("return", Keyword),
                ("try", Keyword),
                ("await", Keyword),
                ("fetch", Function),
            ],
        );
    }

    #[test]
    fn swift_strings_and_directives() {
        expect(
            swift("#if DEBUG\nlet s = \"hej \\(name)\" + \"\"\"\n  to 'linjer'\n  \"\"\"\n#endif\nlet c = it's"),
            &[
                ("#if", Keyword),
                ("let", Keyword),
                ("\"hej \\(name)\"", Str),
                ("\"\"\"\n  to 'linjer'\n  \"\"\"", Str),
                ("#endif", Keyword),
                ("let", Keyword),
            ],
        );
    }

    #[test]
    fn ruby_keywords_methods_and_symbols() {
        expect(
            rb("class Order < Base\n  attr_reader :lines\n  def total(rate = 0.25)\n    @lines.sum * (1 + rate) unless empty?\n  end\n  def self.find(id) = Repo.get(id, :strict)\nend"),
            &[
                ("class", Keyword),
                ("Order", Type),
                ("Base", Type),
                ("attr_reader", Keyword),
                (":lines", Number),
                ("def", Keyword),
                ("total", Function),
                ("0.25", Number),
                ("@lines", Type),
                ("1", Number),
                ("unless", Keyword),
                ("end", Keyword),
                ("def", Keyword),
                ("self", Keyword),
                ("find", Function),
                ("Repo", Type),
                ("get", Function),
                (":strict", Number),
                ("end", Keyword),
            ],
        );
        expect(rb("x ? Foo::Bar : y; h = { a: 1 }; $stdout.puts(nil)"), &[("Foo", Type), ("Bar", Type), ("1", Number), ("$stdout", Type), ("puts", Function), ("nil", Number)]);
    }

    #[test]
    fn ruby_strings_regex_and_comments() {
        expect(
            rb("# note\ns = \"a #{h[\"k\"]} b\" + 'it\\'s' + %w[x y]\nif s =~ /\\d+/i then a / b end\n=begin\nskjult if\n=end\nputs 1"),
            &[
                ("# note", Comment),
                ("\"a #{h[\"k\"]} b\"", Str),
                ("'it\\'s'", Str),
                ("%w[x y]", Str),
                ("if", Keyword),
                ("/\\d+/i", Str),
                ("then", Keyword),
                ("end", Keyword),
                ("=begin\nskjult if\n=end", Comment),
                ("1", Number),
            ],
        );
    }

    #[test]
    fn ruby_heredoc() {
        expect(
            rb("sql = <<~SQL\n  select 'x'\n  if\nSQL\nx << y\nend"),
            &[("  select 'x'\n  if\nSQL", Str), ("end", Keyword)],
        );
    }

    fn c(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::C, source)
    }

    fn cpp(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Cpp, source)
    }

    fn sh(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Bash, source)
    }

    #[test]
    fn c_cpp_and_bash_follow_the_file_name() {
        assert_eq!(Lang::of(Path::new("main.c")), Some(Lang::C));
        assert_eq!(Lang::of(Path::new("util.H")), Some(Lang::C));
        assert_eq!(Lang::of(Path::new("app.cpp")), Some(Lang::Cpp));
        assert_eq!(Lang::of(Path::new("app.hpp")), Some(Lang::Cpp));
        assert_eq!(Lang::of(Path::new("deploy.sh")), Some(Lang::Bash));
        assert_eq!(Lang::of(Path::new("home/.bashrc")), Some(Lang::Bash));
        assert_eq!(Lang::of(Path::new(".env")), Some(Lang::Env));
    }

    #[test]
    fn c_directives_types_and_calls() {
        expect(
            c("#include <stdio.h>\n#  define MAX 10\nstatic size_t count(const struct node *n) { return n->next ? strlen(n->name) : NULL; }"),
            &[
                ("#include", Keyword),
                ("<stdio.h>", Str),
                ("#  define", Keyword),
                ("10", Number),
                ("static", Keyword),
                ("size_t", Type),
                ("count", Function),
                ("const", Keyword),
                ("struct", Keyword),
                ("node", Type),
                ("return", Keyword),
                ("strlen", Function),
                ("NULL", Number),
            ],
        );
        // I C er "class" og "new" almindelige navne.
        expect(c("int class = new + 1; // x"), &[("int", Type), ("1", Number), ("// x", Comment)]);
    }

    #[test]
    fn c_strings_and_comments() {
        expect(
            c("char *s = \"a\\\"b\"; wchar_t w = L'x'; /* to\nlinjer */ x = 0x1FUL + 1.5f;"),
            &[
                ("char", Type),
                ("\"a\\\"b\"", Str),
                ("wchar_t", Type),
                ("L'x'", Str),
                ("/* to\nlinjer */", Comment),
                ("0x1FUL", Number),
                ("1.5f", Number),
            ],
        );
    }

    #[test]
    fn cpp_adds_its_own_keywords_and_raw_strings() {
        expect(
            cpp("template <typename T> class Box : public Base { auto get() const -> T { return new Item(nullptr); } };"),
            &[
                ("template", Keyword),
                ("typename", Keyword),
                ("T", Type),
                ("class", Keyword),
                ("Box", Type),
                ("public", Keyword),
                ("Base", Type),
                ("auto", Keyword),
                ("get", Function),
                ("const", Keyword),
                ("T", Type),
                ("return", Keyword),
                ("new", Keyword),
                ("Item", Type),
                ("nullptr", Number),
            ],
        );
        expect(
            cpp("auto s = R\"x(rå \"tekst\"\n)\" stadig)x\" + u8\"y\"; std::vector<int> v; v.size();"),
            &[
                ("auto", Keyword),
                ("R\"x(rå \"tekst\"\n)\" stadig)x\"", Str),
                ("u8\"y\"", Str),
                ("vector", Type),
                ("int", Type),
                ("size", Function),
            ],
        );
    }

    #[test]
    fn cpp_digit_separators_are_part_of_the_number() {
        expect(cpp("x = 1'000'000 + 'a';"), &[("1'000'000", Number), ("'a'", Str)]);
        // I C er det ikke et tal med skilletegn.
        expect(c("x = 1'0';"), &[("1", Number), ("'0'", Str)]);
    }

    #[test]
    fn bash_commands_keywords_and_variables() {
        expect(
            sh("#!/bin/bash\nNAME=world\nif [ -f \"$HOME/x\" ]; then\n  echo hello $NAME | grep -c 3 # tæl\nfi"),
            &[
                ("#!/bin/bash", Comment),
                ("if", Keyword),
                ("\"", Str),
                ("$HOME", Type),
                ("/x\"", Str),
                ("then", Keyword),
                ("echo", Function),
                ("$NAME", Type),
                ("grep", Function),
                ("3", Number),
                ("# tæl", Comment),
                ("fi", Keyword),
            ],
        );
        expect(
            sh("for f in *.txt; do cp \"$f\" ${dst:-/tmp}/; done; echo if $# a#b"),
            &[
                ("for", Keyword),
                ("in", Keyword),
                ("do", Keyword),
                ("cp", Function),
                ("\"", Str),
                ("$f", Type),
                ("\"", Str),
                ("${dst:-/tmp}", Type),
                ("done", Keyword),
                ("echo", Function),
                ("$#", Type),
            ],
        );
    }

    #[test]
    fn bash_functions_strings_and_heredocs() {
        expect(
            sh("function build() { local x='it $no'; }\ndeploy() {\n  cat <<EOF\nhej $USER\n  if 'x'\nEOF\n  out=$(date +%s)\n}"),
            &[
                ("function", Keyword),
                ("build", Function),
                ("local", Keyword),
                ("'it $no'", Str),
                ("deploy", Function),
                ("cat", Function),
                ("hej $USER\n  if 'x'\nEOF", Str),
                ("date", Function),
            ],
        );
        expect(sh("echo \"a $(ls \"x\") b\" 'åben"), &[("echo", Function), ("\"a $(ls \"x\") b\"", Str), ("'åben", Str)]);
    }

    fn go(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Go, source)
    }

    fn java(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Java, source)
    }

    fn xml(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Xml, source)
    }

    #[test]
    fn go_java_and_xml_follow_the_file_extension() {
        assert_eq!(Lang::of(Path::new("main.go")), Some(Lang::Go));
        assert_eq!(Lang::of(Path::new("Order.java")), Some(Lang::Java));
        assert_eq!(Lang::of(Path::new("pom.xml")), Some(Lang::Xml));
        assert_eq!(Lang::of(Path::new("icon.SVG")), Some(Lang::Xml));
        assert_eq!(Lang::of(Path::new("App.csproj")), Some(Lang::Xml));
    }

    #[test]
    fn go_keywords_types_and_calls() {
        expect(
            go("func (s *Server) Handle(w http.ResponseWriter, n int) error {\n\tfmt.Println(\"hej\", len(s.items), nil)\n\treturn Order{ID: 1}.Err\n}"),
            &[
                ("func", Keyword),
                ("Server", Type),
                ("Handle", Function),
                ("int", Type),
                ("error", Type),
                ("Println", Function),
                ("\"hej\"", Str),
                ("len", Function),
                ("nil", Number),
                ("return", Keyword),
                ("Order", Type),
                ("1", Number),
            ],
        );
        expect(
            go("type order struct{}\nfunc Map[T any](x T) {}\nvar x = string(b)"),
            &[
                ("type", Keyword),
                ("order", Type),
                ("struct", Keyword),
                ("func", Keyword),
                ("Map", Function),
                ("T", Type),
                ("any", Type),
                ("T", Type),
                ("var", Keyword),
                ("string", Type),
            ],
        );
    }

    #[test]
    fn go_strings_and_comments() {
        expect(
            go("a := `rå \"streng\"\nto linjer` + \"x\\\"y\" + 'r' // slut\n/* blok */ 0x1F"),
            &[
                ("`rå \"streng\"\nto linjer`", Str),
                ("\"x\\\"y\"", Str),
                ("'r'", Str),
                ("// slut", Comment),
                ("/* blok */", Comment),
                ("0x1F", Number),
            ],
        );
        expect(go("`aldrig lukket"), &[("`aldrig lukket", Str)]);
    }

    #[test]
    fn java_keywords_types_and_calls() {
        expect(
            java("@Override\npublic List<Order> find(int id) throws IOException { var x = new Order(id); return repo.findAll(MAX, 1.5f, null); }"),
            &[
                ("@Override", Function),
                ("public", Keyword),
                ("List", Type),
                ("Order", Type),
                ("find", Function),
                ("int", Type),
                ("throws", Keyword),
                ("IOException", Type),
                ("var", Keyword),
                ("new", Keyword),
                ("Order", Type),
                ("return", Keyword),
                ("findAll", Function),
                ("1.5f", Number),
                ("null", Number),
            ],
        );
        expect(
            java("public @interface Marker {} record Point(int x) {} System.out.println(obj.record());"),
            &[
                ("public", Keyword),
                ("@interface", Keyword),
                ("Marker", Type),
                ("record", Keyword),
                ("Point", Type),
                ("int", Type),
                ("System", Type),
                ("println", Function),
                ("record", Function),
            ],
        );
    }

    #[test]
    fn java_strings_and_text_blocks() {
        expect(
            java("String s = \"a\\\"b\" + 'c' + \"\"\"\n  to \"linjer\"\n  \"\"\"; // slut"),
            &[("String", Type), ("\"a\\\"b\"", Str), ("'c'", Str), ("\"\"\"\n  to \"linjer\"\n  \"\"\"", Str), ("// slut", Comment)],
        );
    }

    #[test]
    fn xml_tags_attributes_and_cdata() {
        expect(
            xml("<?xml version=\"1.0\"?>\n<!-- note -->\n<ns:item id=\"1\" xml:lang='da'>Tom &amp; Jerry<![CDATA[<b> & ]]></ns:item><br/>"),
            &[
                ("<?xml version=\"1.0\"?>", Keyword),
                ("<!-- note -->", Comment),
                ("ns:item", Keyword),
                ("id", Type),
                ("\"1\"", Str),
                ("xml:lang", Type),
                ("'da'", Str),
                ("&amp;", Number),
                ("<![CDATA[<b> & ]]>", Str),
                ("ns:item", Keyword),
                ("br", Keyword),
            ],
        );
        // I XML er <script> og <style> almindelige tags.
        expect(xml("<style>let a { b: c }</style>"), &[("style", Keyword), ("style", Keyword)]);
    }

    fn cs(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::CSharp, source)
    }

    fn php(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Php, source)
    }

    fn yaml(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Yaml, source)
    }

    #[test]
    fn csharp_php_and_yaml_follow_the_file_extension() {
        assert_eq!(Lang::of(Path::new("Program.cs")), Some(Lang::CSharp));
        assert_eq!(Lang::of(Path::new("index.php")), Some(Lang::Php));
        assert_eq!(Lang::of(Path::new("docker-compose.yml")), Some(Lang::Yaml));
        assert_eq!(Lang::of(Path::new("ci.YAML")), Some(Lang::Yaml));
    }

    #[test]
    fn csharp_keywords_types_and_calls() {
        expect(
            cs("public async Task<int> CountAsync(string name) { var n = await db.Orders.CountAsync(); return n > 0 ? n : MaxValue; }"),
            &[
                ("public", Keyword),
                ("async", Keyword),
                ("Task", Type),
                ("int", Type),
                ("CountAsync", Function),
                ("string", Type),
                ("var", Keyword),
                ("await", Keyword),
                ("CountAsync", Function),
                ("return", Keyword),
                ("0", Number),
                ("MaxValue", Type),
            ],
        );
        // "new X(...)" er en type, og "Get(...)" er et kald selv om "get" er et nøgleord.
        expect(
            cs("var o = new Order(1); int Total { get; set; } x.get(2); null"),
            &[
                ("var", Keyword),
                ("new", Keyword),
                ("Order", Type),
                ("1", Number),
                ("int", Type),
                ("Total", Type),
                ("get", Keyword),
                ("set", Keyword),
                ("get", Function),
                ("2", Number),
                ("null", Number),
            ],
        );
    }

    #[test]
    fn csharp_strings_and_directives() {
        expect(
            cs("a = \"x\\\"y\" + $\"{n} stk\" + @\"C:\\dir\\\" + @\"to\nlinjer \"\"med\"\" det\" + 'c' + @class;"),
            &[
                ("\"x\\\"y\"", Str),
                ("$\"{n} stk\"", Str),
                ("@\"C:\\dir\\\"", Str),
                ("@\"to\nlinjer \"\"med\"\" det\"", Str),
                ("'c'", Str),
                ("class", Keyword),
            ],
        );
        expect(
            cs("#if DEBUG\nvar s = \"\"\"\n  rå \"tekst\"\n  \"\"\"; // slut\n#endif"),
            &[
                ("#if", Keyword),
                ("var", Keyword),
                ("\"\"\"\n  rå \"tekst\"\n  \"\"\"", Str),
                ("// slut", Comment),
                ("#endif", Keyword),
            ],
        );
    }

    #[test]
    fn php_is_html_with_code_between_the_tags() {
        expect(
            php("<p class=\"a\"><?php echo strtoupper($name) . 'x'; ?></p><?= $total ?>"),
            &[
                ("p", Keyword),
                ("class", Type),
                ("\"a\"", Str),
                ("<?php", Keyword),
                ("echo", Keyword),
                ("strtoupper", Function),
                ("'x'", Str),
                ("?>", Keyword),
                ("p", Keyword),
                ("<?=", Keyword),
                ("?>", Keyword),
            ],
        );
        expect(php("<b>let if</b>"), &[("b", Keyword), ("b", Keyword)]);
    }

    #[test]
    fn php_keywords_classes_and_members() {
        expect(
            php("<?php\nfinal class Cart extends Base {\n  PUBLIC function add(int $n): ?self { return $this->items->push(new Item($n)) ?? Cart::make(); }\n}"),
            &[
                ("<?php", Keyword),
                ("final", Keyword),
                ("class", Keyword),
                ("Cart", Type),
                ("extends", Keyword),
                ("Base", Type),
                ("PUBLIC", Keyword),
                ("function", Keyword),
                ("add", Function),
                ("int", Type),
                ("self", Keyword),
                ("return", Keyword),
                ("push", Function),
                ("new", Keyword),
                ("Item", Type),
                ("Cart", Type),
                ("make", Function),
            ],
        );
    }

    #[test]
    fn php_comments_strings_and_heredoc() {
        expect(
            php("<?php // a ?> <i>\n<?php # b\n/* c */ $s = \"to\nlinjer\"; #[Attr]\n$h = <<<EOT\n  if 'x'\n  EOT;\n1.5"),
            &[
                ("<?php", Keyword),
                ("// a ", Comment),
                ("?>", Keyword),
                ("i", Keyword),
                ("<?php", Keyword),
                ("# b", Comment),
                ("/* c */", Comment),
                ("\"to\nlinjer\"", Str),
                ("Attr", Type),
                ("<<<EOT\n  if 'x'\n  EOT", Str),
                ("1.5", Number),
            ],
        );
    }

    #[test]
    fn yaml_keys_values_and_comments() {
        expect(
            yaml("# opsætning\nname: sEdit demo\nport: 8080\ndebug: yes # note\nimage: nginx:1.25\nurl: http://x.dk/#top\n\"quoted key\": 'it''s'\nempty:\n"),
            &[
                ("# opsætning", Comment),
                ("name", Keyword),
                ("sEdit demo", Str),
                ("port", Keyword),
                ("8080", Number),
                ("debug", Keyword),
                ("yes", Number),
                ("# note", Comment),
                ("image", Keyword),
                ("nginx:1.25", Str),
                ("url", Keyword),
                ("http://x.dk/#top", Str),
                ("\"quoted key\"", Keyword),
                ("'it'", Str),
                ("'s'", Str),
                ("empty", Keyword),
            ],
        );
    }

    #[test]
    fn yaml_lists_flow_and_anchors() {
        expect(
            yaml("---\nsteps:\n  - run: make\n  - -1.5\ntags: [a, 2, {k: v}]\nbase: &base !!map\ncopy: *base\nv: 1.2.3"),
            &[
                ("---", Comment),
                ("steps", Keyword),
                ("-", Keyword),
                ("run", Keyword),
                ("make", Str),
                ("-", Keyword),
                ("-1.5", Number),
                ("tags", Keyword),
                ("a", Str),
                ("2", Number),
                ("k", Keyword),
                ("v", Str),
                ("base", Keyword),
                ("&base", Type),
                ("!!map", Type),
                ("copy", Keyword),
                ("*base", Type),
                ("v", Keyword),
                ("1.2.3", Str),
            ],
        );
    }

    #[test]
    fn yaml_block_text_is_one_string() {
        expect(
            yaml("script: |\n  echo # ikke kommentar\n\n  key: ikke nøgle\nnext: >-\n  foldet\nlast: 1"),
            &[
                ("script", Keyword),
                ("  echo # ikke kommentar\n\n  key: ikke nøgle", Str),
                ("next", Keyword),
                ("  foldet", Str),
                ("last", Keyword),
                ("1", Number),
            ],
        );
    }

    fn py(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Python, source)
    }

    fn rs(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Rust, source)
    }

    fn md(source: &str) -> Vec<(String, Kind)> {
        tokens(Lang::Markdown, source)
    }

    #[test]
    fn python_rust_and_markdown_follow_the_file_extension() {
        assert_eq!(Lang::of(Path::new("app.py")), Some(Lang::Python));
        assert_eq!(Lang::of(Path::new("src/main.rs")), Some(Lang::Rust));
        assert_eq!(Lang::of(Path::new("README.md")), Some(Lang::Markdown));
        assert_eq!(Lang::of(Path::new("notes.MARKDOWN")), Some(Lang::Markdown));
    }

    #[test]
    fn python_keywords_calls_and_literals() {
        expect(
            py("def total(items: list[int]) -> float:\n    return sum(items) * 1.25 if items else None  # moms"),
            &[
                ("def", Keyword),
                ("total", Function),
                ("list", Type),
                ("int", Type),
                ("float", Type),
                ("return", Keyword),
                ("sum", Function),
                ("1.25", Number),
                ("if", Keyword),
                ("else", Keyword),
                ("None", Number),
                ("# moms", Comment),
            ],
        );
        expect(py("class Order(Base): pass"), &[("class", Keyword), ("Order", Type), ("Base", Type), ("pass", Keyword)]);
    }

    #[test]
    fn python_strings() {
        expect(
            py("a = f'{x}' + rb\"\\d\" + 'åben\nb = \"\"\"flere\nlinjer \"med\" det hele\"\"\" + u''"),
            &[("f'{x}'", Str), ("rb\"\\d\"", Str), ("'åben", Str), ("\"\"\"flere\nlinjer \"med\" det hele\"\"\"", Str), ("u''", Str)],
        );
        expect(py("'''aldrig lukket"), &[("'''aldrig lukket", Str)]);
        // Et navn der bare starter med et præfiksbogstav er ikke en streng.
        expect(py("buffer = bar"), &[]);
    }

    #[test]
    fn python_decorators_and_soft_keywords() {
        expect(
            py("@app.route('/x')\nasync def f(): ...\nx = a @ b"),
            &[("@app.route", Function), ("'/x'", Str), ("async", Keyword), ("def", Keyword), ("f", Function)],
        );
        expect(py("match command:\n    case [x]: pass"), &[("match", Keyword), ("case", Keyword), ("pass", Keyword)]);
        expect(py("match = re.match(p, s)\nobj.import"), &[("match", Function)]);
    }

    #[test]
    fn rust_keywords_types_and_macros() {
        expect(
            rs("pub fn area(w: u32, h: f64) -> Option<f64> { println!(\"{}\", w); Some(MAX as f64) }"),
            &[
                ("pub", Keyword),
                ("fn", Keyword),
                ("area", Function),
                ("u32", Type),
                ("f64", Type),
                ("Option", Type),
                ("f64", Type),
                ("println!", Function),
                ("\"{}\"", Str),
                ("Some", Type),
                ("as", Keyword),
                ("f64", Type),
            ],
        );
        expect(rs("if a != b { x.await }"), &[("if", Keyword), ("await", Keyword)]);
    }

    #[test]
    fn rust_chars_lifetimes_and_strings() {
        expect(
            rs("fn f<'a>(s: &'a str) { let c = 'x'; let n = '\\n'; let q = '\\''; }"),
            &[
                ("fn", Keyword),
                ("f", Function),
                ("'a", Type),
                ("'a", Type),
                ("str", Type),
                ("let", Keyword),
                ("'x'", Str),
                ("let", Keyword),
                ("'\\n'", Str),
                ("let", Keyword),
                ("'\\''", Str),
            ],
        );
        expect(
            rs("r#\"rå \"streng\" her\"# b\"bytes\" b'x' \"to\nlinjer \\\" stadig\" r"),
            &[("r#\"rå \"streng\" her\"#", Str), ("b\"bytes\"", Str), ("b'x'", Str), ("\"to\nlinjer \\\" stadig\"", Str)],
        );
    }

    #[test]
    fn rust_comments_attributes_and_numbers() {
        expect(
            rs("/* ydre /* indre */ stadig */ let x = 1_000u64 + 0xFF; // slut\n#[derive(Debug, Clone)]\nstruct P;"),
            &[
                ("/* ydre /* indre */ stadig */", Comment),
                ("let", Keyword),
                ("1_000u64", Number),
                ("0xFF", Number),
                ("// slut", Comment),
                ("#[derive(Debug, Clone)]", Type),
                ("struct", Keyword),
                ("P", Type),
            ],
        );
        expect(rs("for i in 0..10 { v[i].max(1.5) }"), &[("for", Keyword), ("in", Keyword), ("0", Number), ("10", Number), ("max", Function), ("1.5", Number)]);
    }

    #[test]
    fn markdown_headings_lists_and_quotes() {
        expect(
            md("# Titel\n\nTekst #ikke overskrift\n- punkt\n  1. [x] færdig\n> citat\n---\n-ikke punkt"),
            &[
                ("# Titel", Keyword),
                ("-", Keyword),
                ("1.", Keyword),
                ("[x]", Number),
                ("> citat", Comment),
                ("---", Comment),
            ],
        );
    }

    #[test]
    fn markdown_inline_code_bold_and_links() {
        expect(
            md("Brug `cargo build` og **husk** [docs](https://x.dk/a) <https://y.dk> a * b"),
            &[
                ("`cargo build`", Str),
                ("**husk**", Function),
                ("[docs]", Type),
                ("(https://x.dk/a)", Str),
                ("<https://y.dk>", Str),
            ],
        );
        expect(md("`åben og ** alene [uden] adresse"), &[]);
        expect(md("før <!-- skjult\nover linjer --> efter `x`"), &[("<!-- skjult\nover linjer -->", Comment), ("`x`", Str)]);
    }

    #[test]
    fn markdown_code_blocks_are_colored_in_their_language() {
        expect(
            md("```js\nconst a = 1;\n```\n# efter"),
            &[("```js", Comment), ("const", Keyword), ("1", Number), ("```", Comment), ("# efter", Keyword)],
        );
        // Et ukendt sprog vises som én blok, og # inde i den er ikke en overskrift.
        expect(md("~~~\n# ikke overskrift\n~~~"), &[("~~~", Comment), ("# ikke overskrift\n", Str), ("~~~", Comment)]);
        expect(md("```python\nx = 1"), &[("```python", Comment), ("1", Number)]);
    }

    #[test]
    fn tokens_are_in_order_and_never_overlap() {
        let source = "`a${b}c` /x/ 'y' -- z\n/* q */ $$ 1.2.3 N'' é£ \u{1F600} `${`${1}`}` \
            <a b='c' d=e>&amp;<script>x('</script><style>a:b{c:url( d) #fff -1px !x}</style><!-- q --> \
            {\"k\": [1, true]} @media (x: 1) { .a::b[c=\"d\"] { e: f(g) } } <p";
        let more = "\n```rs\nfn f<'a>() { r#\"x\"# '\\n' b' }\n```\n- [x] **a** [b](c) `d` \n# h\n> q\n@x.y\n\
            rb''' f\"\"\" /* /* */ #[a(b)] #![c '
<?php $a->b(<<<X
 x
X) ?> <?= @\"\"\"\" $@\"q\" k: |
  t
- [a, {b: *c}] '";
        let source = &[source, more].concat();
        let langs = [
            Lang::JavaScript,
            Lang::TypeScript,
            Lang::Sql,
            Lang::Json,
            Lang::Html,
            Lang::Css,
            Lang::Python,
            Lang::Rust,
            Lang::Markdown,
            Lang::CSharp,
            Lang::Php,
            Lang::Yaml,
            Lang::Go,
            Lang::Java,
            Lang::Xml,
            Lang::C,
            Lang::Cpp,
            Lang::Bash,
            Lang::Kotlin,
            Lang::Swift,
            Lang::Ruby,
            Lang::PowerShell,
            Lang::Lua,
            Lang::Toml,
            Lang::Dockerfile,
            Lang::Ini,
            Lang::Batch,
            Lang::Env,
            Lang::Vue,
            Lang::Svelte,
            Lang::Scss,
        ];
        for lang in langs {
            let text: Vec<u16> = source.replace('\n', "\r").encode_utf16().collect();
            // Hvert præfiks af teksten skal også kunne klares; sådan ser den ud mens man skriver.
            for len in 0..=text.len() {
                let mut end = 0;
                for token in tokenize(lang, &text[..len]) {
                    assert!(token.start >= end && token.end > token.start && token.end <= len, "{lang:?} {len} {token:?}");
                    end = token.end;
                }
            }
        }
    }
}
