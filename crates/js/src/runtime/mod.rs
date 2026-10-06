use std::collections::HashMap;

use crate::ast::{
    BinaryOperator, Expr, LogicalOperator, Program, Statement, UnaryOperator, VarKind,
};
use crate::{JsError, JsResult, Value};

const MAX_CALL_DEPTH: usize = 128;

#[derive(Clone, Debug)]
struct Binding {
    value: Value,
    mutable: bool,
}

#[derive(Clone, Debug)]
struct UserFunction {
    params: Vec<String>,
    body: Vec<Statement>,
}

#[derive(Clone, Debug, Default)]
struct Scope {
    bindings: HashMap<String, Binding>,
    functions: HashMap<String, UserFunction>,
}

/// A chain of lexical environments used while executing a JavaScript program.
#[derive(Debug, Default)]
pub struct Environment {
    scopes: Vec<Scope>,
}

impl Environment {
    /// Creates an environment with one global lexical scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::default()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn declare(&mut self, name: &str, value: Value, kind: VarKind) -> JsResult<()> {
        let scope = self
            .scopes
            .last_mut()
            .ok_or_else(|| JsError::new("environment has no active scope"))?;
        if scope.bindings.contains_key(name) || scope.functions.contains_key(name) {
            return Err(JsError::new(format!("`{name}` has already been declared")));
        }
        scope.bindings.insert(
            name.to_owned(),
            Binding {
                value,
                mutable: !matches!(kind, VarKind::Const),
            },
        );
        Ok(())
    }

    fn declare_function(&mut self, name: &str, function: UserFunction) -> JsResult<()> {
        let scope = self
            .scopes
            .last_mut()
            .ok_or_else(|| JsError::new("environment has no active scope"))?;
        if scope.bindings.contains_key(name) || scope.functions.contains_key(name) {
            return Err(JsError::new(format!("`{name}` has already been declared")));
        }
        scope.functions.insert(name.to_owned(), function);
        Ok(())
    }

    /// Reads the nearest binding with `name`.
    pub fn get(&self, name: &str) -> JsResult<Value> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(name))
            .map(|binding| binding.value.clone())
            .ok_or_else(|| JsError::new(format!("`{name}` is not defined")))
    }

    /// Updates the nearest mutable binding with `name`.
    pub fn set(&mut self, name: &str, value: Value) -> JsResult<Value> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(binding) = scope.bindings.get_mut(name) {
                if !binding.mutable {
                    return Err(JsError::new(format!("Assignment to constant `{name}`")));
                }
                binding.value = value.clone();
                return Ok(value);
            }
        }
        Err(JsError::new(format!("`{name}` is not defined")))
    }

    fn resolve_function(&self, name: &str) -> JsResult<UserFunction> {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.bindings.get(name) {
                return Err(JsError::new(format!(
                    "value {:?} is not callable",
                    binding.value
                )));
            }
            if let Some(function) = scope.functions.get(name) {
                return Ok(function.clone());
            }
        }
        Err(JsError::new(format!("`{name}` is not a function")))
    }
}

/// Evaluates a parsed program with a fresh lexical environment.
///
/// Named functions have isolated call scopes and can call global functions,
/// including themselves recursively. Function values and closures are not
/// represented by this AST subset.
pub fn evaluate_program(program: &Program) -> JsResult<Value> {
    let mut environment = Environment::new();
    match execute_statements(&mut environment, &program.body, false, 0)? {
        Completion::Normal(value) => Ok(value),
        Completion::Return(_) => Err(JsError::new("return is only valid inside a function")),
    }
}

#[derive(Debug)]
enum Completion {
    Normal(Value),
    Return(Value),
}

fn execute_statements(
    environment: &mut Environment,
    statements: &[Statement],
    in_function: bool,
    call_depth: usize,
) -> JsResult<Completion> {
    let mut result = Value::Undefined;
    for statement in statements {
        match execute_statement(environment, statement, in_function, call_depth)? {
            Completion::Normal(value) => result = value,
            returned @ Completion::Return(_) => return Ok(returned),
        }
    }
    Ok(Completion::Normal(result))
}

