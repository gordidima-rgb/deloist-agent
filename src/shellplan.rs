//! Разбор команд PowerShell без исполнения: что команда читает, пишет и удаляет.
//!
//! Две части. `ps_plan` — строгая грамматика: узнаёт только скрипты из безобидных кирпичей
//! (литералы, переменные, чистые методы строк, `Set-Content`, `WriteAllText`, `Get-Content`…) и на всё
//! остальное отвечает `None`. Решение «карточка не нужна» принимается только по ней (см. `tools::needs_approval_for`).
//! `scan` — терпимый просмотр для текста карточки: ищет известные глаголы и литералы, ничего не решает.
//! Текст, который пишет модель (`purpose`), сюда не попадает никогда.

use std::collections::HashMap;

/// Что делает один шаг скрипта.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Read,
    /// Запись целиком (`Set-Content`, `WriteAllText`, `Out-File`).
    Write,
    /// Дописывание в конец.
    Append,
    /// Удаление одного файла.
    Delete,
    MakeDir,
    Copy,
    Move,
}

#[derive(Clone, Debug)]
pub struct Op {
    pub kind: OpKind,
    /// Пути, которые шаг трогает (буквально из скрипта, после подстановки переменных-литералов).
    pub paths: Vec<String>,
}

/// Итог строгого разбора.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub ops: Vec<Op>,
    /// Сколько раз скрипт заменяет куски текста (`.Replace`, `-replace`).
    pub replaces: usize,
    /// Сколько знаков текста он дописывает или записывает литералами.
    pub literal_chars: usize,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Str(String),
    Var(String),
    Word(String),
    /// `[Тип]::Член`.
    Static(String, String),
    P(char),
    Dot,
}

/// Символы, которые PowerShell читает иначе, чем видит глаз: типографские кавычки закрывают строку,
/// «тире» работает как дефис, невидимые знаки и полуширинные формы прячут команду. Такой скрипт не разбираем.
pub fn has_lookalikes(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{2010}'..='\u{2015}' | '\u{2018}'..='\u{201F}' | '\u{00A0}' | '\u{2000}'..='\u{200F}' | '\u{2028}'..='\u{202F}'
            | '\u{2060}'..='\u{206F}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{FF00}'..='\u{FFEF}' | '\u{2212}' | '\u{00AD}' | '\u{0085}')
            || (c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    })
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '\\' | '/' | ':' | '*' | '?' | '~')
}

