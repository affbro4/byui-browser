//! Tree-walk interpreter for parsed programs.
//!
//! Supports expressions, `var`/`let`/`const` declarations, blocks, `if`,
//! `while`, function declarations with closures, calls, and `return`.
//!
//! Semantics follow ECMAScript where supported: `var` and function
//! declarations are hoisted to the enclosing function or script, `let` and
//! `const` are block scoped and unusable before their declaration runs, and
//! operators use JavaScript type conversions.
//!
//! Limitations:
//! - Assigning to an undeclared name is an error, as in strict mode.
//! - There is no garbage collector yet. A function stored in the scope it
//!   closes over forms an `Arc` cycle, so its scope is never freed.
//! - Evaluation runs on a dedicated engine thread with a large stack. Deep
//!   recursion, such as a function that calls itself forever, returns a
//!   "Maximum call stack size exceeded" error once that stack is nearly full,
//!   instead of overflowing the native stack.
//! - Each evaluation has a step budget (see [`DEFAULT_STEP_LIMIT`]), so
//!   endless loops fail with an error instead of running forever.

mod conversions;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::ast::{
    BinaryOperator, Expr, LogicalOperator, Program, Statement, UnaryOperator, VarKind,
};
use crate::stack::{StackGuard, with_engine_stack};
use crate::{HostFunction, JsError, JsResult, Value};
pub(crate) use conversions::number_to_string;
use conversions::{
    is_truthy, less_than, loose_equals, strict_equals, to_int32, to_number, to_primitive, to_uint32,
};

const STACK_OVERFLOW: &str = "Maximum call stack size exceeded";

const STEP_LIMIT_EXCEEDED: &str = "Script exceeded the evaluation step limit";

/// Steps a script may take by default, where one step is evaluating one
/// statement or expression. This stops scripts such as `while (true) {}`
/// from hanging the engine thread. It is a stand-in for a time-based or
/// host-controlled interrupt, which does not exist yet.
pub const DEFAULT_STEP_LIMIT: u64 = 10_000_000;

/// Locks a scope. No code panics while holding the lock, so a poisoned lock
/// still holds consistent bindings and is recovered rather than propagated.
trait LockScope {
    fn lock_scope(&self) -> MutexGuard<'_, Scope>;
}

impl LockScope for Mutex<Scope> {
    fn lock_scope(&self) -> MutexGuard<'_, Scope> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug)]
struct Binding {
    /// `None` while a `let` or `const` binding is before its declaration.
    value: Option<Value>,
    mutable: bool,
    /// Whether the binding came from `let`, `const`, or a block function.
    lexical: bool,
}

#[derive(Default)]
struct Scope {
    bindings: HashMap<String, Binding>,
    parent: Option<ScopeRef>,
}

type ScopeRef = Arc<Mutex<Scope>>;

fn child_scope(parent: &ScopeRef) -> ScopeRef {
    Arc::new(Mutex::new(Scope {
        bindings: HashMap::new(),
        parent: Some(Arc::clone(parent)),
    }))
}

/// Applies `action` to the nearest binding named `name`.
fn with_binding<T>(
    scope: &ScopeRef,
    name: &str,
    action: impl FnOnce(&mut Binding) -> JsResult<T>,
) -> JsResult<T> {
    let mut current = Arc::clone(scope);
    loop {
        if let Some(binding) = current.lock_scope().bindings.get_mut(name) {
            return action(binding);
        }
        let parent = current.lock_scope().parent.clone();
        match parent {
            Some(parent) => current = parent,
            None => return Err(JsError::new(format!("`{name}` is not defined"))),
        }
    }
}

fn read(scope: &ScopeRef, name: &str) -> JsResult<Value> {
    with_binding(scope, name, |binding| {
        binding.value.clone().ok_or_else(|| uninitialized(name))
    })
}

fn assign(scope: &ScopeRef, name: &str, value: Value) -> JsResult<()> {
    with_binding(scope, name, |binding| {
        if binding.value.is_none() {
            return Err(uninitialized(name));
        }
        if !binding.mutable {
            return Err(JsError::new(format!("Assignment to constant `{name}`")));
        }
        binding.value = Some(value);
        Ok(())
    })
}

fn uninitialized(name: &str) -> JsError {
    JsError::new(format!("Cannot access `{name}` before initialization"))
}

fn already_declared(name: &str) -> JsError {
    JsError::new(format!("`{name}` has already been declared"))
}

/// Declares a `var`-style binding in exactly `scope`, keeping any existing
/// `var` binding of the same name.
fn declare_var(scope: &ScopeRef, name: &str) -> JsResult<()> {
    let mut scope = scope.lock_scope();
    match scope.bindings.get(name) {
        Some(binding) if binding.lexical => Err(already_declared(name)),
        Some(_) => Ok(()),
        None => {
            scope.bindings.insert(
                name.to_owned(),
                Binding {
                    value: Some(Value::Undefined),
                    mutable: true,
                    lexical: false,
                },
            );
            Ok(())
        }
    }
}

/// Declares a block-scoped binding in exactly `scope`.
fn declare_lexical(
    scope: &ScopeRef,
    name: &str,
    mutable: bool,
    value: Option<Value>,
) -> JsResult<()> {
    let mut scope = scope.lock_scope();
    if scope.bindings.contains_key(name) {
        return Err(already_declared(name));
    }
    scope.bindings.insert(
        name.to_owned(),
        Binding {
            value,
            mutable,
            lexical: true,
        },
    );
    Ok(())
}

/// Sets a binding that was hoisted into exactly `scope`.
fn initialize(scope: &ScopeRef, name: &str, value: Value) {
    if let Some(binding) = scope.lock_scope().bindings.get_mut(name) {
        binding.value = Some(value);
    }
}

/// A chain of lexical scopes used while executing a JavaScript program.
pub struct Environment {
    scope: ScopeRef,
}

impl Environment {
    /// Creates an environment with one empty global scope.
    pub fn new() -> Self {
        Self {
            scope: Arc::default(),
        }
    }