fn execute_statement(
    environment: &mut Environment,
    statement: &Statement,
    in_function: bool,
    call_depth: usize,
) -> JsResult<Completion> {
    match statement {
        Statement::Expression(expression) => Ok(Completion::Normal(evaluate_in(
            expression,
            environment,
            call_depth,
        )?)),
        Statement::VariableDeclaration { kind, name, init } => {
            let value = init.as_ref().map_or(Ok(Value::Undefined), |expression| {
                evaluate_in(expression, environment, call_depth)
            })?;
            environment.declare(name, value, *kind)?;
            Ok(Completion::Normal(Value::Undefined))
        }
        Statement::Block(statements) => {
            environment.push_scope();
            let result = execute_statements(environment, statements, in_function, call_depth);
            environment.pop_scope();
            result
        }
        Statement::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let condition = evaluate_in(condition, environment, call_depth)?;
            let selected = if is_truthy(&condition) {
                Some(then_branch.as_ref())
            } else {
                else_branch.as_deref()
            };
            if let Some(branch) = selected {
                execute_statement(environment, branch, in_function, call_depth)
            } else {
                Ok(Completion::Normal(Value::Undefined))
            }
        }
        Statement::While { condition, body } => {
            let mut result = Value::Undefined;
            loop {
                let condition = evaluate_in(condition, environment, call_depth)?;
                if !is_truthy(&condition) {
                    break;
                }
                match execute_statement(environment, body, in_function, call_depth)? {
                    Completion::Normal(value) => result = value,
                    returned @ Completion::Return(_) => return Ok(returned),
                }
            }
            Ok(Completion::Normal(result))
        }
        Statement::FunctionDeclaration { name, params, body } => {
            environment.declare_function(
                name,
                UserFunction {
                    params: params.clone(),
                    body: body.clone(),
                },
            )?;
            Ok(Completion::Normal(Value::Undefined))
        }
        Statement::Return(value) => {
            if !in_function {
                return Err(JsError::new("return is only valid inside a function"));
            }
            let value = value.as_ref().map_or(Ok(Value::Undefined), |expression| {
                evaluate_in(expression, environment, call_depth)
            })?;
            Ok(Completion::Return(value))
        }
    }
}

fn evaluate_in(
    expression: &Expr,
    environment: &mut Environment,
    call_depth: usize,
) -> JsResult<Value> {
    match expression {
        Expr::Identifier(name) => environment.get(name),
        Expr::Assign { name, value } => {
            let value = evaluate_in(value, environment, call_depth)?;
            environment.set(name, value)
        }
        Expr::Number(number) => Ok(Value::Number(*number)),
        Expr::String(text) => Ok(Value::String(text.clone())),
        Expr::Boolean(value) => Ok(Value::Boolean(*value)),
        Expr::Null => Ok(Value::Null),
        Expr::Undefined => Ok(Value::Undefined),
        Expr::Unary { operator, operand } => {
            let value = evaluate_in(operand, environment, call_depth)?;
            Ok(match operator {
                UnaryOperator::Negate => {
                    Value::Number(to_number(&value).map_or(f64::NAN, |number| -number))
                }
                UnaryOperator::Not => Value::Boolean(!is_truthy(&value)),
            })
        }
        Expr::Binary {
            left,
            operator,
            right,
        } => {
            let left = evaluate_in(left, environment, call_depth)?;
            let right = evaluate_in(right, environment, call_depth)?;
            binary(*operator, left, right)
        }
        Expr::Logical {
            left,
            operator,
            right,
        } => {
            let left = evaluate_in(left, environment, call_depth)?;
            let use_right = match operator {
                LogicalOperator::And => is_truthy(&left),
                LogicalOperator::Or => !is_truthy(&left),
            };
            if use_right {
                evaluate_in(right, environment, call_depth)
            } else {
                Ok(left)
            }
        }
        Expr::Call { callee, arguments } => {
            let name = match callee.as_ref() {
                Expr::Identifier(name) => name,
                other => {
                    let value = evaluate_in(other, environment, call_depth)?;
                    return Err(JsError::new(format!("value {value:?} is not callable")));
                }
            };

            let function = environment.resolve_function(name)?;

            if arguments.len() != function.params.len() {
                return Err(JsError::new(format!(
                    "function `{name}` expected {} arguments but received {}",
                    function.params.len(),
                    arguments.len()
                )));
            }
            if call_depth >= MAX_CALL_DEPTH {
                return Err(JsError::new(format!(
                    "maximum function call depth ({MAX_CALL_DEPTH}) exceeded"
                )));
            }

            let values = arguments
                .iter()
                .map(|argument| evaluate_in(argument, environment, call_depth))
                .collect::<JsResult<Vec<_>>>()?;

            // Hide the caller's local scopes while preserving global state.
            let caller_scopes = std::mem::take(&mut environment.scopes);
            let global_scope = caller_scopes.first().cloned().unwrap_or_default();
            environment.scopes = vec![global_scope, Scope::default()];
            for (param, value) in function.params.iter().zip(values) {
                if let Err(error) = environment.declare(param, value, VarKind::Let) {
                    let updated_global = environment.scopes[0].clone();
                    environment.scopes = caller_scopes;
                    environment.scopes[0] = updated_global;
                    return Err(error);
                }
            }
            environment.scopes[1]
                .functions
                .insert(name.clone(), function.clone());
            let result = execute_statements(environment, &function.body, true, call_depth + 1);
            let updated_global = environment.scopes[0].clone();
            environment.scopes = caller_scopes;
            environment.scopes[0] = updated_global;
            match result? {
                Completion::Normal(_) => Ok(Value::Undefined),
                Completion::Return(value) => Ok(value),
            }
        }
    }
}

