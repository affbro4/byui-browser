//! Public-contract tests for `js`.
//!
//! Run ignored tests with `cargo test -p js -- --ignored` to see the backlog.

use js::{Value, eval, lexer, parse, parser, runtime};

#[test]
fn evaluates_arithmetic() {
    let tokens = lexer::tokenize("42 + 10");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(
        runtime::evaluate(match &ast.body[0] {
            js::Statement::Expression(expr) => expr,
            _ => panic!("expected expression"),
        }),
        Value::Number(52.0)
    );
}

#[test]
fn evaluates_chained_addition() {
    let tokens = lexer::tokenize("42 + 10 + 8");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(
        runtime::evaluate(match &ast.body[0] {
            js::Statement::Expression(expr) => expr,
            _ => panic!("expected expression"),
        }),
        Value::Number(60.0)
    );
}

#[test]
fn evaluates_other_arithmetic_operators() {
    let tokens = lexer::tokenize("42 - 10 * 2 / 4");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(
        runtime::evaluate(match &ast.body[0] {
            js::Statement::Expression(expr) => expr,
            _ => panic!("expected expression"),
        }),
        Value::Number(37.0)
    );
}

#[test]
fn empty_source_parses_to_empty_program() {
    assert_eq!(parse(""), Ok(js::Program::default()));
    assert_eq!(parse("  \n\t"), Ok(js::Program::default()));
}

#[test]
fn empty_program_evaluates_to_undefined() {
    assert_eq!(eval(""), Ok(Value::Undefined));
}

#[test]
#[ignore = "TODO(js): evaluation of nonempty programs not implemented"]
fn evaluates_string_literal() {
    assert_eq!(eval("'hi'"), Ok(Value::String("hi".into())));
}

#[test]
fn syntax_error_is_an_err_not_a_panic() {
    assert!(parse("let = ;").is_err());
}

#[test]
fn nonempty_program_evaluation_returns_an_error() {
    for source in ["42", "let x = 1;", "f();"] {
        assert_eq!(
            eval(source).unwrap_err().message,
            "JavaScript evaluation is not implemented"
        );
    }
}