    /// Reads the nearest binding with `name`.
    ///
    /// Fails if no binding exists or if a `let`/`const` binding has not been
    /// initialized yet.
    pub fn get(&self, name: &str) -> JsResult<Value> {
        read(&self.scope, name)
    }

    /// Updates the nearest mutable binding with `name`, returning `value`.
    ///
    /// Fails for undeclared names, constants, and uninitialized bindings.
    pub fn set(&mut self, name: &str, value: Value) -> JsResult<Value> {
        assign(&self.scope, name, value.clone())?;
        Ok(value)
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Environment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Environment")
            .finish_non_exhaustive()
    }
}

/// A callable JavaScript function value.
///
/// Cloning is cheap and preserves identity: clones compare equal with `==`
/// and `===`, while separately created functions never do.
#[derive(Clone)]
pub struct Function(Arc<FunctionKind>);

enum FunctionKind {
    Script {
        name: String,
        params: Vec<String>,
        body: Vec<Statement>,
        closure: ScopeRef,
    },
    Host {
        name: String,
        function: HostFunction,
    },
}

impl Function {
    /// Wraps a Rust host function as a JavaScript function named `name`.
    pub fn from_host(name: impl Into<String>, function: HostFunction) -> Self {
        Self(Arc::new(FunctionKind::Host {
            name: name.into(),
            function,
        }))
    }

    /// The function's declared name.
    pub fn name(&self) -> &str {
        match &*self.0 {
            FunctionKind::Script { name, .. } | FunctionKind::Host { name, .. } => name,
        }
    }
}

impl PartialEq for Function {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Function({})", self.name())
    }
}

/// The JavaScript string form of the function. Source text is not retained,
/// so script functions show their signature with a placeholder body.
impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.0 {
            FunctionKind::Script { name, params, .. } => {
                write!(
                    formatter,
                    "function {name}({}) {{ [code] }}",
                    params.join(", ")
                )
            }
            FunctionKind::Host { name, .. } => {
                write!(formatter, "function {name}() {{ [native code] }}")
            }
        }
    }
}

/// Evaluates a parsed program with a fresh environment, returning its
/// completion value (the value of the last value-producing statement).
///
/// Runs on the engine thread described in the module documentation and
/// stops after [`DEFAULT_STEP_LIMIT`] steps.
pub fn evaluate_program(program: &Program) -> JsResult<Value> {
    evaluate_program_with_step_limit(program, DEFAULT_STEP_LIMIT)
}

