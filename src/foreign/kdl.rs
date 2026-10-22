//! A small KDL subset reader — enough of the grammar for usage specs
//! ([usage.jdx.dev](https://usage.jdx.dev)): named nodes with string /
//! number / keyword entries, `key=value` props, `{ }` child blocks,
//! `//` and `/* */` comments, `/-` slash-dash elision, `(type)`
//! annotations, `\` line continuations, `;` terminators, escaped /
//! raw (`#"…"#`) / multiline (`"""`) strings, and bare-word values
//! (`name mise` — common in real `*.usage.kdl` files, though outside
//! the letter of the KDL spec).

/// One KDL node: `name "arg"… key=value… { children }`.
#[derive(Debug)]
pub(crate) struct KdlNode {
    /// Node name (`flag`, `cmd`, …).
    pub name: String,
    /// Positional entries after the name.
    pub args: Vec<KdlVal>,
    /// `key=value` props, in document order.
    pub props: Vec<(String, KdlVal)>,
    /// `{ }` child nodes.
    pub children: Vec<KdlNode>,
}

/// An entry or prop value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KdlVal {
    Str(String),
    Bool(bool),
    Num(String),
    Null,
}

impl KdlVal {
    /// The string payload, or `None` for non-strings.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            KdlVal::Str(s) => Some(s),
            _ => None,
        }
    }

    /// `#true` / `true`.
    pub fn truthy(&self) -> bool {
        matches!(self, KdlVal::Bool(true))
    }

    /// A display form for doc annotations: strings and numbers render
    /// as themselves, booleans as `true`/`false`; `Null` renders `None`.
    pub fn render(&self) -> Option<String> {
        match self {
            KdlVal::Str(s) | KdlVal::Num(s) => Some(s.clone()),
            KdlVal::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
            KdlVal::Null => None,
        }
    }
}

impl KdlNode {
    /// First prop value named `key`.
    pub fn prop(&self, key: &str) -> Option<&KdlVal> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Positional entry `i` as a string.
    pub fn arg_str(&self, i: usize) -> Option<&str> {
        self.args.get(i).and_then(KdlVal::as_str)
    }

    /// Child nodes named `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a KdlNode> {
        self.children.iter().filter(move |c| c.name == name)
    }
}

/// Parse a whole KDL document into its top-level nodes.
pub(crate) fn parse(src: &str) -> Result<Vec<KdlNode>, String> {
    let mut p = P {
        b: src,
        i: 0,
        line: 1,
    };
    let mut nodes = Vec::new();
    loop {
        p.trivia(true)?;
        match p.peek() {
            None => return Ok(nodes),
            Some(b'}') => return Err(p.err("unmatched `}`")),
            _ if p.at_slashdash() => {
                p.i += 2;
                p.trivia(false)?;
                p.node()?;
            }
            _ => nodes.push(p.node()?),
        }
    }
}