fn binary(operator: BinaryOperator, left: Value, right: Value) -> JsResult<Value> {
    if matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::StrictEqual
            | BinaryOperator::StrictNotEqual
    ) {
        let equal = if matches!(
            operator,
            BinaryOperator::StrictEqual | BinaryOperator::StrictNotEqual
        ) {
            left == right
        } else {
            loose_equal(&left, &right)
        };
        return Ok(Value::Boolean(
            if matches!(
                operator,
                BinaryOperator::NotEqual | BinaryOperator::StrictNotEqual
            ) {
                !equal
            } else {
                equal
            },
        ));
    }

    let (left, right) = (to_number(&left)?, to_number(&right)?);
    Ok(match operator {
        BinaryOperator::Add => Value::Number(left + right),
        BinaryOperator::Subtract => Value::Number(left - right),
        BinaryOperator::Multiply => Value::Number(left * right),
        BinaryOperator::Divide => Value::Number(left / right),
        BinaryOperator::Remainder => Value::Number(left % right),
        BinaryOperator::Less => Value::Boolean(left < right),
        BinaryOperator::LessEqual => Value::Boolean(left <= right),
        BinaryOperator::Greater => Value::Boolean(left > right),
        BinaryOperator::GreaterEqual => Value::Boolean(left >= right),
        BinaryOperator::Equal | BinaryOperator::StrictEqual => Value::Boolean(left == right),
        BinaryOperator::NotEqual | BinaryOperator::StrictNotEqual => Value::Boolean(left != right),
    })
}

fn loose_equal(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }
    matches!(
        (to_number(left), to_number(right)),
        (Ok(left), Ok(right)) if left == right
    )
}

fn to_number(value: &Value) -> JsResult<f64> {
    match value {
        Value::Number(number) => Ok(*number),
        Value::Boolean(true) => Ok(1.0),
        Value::Boolean(false) | Value::Null => Ok(0.0),
        Value::Undefined => Ok(f64::NAN),
        Value::String(text) => text
            .trim()
            .parse()
            .map_err(|_| JsError::new("cannot convert string to number")),
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(value) => *value,
        Value::Number(value) => *value != 0.0 && !value.is_nan(),
        Value::String(value) => !value.is_empty(),
    }
}

/// Evaluates an expression without declarations, using a fresh environment.
pub fn evaluate(expression: &Expr) -> Value {
    evaluate_in(expression, &mut Environment::new(), 0).unwrap_or(Value::Undefined)
}