/// Evaluates a program like [`evaluate_program`], but fails with a
/// step-limit error once `step_limit` statements and expressions have been
/// evaluated. A limit of `0` rejects every non-empty program.
pub fn evaluate_program_with_step_limit(program: &Program, step_limit: u64) -> JsResult<Value> {
    evaluate_program_with_globals(program, &HashMap::new(), step_limit)
}

/// Evaluates a program with `globals` installed as callable functions in a
/// scope enclosing the script, so scripts may shadow them with `let`.
pub(crate) fn evaluate_program_with_globals(
    program: &Program,
    globals: &HashMap<String, HostFunction>,
    step_limit: u64,
) -> JsResult<Value> {
    with_engine_stack(|| {
        let global_scope: ScopeRef = Arc::default();
        for (name, function) in globals {
            global_scope.lock_scope().bindings.insert(
                name.clone(),
                Binding {
                    value: Some(Value::Function(Function::from_host(
                        name,
                        Arc::clone(function),
                    ))),
                    mutable: true,
                    lexical: false,
                },
            );
        }
        let script_scope = child_scope(&global_scope);
        let completion = Interpreter::new(step_limit).run_body(&script_scope, &program.body)?;
        Ok(match completion {
            Completion::Normal(value) => value.unwrap_or(Value::Undefined),
            Completion::Return(value) => value,
        })
    })
}

/// Evaluates one expression in a fresh, empty environment.
///
/// Runs on the engine thread described in the module documentation and
/// stops after [`DEFAULT_STEP_LIMIT`] steps.
pub fn evaluate(expression: &Expr) -> JsResult<Value> {
    with_engine_stack(|| Interpreter::new(DEFAULT_STEP_LIMIT).evaluate(expression, &Arc::default()))
}

/// Statement result; `Normal(None)` is an empty completion, such as from a
/// declaration, which does not replace the previous completion value.
enum Completion {
    Normal(Option<Value>),
    Return(Value),
}

struct Interpreter {
    stack: StackGuard,
    steps_remaining: u64,
}

impl Interpreter {
    /// Creates an interpreter measuring stack use from the caller's frame and
    /// allowing `step_limit` evaluation steps.
    fn new(step_limit: u64) -> Self {
        Self {
            stack: StackGuard::new(),
            steps_remaining: step_limit,
        }
    }

    /// Charges one step and checks stack use before evaluating a statement or
    /// expression.
    fn guarded<T>(&mut self, action: impl FnOnce(&mut Self) -> JsResult<T>) -> JsResult<T> {
        if self.steps_remaining == 0 {
            return Err(JsError::new(STEP_LIMIT_EXCEEDED));
        }
        self.steps_remaining -= 1;
        if self.stack.exhausted() {
            return Err(JsError::new(STACK_OVERFLOW));
        }
        action(self)
    }

    /// Runs a script or function body after hoisting its declarations.
    fn run_body(&mut self, scope: &ScopeRef, statements: &[Statement]) -> JsResult<Completion> {
        hoist_var_declarations(scope, statements)?;
        hoist_lexical_declarations(scope, statements, true)?;
        self.execute_statements(scope, statements)
    }

    fn execute_statements(
        &mut self,
        scope: &ScopeRef,
        statements: &[Statement],
    ) -> JsResult<Completion> {
        let mut value = None;
        for statement in statements {
            match self.execute_statement(scope, statement)? {
                Completion::Normal(Some(result)) => value = Some(result),
                Completion::Normal(None) => {}
                completion @ Completion::Return(_) => return Ok(completion),
            }
        }
        Ok(Completion::Normal(value))
    }

    fn execute_statement(
        &mut self,
        scope: &ScopeRef,
        statement: &Statement,
    ) -> JsResult<Completion> {
        self.guarded(|interpreter| interpreter.execute_statement_unguarded(scope, statement))
    }