struct P<'a> {
    b: &'a str,
    i: usize,
    line: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.as_bytes().get(self.i).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.b.as_bytes().get(self.i + off).copied()
    }

    fn err(&self, msg: &str) -> String {
        format!("line {}: {msg}", self.line)
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if let Some(b'\n') = c {
            self.line += 1;
        }
        if c.is_some() {
            self.i += 1;
        }
        c
    }

    /// `/-` — the slash-dash elision marker.
    fn at_slashdash(&self) -> bool {
        self.peek() == Some(b'/') && self.peek_at(1) == Some(b'-')
    }

    /// Whitespace, comments, `\`-continuations; newlines and `;` only
    /// when `newlines` (between nodes, not inside an entry list).
    fn trivia(&mut self, newlines: bool) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | 0x0b | 0x0c | b'\r') => {
                    self.i += 1;
                }
                Some(b'\n' | b';') if newlines => {
                    self.bump();
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => self.block_comment()?,
                Some(b'\\') => {
                    self.i += 1;
                    while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
                        self.i += 1;
                    }
                    match self.peek() {
                        Some(b'\n') => {
                            self.bump();
                        }
                        _ => return Err(self.err("expected a newline after `\\`")),
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn block_comment(&mut self) -> Result<(), String> {
        self.i += 2;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            if c == b'/' && self.peek_at(1) == Some(b'*') {
                depth += 1;
                self.i += 2;
            } else if c == b'*' && self.peek_at(1) == Some(b'/') {
                depth -= 1;
                self.i += 2;
                if depth == 0 {
                    return Ok(());
                }
            } else {
                self.bump();
            }
        }
        Err(self.err("unterminated `/*` comment"))
    }

    /// An optional `(type)` annotation — consumed and ignored.
    fn ty(&mut self) -> Result<(), String> {
        if self.peek() != Some(b'(') {
            return Ok(());
        }
        self.i += 1;
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == b')' {
                return Ok(());
            }
        }
        Err(self.err("unterminated `(type)` annotation"))
    }

    /// One node: name, entries/props, optional `{ children }`.
    fn node(&mut self) -> Result<KdlNode, String> {
        self.ty()?;
        let name = self.word_like("a node name")?;
        let mut n = KdlNode {
            name,
            args: Vec::new(),
            props: Vec::new(),
            children: Vec::new(),
        };
        loop {
            self.trivia(false)?;
            match self.peek() {
                None | Some(b'\n' | b';' | b'}') => {
                    return Ok(n);
                }
                Some(b'{') => {
                    self.i += 1;
                    n.children = self.children()?;
                    return Ok(n);
                }
                _ if self.at_slashdash() => {
                    self.i += 2;
                    self.trivia(false)?;
                    if self.peek() == Some(b'{') {
                        self.i += 1;
                        self.children()?;
                    } else {
                        self.entry()?;
                    }
                }
                _ => {
                    let entry = self.entry()?;
                    match entry {
                        Entry::Arg(v) => n.args.push(v),
                        Entry::Prop(k, v) => n.props.push((k, v)),
                    }
                }
            }
        }
    }

    /// `{ node* }` — the caller just consumed `{`.
    fn children(&mut self) -> Result<Vec<KdlNode>, String> {
        let mut nodes = Vec::new();
        loop {
            self.trivia(true)?;
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    return Ok(nodes);
                }
                None => return Err(self.err("unterminated `{` block")),
                _ if self.at_slashdash() => {
                    self.i += 2;
                    self.trivia(false)?;
                    self.node()?;
                }
                _ => nodes.push(self.node()?),
            }
        }
    }

    /// One entry: a `key = value` prop or a bare value.
    fn entry(&mut self) -> Result<Entry, String> {
        self.ty()?;
        // A bare word followed by `=` is a prop key; otherwise the
        // word itself is the value.
        if matches!(self.peek(), Some(c) if is_word(c)) {
            let w = self.word();
            self.trivia(false)?;
            if self.peek() == Some(b'=') {
                self.i += 1;
                self.trivia(false)?;
                return Ok(Entry::Prop(w, self.value()?));
            }
            return Ok(Entry::Arg(classify(&w)));
        }
        Ok(Entry::Arg(self.value()?))
    }

    /// A node name or a bare value token: `"quoted"` / `#"raw"#` /
    /// `"""multi"""` / an unquoted word.
    fn word_like(&mut self, what: &str) -> Result<String, String> {
        match self.peek() {
            Some(b'"') => self.string(),
            Some(b'#') => self.hash(),
            Some(c) if is_word(c) => Ok(self.word()),
            _ => Err(self.err(&format!("expected {what}"))),
        }
    }

    /// An entry/prop value.
    fn value(&mut self) -> Result<KdlVal, String> {
        match self.peek() {
            Some(b'"') => Ok(KdlVal::Str(self.string()?)),
            Some(b'#') => self.hash_val(),
            Some(b'(') => {
                self.ty()?;
                self.value()
            }
            Some(c) if is_word(c) => Ok(classify(&self.word())),
            _ => Err(self.err("expected a value")),
        }
    }

    /// `"""` opens a multiline string; a single `"` an escaped one.
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        if self.peek() == Some(b'"') && self.peek_at(1) == Some(b'"') {
            return self.multiline();
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => self.escape(&mut out)?,
                Some(_) => {
                    // Consume one UTF-8 code point.
                    let ch = self.b[self.i..].chars().next().expect("non-empty slice");
                    out.push(ch);
                    if ch == '\n' {
                        self.line += 1;
                    }
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    /// Inside `"""`: skip the opening newline, read until `"""`.
    fn multiline(&mut self) -> Result<String, String> {
        self.i += 2;
        if self.peek() == Some(b'\r') && self.peek_at(1) == Some(b'\n') {
            self.i += 1;
        }
        if self.peek() == Some(b'\n') {
            self.bump();
        }
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'"' && self.peek_at(1) == Some(b'"') && self.peek_at(2) == Some(b'"') {
                let out = self.b[start..self.i].to_string();
                self.i += 3;
                return Ok(out);
            }
            self.bump();
        }
        Err(self.err("unterminated multiline string"))
    }

    /// `#"raw"#`, `#"""raw"""#`, or a `#true`/`#null` keyword when it
    /// is not a string — used where a *name* is expected, so keywords
    /// error.
    fn hash(&mut self) -> Result<String, String> {
        let hashes = self.count_hashes();
        match self.peek() {
            Some(b'"') => self.raw(hashes),
            _ => Err(self.err("expected a raw string")),
        }
    }

    /// `#"…"#` value, or a `#true`/`#false`/`#null`/`#inf`/`#nan`
    /// keyword.
    fn hash_val(&mut self) -> Result<KdlVal, String> {
        let hashes = self.count_hashes();
        if self.peek() == Some(b'"') {
            return Ok(KdlVal::Str(self.raw(hashes)?));
        }
        let w = self.word();
        match w.as_str() {
            "true" => Ok(KdlVal::Bool(true)),
            "false" => Ok(KdlVal::Bool(false)),
            "null" => Ok(KdlVal::Null),
            "inf" | "-inf" | "nan" => Ok(KdlVal::Num(w)),
            _ => Err(self.err(&format!("unknown keyword `#{w}`"))),
        }
    }

    /// Past a `#` run — returns the count.
    fn count_hashes(&mut self) -> usize {
        let mut n = 0;
        while self.peek() == Some(b'#') {
            self.i += 1;
            n += 1;
        }
        n
    }

    /// A raw string after the opening `#`s: `"…"###` or `"""…"""###`.
    fn raw(&mut self, hashes: usize) -> Result<String, String> {
        self.i += 1; // opening `"`
        if self.peek() == Some(b'"') && self.peek_at(1) == Some(b'"') {
            self.i += 2;
            if self.peek() == Some(b'\r') && self.peek_at(1) == Some(b'\n') {
                self.i += 1;
            }
            if self.peek() == Some(b'\n') {
                self.bump();
            }
            let start = self.i;
            while self.peek().is_some() {
                if self.peek() == Some(b'"')
                    && self.peek_at(1) == Some(b'"')
                    && self.peek_at(2) == Some(b'"')
                    && self.hashes_at(self.i + 3) == hashes
                {
                    let out = self.b[start..self.i].to_string();
                    self.i += 3 + hashes;
                    return Ok(out);
                }
                self.bump();
            }
            return Err(self.err("unterminated raw string"));
        }
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'"' && self.hashes_at(self.i + 1) == hashes {
                let out = self.b[start..self.i].to_string();
                self.i += 1 + hashes;
                return Ok(out);
            }
            self.bump();
        }
        Err(self.err("unterminated raw string"))
    }

    /// Exactly `n` `#`s at byte offset `at` — the raw-string closer.
    fn hashes_at(&self, at: usize) -> usize {
        let mut n = 0;
        while self.b.as_bytes().get(at + n) == Some(&b'#') {
            n += 1;
        }
        n
    }

    /// `\x` inside a string: a named escape, `\u{…}`, or a whitespace
    /// line continuation.
    fn escape(&mut self, out: &mut String) -> Result<(), String> {
        self.i += 1;
        match self.peek() {
            Some(b'n') => out.push('\n'),
            Some(b'r') => out.push('\r'),
            Some(b't') => out.push('\t'),
            Some(b'b') => out.push('\u{8}'),
            Some(b'f') => out.push('\u{c}'),
            Some(b's') => out.push(' '),
            Some(b'"') => out.push('"'),
            Some(b'/') => out.push('/'),
            Some(b'\\') => out.push('\\'),
            Some(b'u') => {
                self.i += 1;
                match self.peek() {
                    Some(b'{') => {
                        self.i += 1;
                        let start = self.i;
                        while matches!(self.peek(), Some(c) if c.is_ascii_hexdigit()) {
                            self.i += 1;
                        }
                        match self.peek() {
                            Some(b'}') => {
                                let hex = &self.b[start..self.i];
                                let cp = u32::from_str_radix(hex, 16)
                                    .ok()
                                    .and_then(char::from_u32)
                                    .ok_or_else(|| self.err("bad \\u{…} escape"))?;
                                out.push(cp);
                                self.i += 1;
                            }
                            _ => return Err(self.err("unterminated \\u{…} escape")),
                        }
                        return Ok(());
                    }
                    _ => return Err(self.err("expected `{` after `\\u`")),
                }
            }
            // A `\` followed by whitespace + a newline folds the
            // continuation away (string escapes newlines).
            Some(b' ' | b'\t' | b'\r' | b'\n') => {
                while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
                    self.i += 1;
                }
                match self.peek() {
                    Some(b'\n') => {
                        self.bump();
                        return Ok(());
                    }
                    _ => return Err(self.err("expected a newline after `\\`")),
                }
            }
            Some(c) => return Err(self.err(&format!("unknown escape `\\{}`", c as char))),
            None => return Err(self.err("unterminated string")),
        }
        self.i += 1;
        Ok(())
    }

    /// A run of bare-word bytes → string.
    fn word(&mut self) -> String {
        let start = self.i;
        while matches!(self.peek(), Some(c) if is_word(c)) {
            self.i += 1;
        }
        self.b[start..self.i].to_string()
    }
}

enum Entry {
    Arg(KdlVal),
    Prop(String, KdlVal),
}

/// Bare-word bytes: anything that cannot start another construct —
/// excludes whitespace, `"` `'` `#` `(` `)` `{` `}` `<` `>` `[` `]`
/// `;` `,` `:` `=` `\` `/` and control bytes.
fn is_word(c: u8) -> bool {
    !matches!(
        c,
        0..=0x20
            | b'"'
            | b'\''
            | b'#'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'<'
            | b'>'
            | b'['
            | b']'
            | b';'
            | b','
            | b':'
            | b'='
            | b'\\'
            | b'/'
    )
}

/// A bare word → a value: KDL v1 keywords, a number, or a string.
fn classify(w: &str) -> KdlVal {
    match w {
        "true" => KdlVal::Bool(true),
        "false" => KdlVal::Bool(false),
        "null" => KdlVal::Null,
        _ => {
            let mut cs = w.chars();
            let numeric = match cs.next() {
                Some(c) if c.is_ascii_digit() => true,
                Some('+' | '-') => cs.next().is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            };
            if numeric {
                KdlVal::Num(w.to_string())
            } else {
                KdlVal::Str(w.to_string())
            }
        }
    }
}