fn tokenize(src: &str) -> Option<Vec<Tok>> {
    if has_lookalikes(src) {
        return None;
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    let mut depth = 0i32;
    // Сразу перед позицией стоит значение (строка, переменная, `)`): тогда `.` — доступ к члену.
    let mut adjacent_value = false;
    while i < chars.len() {
        let c = chars[i];
        let was_adjacent = adjacent_value;
        adjacent_value = false;
        match c {
            '\n' | '\r' => {
                if depth == 0 && !matches!(out.last(), Some(Tok::P(';'))) && !out.is_empty() {
                    out.push(Tok::P(';'));
                }
                i += 1;
            }
            ' ' | '\t' => i += 1,
            ';' => {
                if !matches!(out.last(), Some(Tok::P(';'))) && !out.is_empty() {
                    out.push(Tok::P(';'));
                }
                i += 1;
            }
            '\'' => {
                let mut text = String::new();
                i += 1;
                loop {
                    let ch = *chars.get(i)?;
                    if ch == '\'' {
                        if chars.get(i + 1) == Some(&'\'') {
                            text.push('\'');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    text.push(ch);
                    i += 1;
                }
                out.push(Tok::Str(text));
                adjacent_value = true;
            }
            '"' => {
                let mut text = String::new();
                i += 1;
                loop {
                    let ch = *chars.get(i)?;
                    match ch {
                        '"' => {
                            if chars.get(i + 1) == Some(&'"') {
                                text.push('"');
                                i += 2;
                                continue;
                            }
                            i += 1;
                            break;
                        }
                        '$' => return None,
                        '`' => {
                            let next = *chars.get(i + 1)?;
                            let mapped = match next {
                                'n' => '\n',
                                'r' => '\r',
                                't' => '\t',
                                '"' | '\'' | '`' | '$' => next,
                                _ => return None,
                            };
                            text.push(mapped);
                            i += 2;
                        }
                        _ => {
                            text.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Str(text));
                adjacent_value = true;
            }
            '@' => {
                // Строка-блок: `@'` или `@"`, сразу перевод строки, конец — `'@` в начале строки.
                let quote = *chars.get(i + 1)?;
                if quote != '\'' && quote != '"' {
                    return None;
                }
                let mut j = i + 2;
                while matches!(chars.get(j), Some(' ' | '\t')) {
                    j += 1;
                }
                if chars.get(j) == Some(&'\r') {
                    j += 1;
                }
                if chars.get(j) != Some(&'\n') {
                    return None;
                }
                j += 1;
                let start = j;
                let mut end = None;
                while j < chars.len() {
                    if (j == start || chars[j - 1] == '\n') && chars[j] == quote && chars.get(j + 1) == Some(&'@') {
                        end = Some(j);
                        break;
                    }
                    j += 1;
                }
                let end = end?;
                let mut body: String = chars[start..end].iter().collect();
                if body.ends_with('\n') {
                    body.pop();
                    if body.ends_with('\r') {
                        body.pop();
                    }
                }
                if quote == '"' && body.chars().any(|ch| ch == '$' || ch == '`') {
                    return None;
                }
                i = end + 2;
                // После закрытия — только пробел, `;`, `)`, `,`, `|` или конец.
                if let Some(next) = chars.get(i) {
                    if !matches!(next, ' ' | '\t' | '\r' | '\n' | ';' | ')' | ',' | '|' | '.') {
                        return None;
                    }
                }
                out.push(Tok::Str(body));
                adjacent_value = true;
            }
            '$' => {
                let mut j = i + 1;
                let mut name = String::new();
                while let Some(&ch) = chars.get(j) {
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        name.push(ch);
                        j += 1;
                    } else {
                        break;
                    }
                }
                // `$(`, `${`, `$env:X`, `$_`, `$$` — не разбираем.
                if name.is_empty() || name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) || chars.get(j) == Some(&':') {
                    return None;
                }
                i = j;
                out.push(Tok::Var(name.to_lowercase()));
                adjacent_value = true;
            }
            '[' => {
                let close = chars[i..].iter().position(|&ch| ch == ']')? + i;
                let ty: String = chars[i + 1..close].iter().collect();
                if ty.is_empty() || !ty.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_') {
                    return None;
                }
                if chars.get(close + 1) != Some(&':') || chars.get(close + 2) != Some(&':') {
                    return None;
                }
                let mut j = close + 3;
                let mut member = String::new();
                while let Some(&ch) = chars.get(j) {
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        member.push(ch);
                        j += 1;
                    } else {
                        break;
                    }
                }
                if member.is_empty() {
                    return None;
                }
                i = j;
                out.push(Tok::Static(ty.to_lowercase(), member.to_lowercase()));
                adjacent_value = true;
            }
            '(' => {
                depth += 1;
                out.push(Tok::P('('));
                i += 1;
            }
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
                out.push(Tok::P(')'));
                i += 1;
                adjacent_value = true;
            }
            '|' | ',' | '=' | '+' => {
                out.push(Tok::P(c));
                i += 1;
            }
            '.' if was_adjacent => {
                out.push(Tok::Dot);
                i += 1;
            }
            _ if is_word_char(c) => {
                let mut word = String::new();
                while let Some(&ch) = chars.get(i) {
                    if is_word_char(ch) {
                        word.push(ch);
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(Tok::Word(word));
            }
            // `{ } & > < ! % # ^ \`` и всё прочее: скрипт не из безобидных кирпичей.
            _ => return None,
        }
    }
    if depth != 0 {
        return None;
    }
    while matches!(out.last(), Some(Tok::P(';'))) {
        out.pop();
    }
    Some(out)
}

/// Что известно о значении выражения.
#[derive(Clone, Debug, Default)]
struct Val {
    lit: Option<String>,
}

struct Parser<'a> {
    toks: &'a [Tok],
    at: usize,
    vars: HashMap<String, Val>,
    plan: Plan,
}

const PURE_METHODS: &[&str] = &[
    "replace", "trim", "trimstart", "trimend", "insert", "substring", "contains", "startswith", "endswith", "tolower", "toupper", "indexof",
    "remove", "padleft", "padright", "tostring", "split",
];

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn eat(&mut self, p: char) -> bool {
        if self.peek() == Some(&Tok::P(p)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn at_end(&self) -> bool {
        matches!(self.peek(), None | Some(Tok::P(';')))
    }

    fn statement(&mut self) -> Option<()> {
        // `$v = expr` и `$v += expr`.
        if let (Some(Tok::Var(name)), Some(Tok::P(op))) = (self.toks.get(self.at), self.toks.get(self.at + 1)) {
            let name = name.clone();
            if *op == '=' {
                self.at += 2;
                let val = self.expr()?;
                self.vars.insert(name, val);
                return Some(());
            }
            if *op == '+' && self.toks.get(self.at + 2) == Some(&Tok::P('=')) {
                self.at += 3;
                let add = self.expr()?;
                let old = self.vars.get(&name)?.clone();
                let lit = match (old.lit, add.lit) {
                    (Some(a), Some(b)) => Some(format!("{a}{b}")),
                    _ => None,
                };
                self.vars.insert(name, Val { lit });
                return Some(());
            }
        }
        // Конвейер: `выражение | приёмник` или просто команда.
        let start = self.at;
        if let Some(Tok::Word(w)) = self.peek() {
            if is_cmdlet(&w.to_lowercase()) {
                return self.command(None);
            }
        }
        if let Some(Tok::Static(..)) = self.peek() {
            if self.static_sink()? {
                return Some(());
            }
            self.at = start;
        }
        let val = self.expr()?;
        if self.eat('|') {
            let Some(Tok::Word(w)) = self.peek() else { return None };
            if !is_cmdlet(&w.to_lowercase()) {
                return None;
            }
            return self.command(Some(val));
        }
        // Просто выражение: его значение напечатается; ничего не меняет.
        Some(())
    }

    fn expr(&mut self) -> Option<Val> {
        let mut left = self.term()?;
        loop {
            match self.peek() {
                Some(Tok::P('+')) if self.toks.get(self.at + 1) != Some(&Tok::P('=')) => {
                    self.at += 1;
                    let right = self.term()?;
                    left.lit = match (left.lit, right.lit) {
                        (Some(a), Some(b)) => Some(format!("{a}{b}")),
                        _ => None,
                    };
                }
                Some(Tok::Word(w)) if matches!(w.to_lowercase().as_str(), "-replace" | "-creplace" | "-join" | "-f") => {
                    let is_replace = w.to_lowercase().contains("replace");
                    self.at += 1;
                    // Правая часть — список через запятую.
                    self.term()?;
                    while self.eat(',') {
                        self.term()?;
                    }
                    if is_replace {
                        self.plan.replaces += 1;
                    }
                    left.lit = None;
                }
                _ => break,
            }
        }
        Some(left)
    }

    fn term(&mut self) -> Option<Val> {
        let mut val = match self.toks.get(self.at)?.clone() {
            Tok::Str(text) => {
                self.at += 1;
                self.plan.literal_chars += text.chars().count();
                Val { lit: Some(text) }
            }
            Tok::Var(name) => {
                self.at += 1;
                match name.as_str() {
                    "true" => Val { lit: Some("True".into()) },
                    "false" => Val { lit: Some("False".into()) },
                    "null" => Val { lit: Some(String::new()) },
                    _ => self.vars.get(&name)?.clone(),
                }
            }
            Tok::P('(') => {
                self.at += 1;
                // `(New-Object System.Text.UTF8Encoding $false)` — кодировка.
                if let Some(Tok::Word(w)) = self.peek() {
                    if w.eq_ignore_ascii_case("new-object") {
                        return self.new_encoding();
                    }
                }
                let inner = self.expr()?;
                if !self.eat(')') {
                    return None;
                }
                inner
            }
            Tok::Static(ty, member) => {
                self.at += 1;
                match (ty.as_str(), member.as_str()) {
                    ("system.io.file" | "io.file", "readalltext" | "readalllines" | "exists") => {
                        let paths = self.call_paths()?;
                        let kind = if member == "exists" { OpKind::Read } else { OpKind::Read };
                        self.plan.ops.push(Op { kind, paths });
                        Val::default()
                    }
                    ("system.text.encoding" | "text.encoding", "utf8" | "unicode" | "default" | "ascii") => Val::default(),
                    ("system.text.utf8encoding" | "text.utf8encoding", "new") => {
                        self.skip_call_args()?;
                        Val::default()
                    }
                    ("system.environment" | "environment", "newline") => Val { lit: Some("\r\n".into()) },
                    _ => return None,
                }
            }
            Tok::Word(w) => {
                let lower = w.to_lowercase();
                if matches!(lower.as_str(), "get-content") {
                    self.command_read()?;
                    Val::default()
                } else if w.chars().all(|c| c.is_ascii_digit()) {
                    self.at += 1;
                    Val::default()
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // Методы строк: `.Replace('a','b')`.
        while self.peek() == Some(&Tok::Dot) {
            self.at += 1;
            let Some(Tok::Word(name)) = self.peek().cloned() else { return None };
            let lower = name.to_lowercase();
            if !PURE_METHODS.contains(&lower.as_str()) {
                return None;
            }
            self.at += 1;
            if !self.eat('(') {
                return None;
            }
            if !self.eat(')') {
                self.expr()?;
                while self.eat(',') {
                    self.expr()?;
                }
                if !self.eat(')') {
                    return None;
                }
            }
            if lower == "replace" {
                self.plan.replaces += 1;
            }
            val.lit = None;
        }
        Some(val)
    }

    fn new_encoding(&mut self) -> Option<Val> {
        // Слово `New-Object` уже на позиции; дальше имя класса, необязательный литерал и `)`.
        self.at += 1;
        let Some(Tok::Word(class)) = self.peek().cloned() else { return None };
        if !matches!(class.to_lowercase().as_str(), "system.text.utf8encoding" | "text.utf8encoding" | "system.text.unicodeencoding") {
            return None;
        }
        self.at += 1;
        if let Some(Tok::Var(v)) = self.peek() {
            if v != "true" && v != "false" {
                return None;
            }
            self.at += 1;
        }
        if !self.eat(')') {
            return None;
        }
        Some(Val::default())
    }

    fn skip_call_args(&mut self) -> Option<()> {
        if !self.eat('(') {
            return None;
        }
        if self.eat(')') {
            return Some(());
        }
        loop {
            match self.peek()? {
                Tok::Var(v) if v == "true" || v == "false" => self.at += 1,
                Tok::Str(_) | Tok::Word(_) => self.at += 1,
                _ => return None,
            }
            if self.eat(')') {
                return Some(());
            }
            if !self.eat(',') {
                return None;
            }
        }
    }

    /// `(путь [, ...])` для чтения: берём первый аргумент как путь, остальные разбираем как выражения.
    fn call_paths(&mut self) -> Option<Vec<String>> {
        if !self.eat('(') {
            return None;
        }
        let first = self.expr()?;
        let path = first.lit?;
        while self.eat(',') {
            self.expr()?;
        }
        if !self.eat(')') {
            return None;
        }
        Some(vec![path])
    }

    /// `[System.IO.File]::WriteAllText(путь, текст[, кодировка])` как отдельный шаг.
    fn static_sink(&mut self) -> Option<bool> {
        let Some(Tok::Static(ty, member)) = self.peek().cloned() else { return Some(false) };
        if !matches!(ty.as_str(), "system.io.file" | "io.file") {
            return Some(false);
        }
        let kind = match member.as_str() {
            "writealltext" | "writealllines" => OpKind::Write,
            "appendalltext" | "appendalllines" => OpKind::Append,
            "delete" => OpKind::Delete,
            "readalltext" | "readalllines" | "exists" => return Some(false),
            _ => return None,
        };
        self.at += 1;
        if !self.eat('(') {
            return None;
        }
        let path = self.expr()?.lit?;
        if kind != OpKind::Delete {
            if !self.eat(',') {
                return None;
            }
            self.expr()?;
            if self.eat(',') {
                self.expr()?;
            }
        }
        if !self.eat(')') {
            return None;
        }
        self.plan.ops.push(Op { kind, paths: vec![path] });
        Some(true)
    }

    fn command_read(&mut self) -> Option<()> {
        self.at += 1;
        let args = self.cmd_args("get-content")?;
        let path = args.path?;
        self.plan.ops.push(Op { kind: OpKind::Read, paths: vec![path] });
        Some(())
    }

    fn command(&mut self, piped: Option<Val>) -> Option<()> {
        let Some(Tok::Word(name)) = self.peek().cloned() else { return None };
        let lower = name.to_lowercase();
        self.at += 1;
        let args = self.cmd_args(&lower)?;
        let has_content = piped.is_some() || args.value.is_some();
        match lower.as_str() {
            "set-content" | "out-file" => {
                if !has_content || args.recurse {
                    return None;
                }
                let kind = if args.append { OpKind::Append } else { OpKind::Write };
                self.plan.ops.push(Op { kind, paths: vec![args.path?] });
            }
            "add-content" => {
                if !has_content {
                    return None;
                }
                self.plan.ops.push(Op { kind: OpKind::Append, paths: vec![args.path?] });
            }
            "get-content" | "test-path" => {
                if piped.is_some() {
                    return None;
                }
                self.plan.ops.push(Op { kind: OpKind::Read, paths: vec![args.path?] });
            }
            "remove-item" => {
                if args.recurse || piped.is_some() {
                    return None;
                }
                self.plan.ops.push(Op { kind: OpKind::Delete, paths: vec![args.path?] });
            }
            "new-item" => {
                if args.recurse || piped.is_some() || args.value.is_some() {
                    return None;
                }
                let dir = args.item_type.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("directory"));
                self.plan.ops.push(Op { kind: if dir { OpKind::MakeDir } else { OpKind::Write }, paths: vec![args.path?] });
            }
            "copy-item" | "move-item" => {
                if args.recurse || piped.is_some() {
                    return None;
                }
                let kind = if lower == "copy-item" { OpKind::Copy } else { OpKind::Move };
                self.plan.ops.push(Op { kind, paths: vec![args.path?, args.second_path?] });
            }
            "write-output" | "write-host" => {}
            _ => return None,
        }
        if !self.at_end() {
            return None;
        }
        Some(())
    }

    /// Аргументы командлета до `;`, `|`, `)` или конца. Значения без ключа занимают места по порядку
    /// (путь, затем значение или назначение), пропуская те, что заданы ключом.
    fn cmd_args(&mut self, cmdlet: &str) -> Option<Args> {
        let mut args = Args::default();
        let mut positional: Vec<Val> = Vec::new();
        loop {
            match self.peek() {
                None | Some(Tok::P(';')) | Some(Tok::P('|')) | Some(Tok::P(')')) => break,
                Some(Tok::Word(w)) if w.starts_with('-') && w.len() > 1 && !w.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) => {
                    let name = w.to_lowercase();
                    self.at += 1;
                    match name.as_str() {
                        "-path" | "-literalpath" | "-filepath" => args.path = Some(self.arg_path()?),
                        "-destination" => args.second_path = Some(self.arg_path()?),
                        "-value" | "-inputobject" => args.value = Some(self.primary()?),
                        "-encoding" => {
                            self.primary_or_word()?;
                        }
                        "-itemtype" => {
                            let Some(Tok::Word(t)) = self.peek().cloned() else { return None };
                            self.at += 1;
                            args.item_type = Some(t);
                        }
                        "-nonewline" | "-force" | "-raw" => {}
                        "-append" => args.append = true,
                        "-recurse" => args.recurse = true,
                        _ => return None,
                    }
                }
                _ => positional.push(self.primary_or_word()?),
            }
        }
        let output_only = matches!(cmdlet, "write-output" | "write-host");
        let mut slots: Vec<&str> = Vec::new();
        if !output_only {
            if args.path.is_none() {
                slots.push("path");
            }
            if matches!(cmdlet, "set-content" | "add-content" | "out-file") && args.value.is_none() {
                slots.push("value");
            }
            if matches!(cmdlet, "copy-item" | "move-item") && args.second_path.is_none() {
                slots.push("dest");
            }
        }
        if positional.len() > slots.len() && !output_only {
            return None;
        }
        for (slot, val) in slots.iter().zip(positional) {
            match *slot {
                "path" => args.path = Some(val.lit?),
                "value" => args.value = Some(val),
                _ => args.second_path = Some(val.lit?),
            }
        }
        Some(args)
    }

    fn arg_path(&mut self) -> Option<String> {
        self.primary_or_word()?.lit
    }

    fn primary_or_word(&mut self) -> Option<Val> {
        if let Some(Tok::Word(w)) = self.peek().cloned() {
            if !w.starts_with('-') {
                self.at += 1;
                return Some(Val { lit: Some(w) });
            }
        }
        self.primary()
    }

    /// Значение аргумента в командном режиме: строка, переменная, `( выражение )`, метод.
    fn primary(&mut self) -> Option<Val> {
        match self.peek()? {
            Tok::Str(_) | Tok::Var(_) | Tok::P('(') | Tok::Static(..) => self.term(),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Args {
    path: Option<String>,
    second_path: Option<String>,
    value: Option<Val>,
    item_type: Option<String>,
    append: bool,
    recurse: bool,
}

fn is_cmdlet(lower: &str) -> bool {
    matches!(
        lower,
        "set-content" | "out-file" | "add-content" | "get-content" | "test-path" | "remove-item" | "new-item" | "copy-item" | "move-item"
            | "write-output" | "write-host"
    )
}

/// Строгий разбор. `None` — скрипт не из безобидных кирпичей, решать по нему нельзя.
pub fn ps_plan(script: &str) -> Option<Plan> {
    let toks = tokenize(script)?;
    if toks.is_empty() {
        return None;
    }
    let mut parser = Parser { toks: &toks, at: 0, vars: HashMap::new(), plan: Plan::default() };
    loop {
        parser.statement()?;
        if parser.at >= toks.len() {
            break;
        }
        if !parser.eat(';') {
            return None;
        }
        if parser.at >= toks.len() {
            break;
        }
    }
    // Скрипт без единого файлового шага ничего не меняет, но и в карточке спорить не о чем: решение за вызывающим.
    Some(parser.plan)
}

/// Путь по тексту, без обращения к диску: то, что нельзя сравнить с папками честно (подстановки, групповые знаки,
/// потоки `file:stream`, короткие имена 8.3, хвостовые точки и пробелы, устройства, сетевые и служебные префиксы).
pub fn suspicious_path(raw: &str) -> bool {
    if raw.trim().is_empty() || raw.chars().count() > 260 {
        return true;
    }
    if raw.chars().any(|c| matches!(c, '%' | '$' | '`' | '*' | '?' | '"' | '<' | '>' | '|' | '[' | ']' | '{' | '}' | '^' | '!' | '\n' | '\r' | '\0')) {
        return true;
    }
    if has_lookalikes(raw) || raw.starts_with('~') || raw.starts_with("\\\\") || raw.starts_with("//") {
        return true;
    }
    // Двоеточие допустимо только в «C:» в начале.
    let rest = match raw.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => &raw[2..],
        _ => raw,
    };
    if rest.contains(':') {
        return true;
    }
    for part in raw.split(['\\', '/']) {
        if part.is_empty() {
            continue;
        }
        if part == ".." || part.ends_with('.') && part != "." || part.ends_with(' ') || part.starts_with(' ') {
            return true;
        }
        let stem = part.split('.').next().unwrap_or_default().to_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit()) {
            return true;
        }
        // Короткое имя 8.3: `PROGRA~1`.
        if let Some(tilde) = part.find('~') {
            if part[tilde + 1..].chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// Скрипт из командной строки для `cmd /c`: `pwsh [ключи] -Command <скрипт>`. Учитывает, что cmd видит строку
/// раньше PowerShell: `& | < > ^ %` вне двойных кавычек исполнил бы сам cmd, а перевод строки обрывает команду.
pub fn script_from_cmdline(command: &str) -> Option<String> {
    script_from_cmdline_for(command, cfg!(windows))
}

/// Разбор рассчитан на `cmd.exe /d /s /c`. На других системах команда уходит в `/bin/sh -c`, где `$(…)`, обратные
/// кавычки, `$VAR`, `!` и кавычки раскрываются раньше PowerShell: там доверять разбору нельзя вовсе.
/// Программа — только голое `pwsh` или `powershell`: путь и расширение дали бы запуск подложенного файла.
pub fn script_from_cmdline_for(command: &str, windows: bool) -> Option<String> {
    if !windows {
        return None;
    }
    let trimmed = command.trim();
    let (exe, rest) = trimmed.split_once(char::is_whitespace)?;
    if !matches!(exe.to_lowercase().as_str(), "pwsh" | "powershell") {
        return None;
    }
    if trimmed.contains(['\n', '\r', '%', '^', '\0']) {
        return None;
    }
    // Ключи до -Command: только известные безобидные.
    let mut rest = rest.trim_start();
    loop {
        let (word, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let lower = word.to_lowercase();
        match lower.as_str() {
            "-noprofile" | "-nonprofile" | "-noni" | "-noninteractive" | "-nologo" | "-nol" | "-nop" => rest = tail.trim_start(),
            "-executionpolicy" | "-ep" => {
                let (value, tail) = tail.trim_start().split_once(char::is_whitespace).unwrap_or((tail.trim_start(), ""));
                if !value.chars().all(|c| c.is_ascii_alphabetic()) {
                    return None;
                }
                rest = tail.trim_start();
            }
            "-command" | "-c" => {
                rest = tail.trim();
                break;
            }
            _ => return None,
        }
    }
    if rest.is_empty() {
        return None;
    }
    if let Some(inner) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        // Одна пара кавычек; обратная косая перед закрывающей кавычкой экранировала бы её.
        if inner.contains('"') || inner.ends_with('\\') {
            return None;
        }
        return Some(inner.to_string());
    }
    // Одинарные кавычки cmd не знает: pwsh получит их как часть текста и разберёт `'...'` как строку, а не как скрипт.
    // Поэтому годится только форма в двойных кавычках: тогда pwsh получает ровно то, что внутри.
    None
}

/// Скрипт из аргументов процесса (так запускает ACP-агент): `[pwsh, ключи…, -Command, скрипт]`.
pub fn script_from_argv(argv: &[String]) -> Option<String> {
    let exe = argv.first()?;
    // Только имя без пути (`pwsh`, `powershell.exe`): путь к подложенному файлу не принимаем.
    if !matches!(exe.to_lowercase().as_str(), "pwsh" | "pwsh.exe" | "powershell" | "powershell.exe") {
        return None;
    }
    let mut i = 1;
    while i < argv.len() {
        let lower = argv[i].to_lowercase();
        match lower.as_str() {
            "-noprofile" | "-noni" | "-noninteractive" | "-nologo" => i += 1,
            "-executionpolicy" | "-ep" => i += 2,
            "-command" | "-c" => {
                return (argv.len() == i + 2).then(|| argv[i + 1].clone());
            }
            _ => return None,
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Терпимый просмотр для текста карточки
// ---------------------------------------------------------------------------

/// Что нашёл просмотр; решений по нему не принимается, только слова в карточке.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub writes: Vec<String>,
    pub deletes: Vec<String>,
    pub reads: Vec<String>,
    pub hosts: Vec<String>,
    pub installs: Vec<String>,
    pub runs: Vec<String>,
    pub encoded: bool,
    pub eval: bool,
}

/// Текст для людей без невидимых и переключающих направление знаков (RLO, нулевая ширина, типографские кавычки):
/// иначе имя файла в карточке можно записать наоборот или спрятать часть.
pub fn plain_text(text: &str) -> String {
    text.chars().filter(|c| !has_lookalikes(&c.to_string()) || matches!(c, '\n')).collect()
}

/// Тот же текст, но подозрительные знаки видны кодом `⟦U+202E⟧`: для технических деталей.
pub fn reveal_odd(text: &str) -> String {
    text.chars()
        .map(|c| if has_lookalikes(&c.to_string()) && !matches!(c, '\n' | '\r' | '\t') { format!("⟦U+{:04X}⟧", c as u32) } else { c.to_string() })
        .collect()
}

/// Все строки в кавычках, в порядке появления.
fn quoted_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\'' || chars[i] == '"' {
            let q = chars[i];
            let mut j = i + 1;
            let mut s = String::new();
            while j < chars.len() && chars[j] != q {
                s.push(chars[j]);
                j += 1;
            }
            out.push(s);
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn looks_like_path(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() < 260
        && !s.contains('\n')
        && (!s.contains(' ') || s.contains(":\\"))
        && (s.contains('\\') || s.contains('/') || s.contains('.') && s.rsplit('.').next().is_some_and(|e| (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric())))
        && !s.starts_with("http")
}

pub fn scan(text: &str) -> Scan {
    let mut out = Scan::default();
    let lower = text.to_lowercase();
    let has = |needle: &str| lower.contains(needle);
    out.encoded = has("-encodedcommand") || has(" -enc ") || has(" -ec ") || has("frombase64string");
    out.eval = has("invoke-expression") || has("iex ") || has("iex(") || has("[scriptblock]") || has("invoke-command");
    let literals = quoted_literals(text);
    let paths: Vec<String> = literals.iter().filter(|l| looks_like_path(l)).cloned().collect();
    let write_verbs = ["set-content", "add-content", "out-file", "writealltext", "appendalltext", "writealllines", "copy-item", "move-item", "new-item", "rename-item", "tee-object", "export-csv", " > ", ">>"];
    if write_verbs.iter().any(|v| has(v)) {
        out.writes = paths.clone();
    }
    if ["remove-item", "del ", "erase ", "rmdir", "rm ", "remove-", "file]::delete", "rd "].iter().any(|v| has(v)) {
        out.deletes = paths.clone();
    }
    if ["get-content", "readalltext", "readalllines", "select-string", "get-childitem", "type "].iter().any(|v| has(v)) {
        out.reads = paths.clone();
    }
    for verb in ["curl", "wget", "invoke-webrequest", "invoke-restmethod", "iwr ", "irm ", "start-bitstransfer", "webclient", "httpclient", "ssh ", "scp ", "sftp", "test-netconnection", "ftp "] {
        if has(verb) {
            for word in text.split(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '(' | ')' | ',')) {
                if let Some(host) = word.strip_prefix("https://").or_else(|| word.strip_prefix("http://")).or_else(|| word.strip_prefix("ftp://")) {
                    let host = host.split(['/', '?', '#']).next().unwrap_or_default();
                    if !host.is_empty() && !out.hosts.contains(&host.to_string()) {
                        out.hosts.push(host.to_string());
                    }
                }
            }
            if out.hosts.is_empty() {
                out.hosts.push(String::new());
            }
            break;
        }
    }
    for (needle, label) in [
        ("npm install", "npm"), ("npm i ", "npm"), ("npm add", "npm"), ("pnpm add", "pnpm"), ("pnpm install", "pnpm"), ("yarn add", "yarn"),
        ("pip install", "pip"), ("pip3 install", "pip"), ("uv add", "uv"), ("uv pip", "uv"), ("cargo install", "cargo"), ("winget install", "winget"),
        ("choco install", "choco"), ("install-module", "PowerShell"), ("install-package", "PowerShell"), ("dotnet add", "dotnet"), ("dotnet tool install", "dotnet"),
        ("scoop install", "scoop"), ("npx ", "npx"), ("uvx ", "uvx"),
    ] {
        if has(needle) && !out.installs.contains(&label.to_string()) {
            out.installs.push(label.to_string());
        }
    }
    for verb in ["start-process", "saps ", "start ", "invoke-item", "& ", "[diagnostics.process]", "system.diagnostics.process", "wscript", "cscript", "mshta", "rundll32", "regsvr32", "schtasks", "new-service"] {
        if has(verb) {
            out.runs.push(verb.trim().to_string());
        }
    }
    // Программы, через которые скачивают, правят систему или запускают чужое: отдельные слова, а не куски других слов.
    let words: Vec<&str> = lower.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.')).filter(|w| !w.is_empty()).collect();
    for tool in ["certutil", "bitsadmin", "nslookup", "mshta", "rundll32", "regsvr32", "wmic", "schtasks", "reg", "sc", "netsh", "msiexec", "installutil", "cmstp", "forfiles", "pcalua"] {
        if words.iter().any(|w| w == &tool || w.strip_suffix(".exe") == Some(tool)) && !out.runs.contains(&tool.to_string()) {
            out.runs.push(tool.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDITOR: &str = "$p = 'ai-agents-market-map.md'\n$c = [System.IO.File]::ReadAllText($p)\n$c = $c.Replace('старая фраза', 'новая фраза').Replace('a', 'b')\n$add = @'\n## Новое\n- ссылка 1\n'@\n$c = $c + \"`n\" + $add\n[System.IO.File]::WriteAllText($p, $c, (New-Object System.Text.UTF8Encoding $false))";

    #[test]
    fn the_editor_script_is_understood_as_one_read_and_one_write_of_the_same_file() {
        let plan = ps_plan(EDITOR).expect("понят");
        let kinds: Vec<(OpKind, &str)> = plan.ops.iter().map(|o| (o.kind, o.paths[0].as_str())).collect();
        assert_eq!(kinds, vec![(OpKind::Read, "ai-agents-market-map.md"), (OpKind::Write, "ai-agents-market-map.md")]);
        assert_eq!(plan.replaces, 2);
    }

    #[test]
    fn pipelines_and_cmdlets_with_literals_are_understood() {
        let plan = ps_plan("'привет' | Set-Content -LiteralPath 'a.txt' -Encoding UTF8").unwrap();
        assert_eq!(plan.ops[0].kind, OpKind::Write);
        let plan = ps_plan("Add-Content -Path notes.md -Value 'x'; Get-Content notes.md -Raw").unwrap();
        assert_eq!(plan.ops.len(), 2);
    }

    #[test]
    fn everything_else_is_not_understood() {
        for script in [
            "Invoke-Expression 'x'",
            "iex 'x'",
            "& 'calc.exe'",
            "Start-Process calc",
            "$p = $env:TEMP; Set-Content $p 'x'",
            "Set-Content $undefined 'x'",
            "$x = (Get-Date); Set-Content a.txt $x",
            "Get-ChildItem | Remove-Item",
            "Set-Content a.txt 'x'; calc",
            "Set-Content a.txt 'x' > b.txt",
            "Set-Content a.txt 'x' | Out-Null",
            "Set-Content -Path (Join-Path $a 'b') 'x'",
            "Remove-Item -Recurse -Path x",
            "Set-Content a.txt \"$(calc)\"",
            "Set-Content a.txt \"$env:USERNAME\"",
            "$s = @\"\n$(calc)\n\"@",
            "Set-Content a.txt 'x'; Invoke-WebRequest http://x",
            "[System.Diagnostics.Process]::Start('calc')",
            "$f = { calc }; & $f",
            "Set-Content a.txt 'x' -Credential y",
            "Set-Content a.txt `\nx",
            "Set-Content a.txt ‘x’",
            "Set-Content a.txt 'x’; Remove-Item b; ‘'",
            "$a.GetType().Assembly",
            "'x'.GetType()",
            "Set-Content a.txt 'x' # comment",
        ] {
            assert!(ps_plan(script).is_none(), "{script}");
        }
    }

    #[test]
    fn a_here_string_cannot_hide_a_second_command() {
        // Закрытие не в начале строки — не конец строки-блока: скрипт не разбираем.
        assert!(ps_plan("$s = @'\nx\n'@ ; calc\nSet-Content a.txt $s").is_none());
        assert!(ps_plan("$s = @'x\n'@").is_none());
    }

    #[test]
    fn paths_with_tricks_are_suspicious() {
        for raw in [
            "..\\x", "a/../b", "C:\\ws\\..\\x", "\\\\server\\share\\x", "%TEMP%\\x", "$p\\x", "a.txt:stream", "con.txt", "NUL", "PROGRA~1\\x", "dir.\\x",
            "x ", "*.md", "a?.md", "~\\x", "ａ.txt", "a\u{200B}.txt",
        ] {
            assert!(suspicious_path(raw), "{raw}");
        }
        for raw in ["notes.md", "sub\\Файл заметки.md", "C:\\Users\\Public\\x\\a.txt", "docs/a-b_c.md"] {
            assert!(!suspicious_path(raw), "{raw}");
        }
    }

    #[test]
    fn the_command_line_wrappers_respect_what_cmd_would_do() {
        assert_eq!(script_from_cmdline("pwsh -NoProfile -Command \"Set-Content a.txt 'x'\"").as_deref(), Some("Set-Content a.txt 'x'"));
        assert!(script_from_cmdline("pwsh -NoProfile -Command 'Set-Content a.txt x'").is_none(), "в cmd одинарные кавычки не защищают");
        // Программа — голое имя: путь, расширение и кавычки вокруг имени не принимаются.
        for exe in [r"C:\x\pwsh.exe", r".\pwsh", "pwsh.exe", "pwsh.bat", "\"pwsh\"", "powershell.cmd"] {
            assert!(script_from_cmdline(&format!("{exe} -NoProfile -Command \"Set-Content a.txt x\"")).is_none(), "{exe}");
        }
        // Не cmd: `/bin/sh -c` раскрывает `$(…)` до PowerShell, разбор отключён.
        assert!(script_from_cmdline_for("pwsh -NoProfile -Command \"Set-Content a.txt x\"", false).is_none());
        assert!(script_from_cmdline_for("pwsh -NoProfile -Command \"Set-Content a.txt x\"", true).is_some());
        assert_eq!(script_from_cmdline("powershell -c \"Set-Content a.txt 'x'|Out-Null\"").as_deref(), Some("Set-Content a.txt 'x'|Out-Null"));
        for bad in [
            "pwsh -Command 'Set-Content a.txt x' & calc",
            "pwsh -Command 'a|b'",
            "pwsh -Command 'Set-Content a.txt x\nSet-Content b.txt y'",
            "pwsh -Command \"Set-Content a.txt x\" & calc",
            "pwsh -Command \"Set-Content a.txt \"x\"\"",
            "pwsh -Command \"Set-Content C:\\x\\\"",
            "pwsh -Command 'Set-Content %TEMP%\\a x'",
            "pwsh -EncodedCommand AAAA",
            "pwsh -File x.ps1",
            "pwsh -WorkingDirectory C:\\ -Command 'x'",
            "cmd /c pwsh -Command 'x'",
            "pwsh -Command 'a' ^ calc",
        ] {
            assert!(script_from_cmdline(bad).is_none(), "{bad}");
        }
        let argv: Vec<String> = ["pwsh.exe", "-NoProfile", "-Command", "Set-Content a.txt 'x'"].iter().map(|s| s.to_string()).collect();
        assert_eq!(script_from_argv(&argv).as_deref(), Some("Set-Content a.txt 'x'"));
        let argv: Vec<String> = ["pwsh.exe", "-Command", "x", "extra"].iter().map(|s| s.to_string()).collect();
        assert!(script_from_argv(&argv).is_none());
    }

    #[test]
    fn aliases_are_not_cmdlets() {
        for script in ["sc stop X", "sc delete X", "ac a.txt x", "ri a.txt", "ni a.txt", "gc a.txt", "cat a.txt", "type a.txt", "rm a.txt", "del a.txt", "cp a b", "mv a b", "copy a b", "move a b", "erase a"] {
            assert!(ps_plan(script).is_none(), "{script}");
        }
        assert!(ps_plan("Remove-Item -LiteralPath a.txt").is_some());
    }

    #[test]
    fn scan_finds_verbs_for_the_card_text() {
        let s = scan("Invoke-WebRequest 'https://example.com/a?token=1' -OutFile 'x.zip'; Remove-Item 'old.txt'");
        assert_eq!(s.hosts, vec!["example.com"]);
        assert!(s.deletes.contains(&"old.txt".to_string()));
        assert!(scan("pwsh -EncodedCommand AAAA").encoded);
        assert!(scan("npm install left-pad").installs.contains(&"npm".to_string()));
    }
}