    fn execute_statement_unguarded(
        &mut self,
        scope: &ScopeRef,
        statement: &Statement,
    ) -> JsResult<Completion> {
        match statement {
            Statement::Expression(expression) => {
                Ok(Completion::Normal(Some(self.evaluate(expression, scope)?)))
            }
            Statement::VariableDeclaration { kind, name, init } => {
                let value = match init {
                    Some(expression) => Some(self.evaluate(expression, scope)?),
                    None => None,
                };
                match (kind, value) {
                    (VarKind::Var, Some(value)) => assign(scope, name, value)?,
                    (VarKind::Var, None) => {}
                    (VarKind::Let | VarKind::Const, value) => {
                        initialize(scope, name, value.unwrap_or(Value::Undefined));
                    }
                }
                Ok(Completion::Normal(None))
            }
            Statement::Block(statements) => {
                let block_scope = child_scope(scope);
                hoist_lexical_declarations(&block_scope, statements, false)?;
                self.execute_statements(&block_scope, statements)
            }
            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let branch = if is_truthy(&self.evaluate(condition, scope)?) {
                    Some(then_branch)
                } else {
                    else_branch.as_ref()
                };
                let completion = match branch {
                    Some(branch) => self.execute_statement(scope, branch)?,
                    None => Completion::Normal(None),
                };
                Ok(match completion {
                    Completion::Normal(value) => {
                        Completion::Normal(Some(value.unwrap_or(Value::Undefined)))
                    }
                    completion => completion,
                })
            }
            Statement::While { condition, body } => {
                let mut last = Value::Undefined;
                while is_truthy(&self.evaluate(condition, scope)?) {
                    match self.execute_statement(scope, body)? {
                        Completion::Normal(Some(value)) => last = value,
                        Completion::Normal(None) => {}
                        completion @ Completion::Return(_) => return Ok(completion),
                    }
                }
                Ok(Completion::Normal(Some(last)))
            }
            // Bound when the enclosing body or block was entered.
            Statement::FunctionDeclaration { .. } => Ok(Completion::Normal(None)),
            Statement::Return(expression) => Ok(Completion::Return(match expression {
                Some(expression) => self.evaluate(expression, scope)?,
                None => Value::Undefined,
            })),
        }
    }

    fn evaluate(&mut self, expression: &Expr, scope: &ScopeRef) -> JsResult<Value> {
        self.guarded(|interpreter| interpreter.evaluate_unguarded(expression, scope))
    }

    fn evaluate_unguarded(&mut self, expression: &Expr, scope: &ScopeRef) -> JsResult<Value> {
        match expression {
            Expr::Identifier(name) => read(scope, name),
            Expr::Assign { name, value } => {
                let value = self.evaluate(value, scope)?;
                assign(scope, name, value.clone())?;
                Ok(value)
            }
            Expr::Number(number) => Ok(Value::Number(*number)),
            Expr::String(text) => Ok(Value::String(text.clone())),
            Expr::Boolean(value) => Ok(Value::Boolean(*value)),
            Expr::Null => Ok(Value::Null),
            Expr::Undefined => Ok(Value::Undefined),
            Expr::Unary { operator, operand } => {
                let value = self.evaluate(operand, scope)?;
                Ok(match operator {
                    UnaryOperator::Negate => Value::Number(-to_number(&value)),
                    UnaryOperator::Not => Value::Boolean(!is_truthy(&value)),
                    UnaryOperator::BitwiseNot => Value::Number(f64::from(!to_int32(&value))),
                })
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                let left = self.evaluate(left, scope)?;
                let right = self.evaluate(right, scope)?;
                Ok(binary(*operator, &left, &right))
            }
            Expr::Logical {
                left,
                operator,
                right,
            } => {
                let left = self.evaluate(left, scope)?;
                let use_right = match operator {
                    LogicalOperator::And => is_truthy(&left),
                    LogicalOperator::Or => !is_truthy(&left),
                };
                if use_right {
                    self.evaluate(right, scope)
                } else {
                    Ok(left)
                }
            }
            Expr::Call { callee, arguments } => {
                let function = self.evaluate(callee, scope)?;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.evaluate(argument, scope))
                    .collect::<JsResult<Vec<_>>>()?;
                let Value::Function(function) = function else {
                    let description = match callee.as_ref() {
                        Expr::Identifier(name) => format!("`{name}`"),
                        _ => "expression".to_owned(),
                    };
                    return Err(JsError::new(format!("{description} is not a function")));
                };
                self.call(&function, &arguments)
            }
        }
    }

    fn call(&mut self, function: &Function, arguments: &[Value]) -> JsResult<Value> {
        match &*function.0 {
            FunctionKind::Host { function, .. } => function(arguments),
            FunctionKind::Script {
                params,
                body,
                closure,
                ..
            } => self.guarded(|interpreter| {
                let scope = child_scope(closure);
                for (index, param) in params.iter().enumerate() {
                    declare_var(&scope, param)?;
                    initialize(
                        &scope,
                        param,
                        arguments.get(index).cloned().unwrap_or(Value::Undefined),
                    );
                }
                Ok(match interpreter.run_body(&scope, body)? {
                    Completion::Return(value) => value,
                    Completion::Normal(_) => Value::Undefined,
                })
            }),
        }
    }
}

