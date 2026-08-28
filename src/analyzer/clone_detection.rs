use std::hash::{DefaultHasher, Hash, Hasher};

use crate::model::{CloneCandidate, Language};

const MIN_CLONE_LINES: usize = 8;
const MIN_CLONE_TOKENS: usize = 60;

pub(super) fn candidate(
    source: &str,
    language: Language,
    line: usize,
    end_line: usize,
) -> Option<CloneCandidate> {
    if end_line.saturating_sub(line) + 1 < MIN_CLONE_LINES {
        return None;
    }
    let (normalized, tokens) = normalize(source, language);
    if tokens < MIN_CLONE_TOKENS {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    language.hash(&mut hasher);
    normalized.hash(&mut hasher);
    Some(CloneCandidate {
        fingerprint: hasher.finish(),
        tokens,
        line,
        end_line,
    })
}

fn normalize(source: &str, language: Language) -> (String, usize) {
    let mut scanner = Normalizer::new(source, language);
    let mut normalized = String::with_capacity(source.len() / 2);
    let mut tokens = 0;
    while scanner.has_more() {
        scanner.consume(&mut normalized, &mut tokens);
    }
    (normalized, tokens)
}

struct Normalizer {
    characters: Vec<char>,
    index: usize,
    language: Language,
}

impl Normalizer {
    fn new(source: &str, language: Language) -> Self {
        Self {
            characters: source.chars().collect(),
            index: 0,
            language,
        }
    }

    fn has_more(&self) -> bool {
        self.index < self.characters.len()
    }

    fn consume(&mut self, output: &mut String, tokens: &mut usize) {
        let current = self.characters[self.index];
        match current {
            character if character.is_whitespace() => self.index += 1,
            '/' if self.next_is('/') => {
                self.index = skip_line_comment(&self.characters, self.index + 2);
            }
            '/' if self.next_is('*') => {
                self.index = skip_block_comment(&self.characters, self.index + 2);
            }
            character if self.at_quoted_literal(character) => {
                self.index = skip_quoted(&self.characters, self.index + 1, character);
                push_token(output, "L", tokens);
            }
            character if is_identifier_start(character) => self.consume_word(output, tokens),
            character if character.is_ascii_digit() => {
                self.index = literal_end(&self.characters, self.index + 1);
                push_token(output, "L", tokens);
            }
            character => {
                output.push(character);
                *tokens += 1;
                self.index += 1;
            }
        }
    }

    fn consume_word(&mut self, output: &mut String, tokens: &mut usize) {
        let end = identifier_end(&self.characters, self.index + 1);
        let word = self.characters[self.index..end].iter().collect::<String>();
        let token = normalized_word(&word, self.language);
        push_token(output, token, tokens);
        self.index = end;
    }

    fn next_is(&self, expected: char) -> bool {
        self.characters.get(self.index + 1) == Some(&expected)
    }

    fn at_rust_lifetime(&self) -> bool {
        is_rust_lifetime(&self.characters, self.index, self.language)
    }

    fn at_quoted_literal(&self, character: char) -> bool {
        is_quote(character) && !self.at_rust_lifetime()
    }
}

fn normalized_word(word: &str, language: Language) -> &str {
    if is_word_literal(word, language) {
        "L"
    } else if is_keyword(word, language) {
        word
    } else {
        "I"
    }
}

fn skip_line_comment(characters: &[char], mut index: usize) -> usize {
    while characters
        .get(index)
        .is_some_and(|character| *character != '\n')
    {
        index += 1;
    }
    index
}

fn skip_block_comment(characters: &[char], mut index: usize) -> usize {
    while index + 1 < characters.len() {
        if characters[index] == '*' && characters[index + 1] == '/' {
            return index + 2;
        }
        index += 1;
    }
    characters.len()
}

fn skip_quoted(characters: &[char], mut index: usize, quote: char) -> usize {
    while index < characters.len() {
        match characters[index] {
            '\\' => index += 2,
            character if character == quote => return index + 1,
            _ => index += 1,
        }
    }
    characters.len()
}

fn is_quote(character: char) -> bool {
    matches!(character, '\'' | '"' | '`')
}

fn is_rust_lifetime(characters: &[char], index: usize, language: Language) -> bool {
    language == Language::Rust
        && characters.get(index) == Some(&'\'')
        && characters
            .get(index + 1)
            .is_some_and(|character| is_identifier_start(*character))
        && characters.get(index + 2) != Some(&'\'')
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character == '$' || character.is_alphabetic()
}

fn identifier_end(characters: &[char], mut index: usize) -> usize {
    while characters.get(index).is_some_and(|character| {
        *character == '_' || *character == '$' || character.is_alphanumeric()
    }) {
        index += 1;
    }
    index
}

fn literal_end(characters: &[char], mut index: usize) -> usize {
    while characters.get(index).is_some_and(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '.')
    }) {
        index += 1;
    }
    index
}

fn push_token(output: &mut String, token: &str, count: &mut usize) {
    output.push_str(token);
    output.push(';');
    *count += 1;
}

fn is_keyword(word: &str, language: Language) -> bool {
    match language {
        Language::TypeScript => TS_KEYWORDS.contains(&word),
        Language::Rust => RUST_KEYWORDS.contains(&word),
    }
}

fn is_word_literal(word: &str, language: Language) -> bool {
    match language {
        Language::TypeScript => matches!(word, "false" | "null" | "true" | "undefined"),
        Language::Rust => matches!(word, "false" | "true"),
    }
}

const TS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_ignores_names_and_literal_values() {
        let (first, _) = normalize(
            "function price(order: Order) { return order.total > 10 ? 'large' : false; }",
            Language::TypeScript,
        );
        let (second, _) = normalize(
            "function cost(invoice: Invoice) { return invoice.amount > 500 ? 'high' : true; }",
            Language::TypeScript,
        );
        assert_eq!(first, second);
    }

    #[test]
    fn normalization_preserves_control_operators() {
        let (greater, _) = normalize("fn check(value: i32) { value > 10 }", Language::Rust);
        let (less, _) = normalize("fn check(value: i32) { value < 10 }", Language::Rust);
        assert_ne!(greater, less);
    }
}
