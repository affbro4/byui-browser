/// A token produced by the lexer.
#[derive(Debug, PartialEq)]
pub enum Token {
    /// Numeric literal.
    Number(f64),
    /// Identifier.
    Identifier(String),
    /// `+` operator.
    Plus,
    /// `-` operator.
    Subtract,
    /// `*` operator.
    Multiply,
    /// `/` operator.
    Divide,
    /// `let` keyword.
    Let,
    /// `const` keyword.
    Const,
    /// `if` keyword.
    If,
    /// `else` keyword.
    Else,
    /// `function` keyword.
    Function,
    /// `return` keyword.
    Return,
    /// `true` literal.
    True,
    /// `false` literal.
    False,
    /// `null` literal.
    Null,
    /// `=` operator.
    Assign,
    /// `;` terminator.
    Semicolon,
    /// `(` delimiter.
    LeftParen,
    /// `)` delimiter.
    RightParen,
    /// `==` operator.
    EqualEqual,
    /// `<` operator.
    LessThan,
    /// `>` operator.
    GreaterThan,
    /// Unrecognized character.
    Unknown(char),
}

/// Converts JavaScript source text into tokens, preserving unsupported
/// characters as [`Token::Unknown`].
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        if ch.is_ascii_digit() {
            let start = chars.peek().expect("peeked character exists").0;
            let mut end = start;
            let mut seen_decimal_point = false;

            while let Some((index, next)) = chars.peek().copied() {
                if next.is_ascii_digit() {
                    end = index + next.len_utf8();
                    chars.next();
                } else if next == '.' && !seen_decimal_point {
                    seen_decimal_point = true;
                    end = index + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }

            let number_text = &input[start..end];
            if let Ok(number) = number_text.parse::<f64>() {
                tokens.push(Token::Number(number));
            } else {
                tokens.extend(number_text.chars().map(Token::Unknown));
            }
            continue;
        }

        if is_identifier_start(ch) {
            let start = chars.peek().expect("peeked character exists").0;
            let mut end = start;

            while let Some((index, next)) = chars.peek().copied() {
                if is_identifier_continue(next) {
                    end = index + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }

            let identifier = &input[start..end];
            tokens.push(match identifier {
                "let" => Token::Let,
                "const" => Token::Const,
                "if" => Token::If,
                "else" => Token::Else,
                "function" => Token::Function,
                "return" => Token::Return,
                "true" => Token::True,
                "false" => Token::False,
                "null" => Token::Null,
                _ => Token::Identifier(identifier.to_owned()),
            });
            continue;
        }

        chars.next();
        let token = match ch {
            '+' => Token::Plus,
            '-' => Token::Subtract,
            '*' => Token::Multiply,
            '/' => Token::Divide,
            '=' if chars.peek().is_some_and(|(_, next)| *next == '=') => {
                chars.next();
                Token::EqualEqual
            }
            '=' => Token::Assign,
            ';' => Token::Semicolon,
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '<' => Token::LessThan,
            '>' => Token::GreaterThan,
            other => Token::Unknown(other),
        };
        tokens.push(token);
    }

    tokens
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}
