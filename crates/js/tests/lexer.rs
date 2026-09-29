use js::lexer::{Token, tokenize};

#[test]
fn lexes_adjacent_numbers_operators_and_identifiers() {
    assert_eq!(
        tokenize("let total=12.5+count;"),
        vec![
            Token::Let,
            Token::Identifier("total".into()),
            Token::Assign,
            Token::Number(12.5),
            Token::Plus,
            Token::Identifier("count".into()),
            Token::Semicolon,
        ]
    );
}

#[test]
fn lexes_keywords_comparisons_and_unknown_characters() {
    assert_eq!(
        tokenize("if (value == true) @"),
        vec![
            Token::If,
            Token::LeftParen,
            Token::Identifier("value".into()),
            Token::EqualEqual,
            Token::True,
            Token::RightParen,
            Token::Unknown('@'),
        ]
    );
}