/// Declares every `var` in a function or script body, including those nested
/// in blocks and control flow, but not those inside nested functions.
fn hoist_var_declarations(scope: &ScopeRef, statements: &[Statement]) -> JsResult<()> {
    for statement in statements {
        match statement {
            Statement::VariableDeclaration {
                kind: VarKind::Var,
                name,
                ..
            } => declare_var(scope, name)?,
            Statement::Block(statements) => hoist_var_declarations(scope, statements)?,
            Statement::If {
                then_branch,
                else_branch,
                ..
            } => {
                hoist_var_declarations(scope, std::slice::from_ref(then_branch))?;
                if let Some(else_branch) = else_branch {
                    hoist_var_declarations(scope, std::slice::from_ref(else_branch))?;
                }
            }
            Statement::While { body, .. } => {
                hoist_var_declarations(scope, std::slice::from_ref(body))?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Declares the `let`, `const`, and function declarations that appear
/// directly in `statements`. `let`/`const` start uninitialized. Functions are
/// bound immediately: `var`-style in a function or script body, block scoped
/// otherwise.
fn hoist_lexical_declarations(
    scope: &ScopeRef,
    statements: &[Statement],
    function_body: bool,
) -> JsResult<()> {
    for statement in statements {
        match statement {
            Statement::VariableDeclaration {
                kind: kind @ (VarKind::Let | VarKind::Const),
                name,
                ..
            } => declare_lexical(scope, name, *kind == VarKind::Let, None)?,
            Statement::FunctionDeclaration { name, params, body } => {
                let function = Value::Function(Function(Arc::new(FunctionKind::Script {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: Arc::clone(scope),
                })));
                if function_body {
                    declare_var(scope, name)?;
                    initialize(scope, name, function);
                } else {
                    declare_lexical(scope, name, true, Some(function))?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn binary(operator: BinaryOperator, left: &Value, right: &Value) -> Value {
    let number = |operation: fn(f64, f64) -> f64| {
        Value::Number(operation(to_number(left), to_number(right)))
    };
    let int32 = |operation: fn(i32, i32) -> i32| {
        Value::Number(f64::from(operation(to_int32(left), to_int32(right))))
    };
    let shift = to_uint32(right) & 31;
    match operator {
        BinaryOperator::Add => {
            let (left, right) = (to_primitive(left), to_primitive(right));
            if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                Value::String(format!("{left}{right}"))
            } else {
                Value::Number(to_number(&left) + to_number(&right))
            }
        }
        BinaryOperator::Subtract => number(|left, right| left - right),
        BinaryOperator::Multiply => number(|left, right| left * right),
        BinaryOperator::Divide => number(|left, right| left / right),
        BinaryOperator::Remainder => number(|left, right| left % right),
        BinaryOperator::BitwiseAnd => int32(|left, right| left & right),
        BinaryOperator::BitwiseOr => int32(|left, right| left | right),
        BinaryOperator::BitwiseXor => int32(|left, right| left ^ right),
        BinaryOperator::LeftShift => Value::Number(f64::from(to_int32(left).wrapping_shl(shift))),
        BinaryOperator::SignedRightShift => Value::Number(f64::from(to_int32(left) >> shift)),
        BinaryOperator::UnsignedRightShift => Value::Number(f64::from(to_uint32(left) >> shift)),
        BinaryOperator::Less => Value::Boolean(less_than(left, right) == Some(true)),
        BinaryOperator::Greater => Value::Boolean(less_than(right, left) == Some(true)),
        BinaryOperator::LessEqual => Value::Boolean(less_than(right, left) == Some(false)),
        BinaryOperator::GreaterEqual => Value::Boolean(less_than(left, right) == Some(false)),
        BinaryOperator::Equal => Value::Boolean(loose_equals(left, right)),
        BinaryOperator::NotEqual => Value::Boolean(!loose_equals(left, right)),
        BinaryOperator::StrictEqual => Value::Boolean(strict_equals(left, right)),
        BinaryOperator::StrictNotEqual => Value::Boolean(!strict_equals(left, right)),
    }
}
