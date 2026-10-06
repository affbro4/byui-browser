use js::{Value, eval};

#[test]
fn if_else_executes_only_the_selected_branch() {
    assert_eq!(
        eval("let result = 0; if (true) { result = 1; } else { missing; } result"),
        Ok(Value::Number(1.0))
    );
    assert_eq!(
        eval("let result = 0; if (false) { missing; } else { result = 2; } result"),
        Ok(Value::Number(2.0))
    );
}

#[test]
fn while_loop_supports_assignment_based_progress() {
    assert_eq!(
        eval("let i = 0; let sum = 0; while (i < 5) { sum = sum + i; i = i + 1; } sum"),
        Ok(Value::Number(10.0))
    );
}

#[test]
fn functions_bind_parameters_and_return_values() {
    assert_eq!(
        eval("function add(a, b) { return a + b; } add(2, 3)"),
        Ok(Value::Number(5.0))
    );
}

#[test]
fn nested_calls_and_local_call_scopes_work() {
    assert_eq!(
        eval(
            "let value = 9; \
             function twice(x) { let value = x * 2; return value; } \
             function add(a, b) { return a + b; } \
             add(twice(2), twice(3)) + value"
        ),
        Ok(Value::Number(19.0))
    );
}

#[test]
fn calls_do_not_read_the_callers_local_bindings() {
    assert_eq!(
        eval(
            "let value = 1; \
             function readValue() { return value; } \
             function caller() { let value = 9; return readValue(); } \
             caller()"
        ),
        Ok(Value::Number(1.0))
    );
}

#[test]
fn return_exits_the_function_and_preserves_caller_execution() {
    assert_eq!(
        eval(
            "function inner() { return 2; 30; } \
             function outer() { let local = inner(); return local + 1; 40; } \
             outer()"
        ),
        Ok(Value::Number(3.0))
    );
}

#[test]
fn return_propagates_out_of_a_loop() {
    assert_eq!(
        eval(
            "function firstMatch() { \
                 let i = 0; \
                 while (i < 4) { \
                     i = i + 1; \
                     if (i == 2) { return i; } \
                 } \
                 return 0; \
             } firstMatch()"
        ),
        Ok(Value::Number(2.0))
    );
}

#[test]
fn recursive_function_returns_value() {
    assert_eq!(
        eval(
            "function factorial(n) { \
                 if (n <= 1) { return 1; } \
                 return n * factorial(n - 1); \
             } factorial(5)"
        ),
        Ok(Value::Number(120.0))
    );
}

#[test]
fn invalid_calls_and_wrong_arity_return_errors() {
    assert!(eval("function one(a) { return a; } one()").is_err());
    assert!(eval("let value = 3; value()").is_err());
    assert!(eval("missing()").is_err());
}

#[test]
fn excessive_recursion_returns_an_error() {
    let error = eval("function loop() { return loop(); } loop()")
        .expect_err("recursive calls should be bounded");
    assert!(error.message.contains("maximum function call depth"));
}

#[test]
fn return_outside_function_returns_an_error() {
    assert!(eval("return 1;").is_err());
}
