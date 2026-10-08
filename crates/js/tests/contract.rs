//! Public-contract tests for `js`.
//!
//! Run ignored tests with `cargo test -p js -- --ignored` to see the backlog.

use js::{Value, eval, lexer, parse, parser, runtime};

#[test]
fn evaluates_arithmetic() {
    let tokens = lexer::tokenize("42 + 10");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Value::Number(52.0));
}

#[test]
fn evaluates_chained_addition() {
    let tokens = lexer::tokenize("42 + 10 + 8");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Value::Number(60.0));
}

#[test]
fn evaluates_other_arithmetic_operators() {
    let tokens = lexer::tokenize("42 - 10 * 2 / 4");
    let ast = parser::parse(&tokens).expect("expression should parse");

    assert_eq!(runtime::evaluate(&ast), Value::Number(37.0));
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
fn evaluates_string_literal() {
    assert_eq!(eval("'hi'"), Ok(Value::String("hi".into())));
}

#[test]
fn syntax_error_is_an_err_not_a_panic() {
    assert!(parse("let = ;").is_err());
}

#[test]
fn evaluates_declaration_read_and_assignment_from_source() {
    assert_eq!(
        eval("let count = 1; count = count + 2; count"),
        Ok(Value::Number(3.0))
    );
}

#[test]
fn nested_blocks_shadow_without_mutating_the_outer_binding() {
    assert_eq!(
        eval("let value = 1; { let value = 2; value = 3; } value"),
        Ok(Value::Number(1.0))
    );
}

#[test]
fn invalid_bindings_return_errors() {
    assert!(eval("missing").is_err());
    assert!(eval("const answer = 42; answer = 7").is_err());
    assert!(eval("let answer = 1; let answer = 2").is_err());
}

#[test]
fn executes_only_selected_branch() {
    assert_eq!(
        eval("let x = 0; if (1 < 2) { x = 4; } else { x = 9; } x"),
        Ok(Value::Number(4.0))
    );
}

#[test]
fn while_loop_supports_assignment_progress() {
    assert_eq!(
        eval("let i = 0; while (i < 5) { i = i + 1; } i"),
        Ok(Value::Number(5.0))
    );
}

#[test]
fn nested_calls_bind_parameters_and_propagate_return() {
    assert_eq!(
        eval(
            "function add(a, b) { return a + b; } function twice(x) { return add(x, x); } twice(7)"
        ),
        Ok(Value::Number(14.0))
    );
}

#[test]
fn recursive_calls_have_independent_local_scopes() {
    assert_eq!(
        eval("function fact(n) { if (n <= 1) { return 1; } return n * fact(n - 1); } fact(6)"),
        Ok(Value::Number(720.0))
    );
}

#[test]
fn bad_calls_and_excessive_recursion_return_errors() {
    assert!(eval("function f(a) { return a; } f()").is_err());
    assert!(eval("notCallable()").is_err());
    assert!(
        eval("function loop() { return loop(); } loop()")
            .unwrap_err()
            .message
            .contains("recursion")
    );
}
