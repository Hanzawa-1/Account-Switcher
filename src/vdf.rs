//! A small reader and writer for Valve's text KeyValues format
//! (the .vdf files Steam keeps its settings in).

/// A value is either a string or an ordered list of key/value pairs.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// Looks up a child by key. Steam treats keys case-insensitively, so this does too.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(pairs) => find(pairs, key),
            Value::Str(_) => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            Value::Obj(_) => None,
        }
    }

    /// Replaces the string under `key` (matched case-insensitively), or appends it if missing.
    pub fn set_str(&mut self, key: &str, value: &str) {
        if let Value::Obj(pairs) = self {
            match pairs.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                Some((_, v)) => *v = Value::Str(value.to_owned()),
                None => pairs.push((key.to_owned(), Value::Str(value.to_owned()))),
            }
        }
    }
}

pub fn find<'a>(pairs: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    pairs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

pub fn find_mut<'a>(pairs: &'a mut [(String, Value)], key: &str) -> Option<&'a mut Value> {
    pairs
        .iter_mut()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

pub fn parse(text: &str) -> Result<Vec<(String, Value)>, String> {
    let tokens = tokenize(text)?;
    let mut pos = 0;
    let pairs = parse_pairs(&tokens, &mut pos)?;
    if pos != tokens.len() {
        return Err("unexpected '}'".into());
    }
    Ok(pairs)
}

/// Writes pairs out the way Steam formats them: tab indents, two tabs between key and value.
pub fn write(pairs: &[(String, Value)], newline: &str) -> String {
    let mut out = String::new();
    write_pairs(&mut out, pairs, 0, newline);
    out
}

enum Token {
    Str(String),
    Open,
    Close,
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars: Chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            '{' => {
                chars.next();
                tokens.push(Token::Open);
            }
            '}' => {
                chars.next();
                tokens.push(Token::Close);
            }
            '"' => {
                chars.next();
                tokens.push(Token::Str(read_quoted(&mut chars)?));
            }
            '/' => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    // A comment runs to the end of the line.
                    for ch in chars.by_ref() {
                        if ch == '\n' {
                            break;
                        }
                    }
                } else {
                    tokens.push(Token::Str(format!("/{}", read_bare(&mut chars))));
                }
            }
            c if c.is_whitespace() => {
                chars.next();
            }
            _ => tokens.push(Token::Str(read_bare(&mut chars))),
        }
    }
    Ok(tokens)
}

fn read_quoted(chars: &mut Chars) -> Result<String, String> {
    let mut s = String::new();
    loop {
        match chars.next() {
            Some('"') => return Ok(s),
            Some('\\') => match chars.next() {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some('\\') => s.push('\\'),
                Some('"') => s.push('"'),
                Some(other) => {
                    s.push('\\');
                    s.push(other);
                }
                None => break,
            },
            Some(ch) => s.push(ch),
            None => break,
        }
    }
    Err("unterminated string".into())
}

fn read_bare(chars: &mut Chars) -> String {
    let mut s = String::new();
    while let Some(&ch) = chars.peek() {
        if ch.is_whitespace() || matches!(ch, '{' | '}' | '"') {
            break;
        }
        s.push(ch);
        chars.next();
    }
    s
}

fn parse_pairs(tokens: &[Token], pos: &mut usize) -> Result<Vec<(String, Value)>, String> {
    let mut pairs = Vec::new();
    while let Some(token) = tokens.get(*pos) {
        let key = match token {
            Token::Str(k) => k.clone(),
            Token::Close => break,
            Token::Open => return Err("unexpected '{'".into()),
        };
        *pos += 1;
        let value = match tokens.get(*pos) {
            Some(Token::Str(v)) => {
                *pos += 1;
                Value::Str(v.clone())
            }
            Some(Token::Open) => {
                *pos += 1;
                let children = parse_pairs(tokens, pos)?;
                match tokens.get(*pos) {
                    Some(Token::Close) => *pos += 1,
                    _ => return Err(format!("missing '}}' after \"{key}\"")),
                }
                Value::Obj(children)
            }
            _ => return Err(format!("missing value for \"{key}\"")),
        };
        pairs.push((key, value));
    }
    Ok(pairs)
}

fn write_pairs(out: &mut String, pairs: &[(String, Value)], depth: usize, nl: &str) {
    let indent = "\t".repeat(depth);
    for (key, value) in pairs {
        match value {
            Value::Str(s) => {
                out.push_str(&format!(
                    "{indent}\"{}\"\t\t\"{}\"{nl}",
                    escape(key),
                    escape(s)
                ));
            }
            Value::Obj(children) => {
                out.push_str(&format!("{indent}\"{}\"{nl}{indent}{{{nl}", escape(key)));
                write_pairs(out, children, depth + 1, nl);
                out.push_str(&format!("{indent}}}{nl}"));
            }
        }
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\"users\"\n{\n\t\"76561198000000001\"\n\t{\n\t\t\"AccountName\"\t\t\"main_acc\"\n\t\t\"PersonaName\"\t\t\"Say \\\"hi\\\" \\\\o/\"\n\t\t\"AutoLogin\"\t\t\"1\"\n\t\t\"timestamp\"\t\t\"1700000000\"\n\t}\n}\n";

    #[test]
    fn roundtrip_is_byte_identical() {
        let parsed = parse(SAMPLE).unwrap();
        assert_eq!(write(&parsed, "\n"), SAMPLE);
    }

    #[test]
    fn unescapes_and_ignores_key_case() {
        let parsed = parse(SAMPLE).unwrap();
        let user = find(&parsed, "USERS")
            .unwrap()
            .get("76561198000000001")
            .unwrap();
        assert_eq!(user.get_str("personaname"), Some("Say \"hi\" \\o/"));
        assert_eq!(user.get_str("Timestamp"), Some("1700000000"));
    }

    #[test]
    fn skips_comments_and_reads_bare_words() {
        let parsed = parse("// note\nroot { key value }").unwrap();
        assert_eq!(find(&parsed, "root").unwrap().get_str("key"), Some("value"));
    }

    #[test]
    fn rejects_broken_files() {
        assert!(parse("\"a\" { \"b\" \"c\"").is_err());
        assert!(parse("\"a\"").is_err());
        assert!(parse("\"a\" \"b\" }").is_err());
    }

    /// Run with `SSS_LOGINUSERS=<copy of loginusers.vdf> cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs SSS_LOGINUSERS pointing at a copy of a real loginusers.vdf"]
    fn real_file_roundtrip() {
        let path = std::env::var("SSS_LOGINUSERS").expect("SSS_LOGINUSERS is not set");
        let text = std::fs::read_to_string(path).unwrap();
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let parsed = parse(&text).unwrap();
        let written = write(&parsed, newline);
        assert_eq!(parse(&written).unwrap(), parsed);
        println!("byte-identical after rewrite: {}", written == text);
    }
}
