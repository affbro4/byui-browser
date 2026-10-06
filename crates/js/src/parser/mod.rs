use crate::ast::{
    BinaryOperator, Expr, LogicalOperator, Program, Statement, UnaryOperator, VarKind,
};
use crate::lexer::Token;

/// Parses a complete JavaScript expression from tokens.
pub fn parse(tokens: &[Token]) -> Result<Expr, String> {
    let mut parser = Parser::new(tokens);
    let expression = parser.parse_expression(0)?;
    if let Some(token) = parser.peek() {
        return Err(format!("Unexpected token after expression: {token:?}"));
    }
    Ok(expression)
}

/// Parses a token stream into the supported JavaScript program subset.
pub fn parse_program(tokens: &[Token]) -> Result<Program, String> {
    Parser::new(tokens).parse_program()
}

pub(super) struct Parser<'tokens> {
    tokens: &'tokens [Token],
    position: usize,
}

impl<'tokens> Parser<'tokens> {
    fn new(tokens: &'tokens [Token]) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }
    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.position);
        self.position += usize::from(token.is_some());
        token
    }

    fn parse_program(mut self) -> Result<Program, String> {
        let mut body = Vec::new();
        while self.peek().is_some() {
            if matches!(self.peek(), Some(Token::Semicolon)) {
                self.advance();
                continue;
            }
            body.push(self.parse_statement()?);
        }
        Ok(Program { body })
    }

    fn parse_statement(&mut self) -> Result<Statement, String> {
        match self.peek() {
            Some(Token::Let) | Some(Token::Const) | Some(Token::Var) => self.parse_declaration(),
            Some(Token::If) => self.parse_if_statement(),
            Some(Token::While) => self.parse_while_statement(),
            Some(Token::Function) => self.parse_function_declaration(),
            Some(Token::Return) => self.parse_return_statement(),
            Some(Token::LeftBrace) => self.parse_block(),
            _ => {
                let expression = self.parse_expression(0)?;
                if matches!(self.peek(), Some(Token::Semicolon)) {
                    self.advance();
                }
                Ok(Statement::Expression(expression))
            }
        }
    }

    fn parse_declaration(&mut self) -> Result<Statement, String> {
        let kind = match self.advance() {
            Some(Token::Let) => VarKind::Let,
            Some(Token::Const) => VarKind::Const,
            Some(Token::Var) => VarKind::Var,
            Some(token) => return Err(format!("Expected a declaration keyword, found {token:?}")),
            None => return Err("Expected a declaration keyword".into()),
        };
        let name = match self.advance() {
            Some(Token::Identifier(name)) => name.clone(),
            Some(token) => return Err(format!("Expected a binding name, found {token:?}")),
            None => return Err("Expected a binding name".into()),
        };
        let init = if matches!(self.peek(), Some(Token::Assign)) {
            self.advance();
            Some(self.parse_expression(0)?)
        } else {
            None
        };
        if matches!(kind, VarKind::Const) && init.is_none() {
            return Err("A const declaration requires an initializer".into());
        }
        if matches!(self.peek(), Some(Token::Semicolon)) {
            self.advance();
        }
        Ok(Statement::VariableDeclaration { kind, name, init })
    }

    fn parse_block(&mut self) -> Result<Statement, String> {
        self.expect(Token::LeftBrace)?;
        let mut body = Vec::new();
        while !matches!(self.peek(), Some(Token::RightBrace)) {
            if self.peek().is_none() {
                return Err("Expected `}` to close block".into());
            }
            body.push(self.parse_statement()?);
        }
        self.advance();
        Ok(Statement::Block(body))
    }

    fn parse_if_statement(&mut self) -> Result<Statement, String> {
        self.advance();
        self.expect(Token::LeftParen)?;
        let condition = self.parse_expression(0)?;
        self.expect(Token::RightParen)?;
        let then_branch = self.parse_statement()?;
        let else_branch = if matches!(self.peek(), Some(Token::Else)) {
            self.advance();
            Some(self.parse_statement()?)
        } else {
            None
        };
        Ok(Statement::If {
            condition,
            then_branch: Box::new(then_branch),
            else_branch: else_branch.map(Box::new),
        })
    }

    fn parse_while_statement(&mut self) -> Result<Statement, String> {
        self.advance();
        self.expect(Token::LeftParen)?;
        let condition = self.parse_expression(0)?;
        self.expect(Token::RightParen)?;
        let body = self.parse_statement()?;
        Ok(Statement::While {
            condition,
            body: Box::new(body),
        })
    }

    fn parse_function_declaration(&mut self) -> Result<Statement, String> {
        self.advance();
        let name = self.parse_binding_name()?;
        self.expect(Token::LeftParen)?;
        let mut params = Vec::new();
        if !matches!(self.peek(), Some(Token::RightParen)) {
            loop {
                params.push(self.parse_binding_name()?);
                if !matches!(self.peek(), Some(Token::Comma)) {
                    break;
                }
                self.advance();
            }
        }
        self.expect(Token::RightParen)?;
        let body = self.parse_block()?;
        let Statement::Block(body) = body else {
            return Err("Expected a block as the function body".into());
        };
        Ok(Statement::FunctionDeclaration { name, params, body })
    }

    fn parse_return_statement(&mut self) -> Result<Statement, String> {
        self.advance();
        let value = if matches!(
            self.peek(),
            None | Some(Token::Semicolon) | Some(Token::RightBrace)
        ) {
            None
        } else {
            Some(self.parse_expression(0)?)
        };
        if matches!(self.peek(), Some(Token::Semicolon)) {
            self.advance();
        }
        Ok(Statement::Return(value))
    }

    fn parse_binding_name(&mut self) -> Result<String, String> {
        match self.advance() {
            Some(Token::Identifier(name)) => Ok(name.clone()),
            Some(token) => Err(format!("Expected a binding name, found {token:?}")),
            None => Err("Expected a binding name".into()),
        }
    }

    pub(super) fn parse_expression(&mut self, minimum_precedence: u8) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while let Some((precedence, operator)) = self.peek().and_then(infix_operator) {
            if precedence < minimum_precedence {
                break;
            }
            self.advance();
            let right = self.parse_expression(precedence + 1)?;
            left = match operator {
                InfixOperator::Binary(operator) => Expr::Binary {
                    left: Box::new(left),
                    operator,
                    right: Box::new(right),
                },
                InfixOperator::Logical(operator) => Expr::Logical {
                    left: Box::new(left),
                    operator,
                    right: Box::new(right),
                },
            };
        }
        if minimum_precedence == 0 && matches!(self.peek(), Some(Token::Assign)) {
            self.advance();
            let name = match left {
                Expr::Identifier(name) => name,
                _ => return Err("Invalid assignment target".into()),
            };
            let value = self.parse_expression(0)?;
            return Ok(Expr::Assign {
                name,
                value: Box::new(value),
            });
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Token::Subtract) => {
                self.advance();
                Ok(Expr::Unary {
                    operator: UnaryOperator::Negate,
                    operand: Box::new(self.parse_unary()?),
                })
            }
            Some(Token::Bang) => {
                self.advance();
                Ok(Expr::Unary {
                    operator: UnaryOperator::Not,
                    operand: Box::new(self.parse_unary()?),
                })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let mut expression = match self.advance() {
            Some(Token::Number(number)) => Expr::Number(*number),
            Some(Token::String(text)) => Expr::String(text.clone()),
            Some(Token::True) => Expr::Boolean(true),
            Some(Token::False) => Expr::Boolean(false),
            Some(Token::Null) => Expr::Null,
            Some(Token::Undefined) => Expr::Undefined,
            Some(Token::Identifier(name)) => Expr::Identifier(name.clone()),
            Some(Token::LeftParen) => {
                let expression = self.parse_expression(0)?;
                self.expect(Token::RightParen)?;
                expression
            }
            Some(token) => return Err(format!("Expected an expression, found {token:?}")),
            None => return Err("Expected an expression".into()),
        };

        while matches!(self.peek(), Some(Token::LeftParen)) {
            self.advance();
            let mut arguments = Vec::new();
            if !matches!(self.peek(), Some(Token::RightParen)) {
                loop {
                    arguments.push(self.parse_expression(0)?);
                    if !matches!(self.peek(), Some(Token::Comma)) {
                        break;
                    }
                    self.advance();
                }
            }
            self.expect(Token::RightParen)?;
            expression = Expr::Call {
                callee: Box::new(expression),
                arguments,
            };
        }
        Ok(expression)
    }

    fn expect(&mut self, expected: Token) -> Result<(), String> {
        if self.advance() == Some(&expected) {
            Ok(())
        } else {
            Err(format!("Expected {expected:?}"))
        }
    }
}

fn binary_operator(token: &Token) -> Option<BinaryOperator> {
    Some(match token {
        Token::Plus => BinaryOperator::Add,
        Token::Subtract => BinaryOperator::Subtract,
        Token::Multiply => BinaryOperator::Multiply,
        Token::Divide => BinaryOperator::Divide,
        Token::EqualEqual => BinaryOperator::Equal,
        Token::BangEqual => BinaryOperator::NotEqual,
        Token::StrictEqual => BinaryOperator::StrictEqual,
        Token::StrictBangEqual => BinaryOperator::StrictNotEqual,
        Token::LessThan => BinaryOperator::Less,
        Token::LessEqual => BinaryOperator::LessEqual,
        Token::GreaterThan => BinaryOperator::Greater,
        Token::GreaterEqual => BinaryOperator::GreaterEqual,
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum InfixOperator {
    Binary(BinaryOperator),
    Logical(LogicalOperator),
}

fn infix_operator(token: &Token) -> Option<(u8, InfixOperator)> {
    match token {
        Token::OrOr => Some((1, InfixOperator::Logical(LogicalOperator::Or))),
        Token::AndAnd => Some((2, InfixOperator::Logical(LogicalOperator::And))),
        _ => binary_operator(token)
            .map(|operator| (operator.precedence() + 2, InfixOperator::Binary(operator))),
    }
}

impl BinaryOperator {
    fn precedence(self) -> u8 {
        match self {
            Self::Equal | Self::NotEqual | Self::StrictEqual | Self::StrictNotEqual => 1,
            Self::Less | Self::LessEqual | Self::Greater | Self::GreaterEqual => 2,
            Self::Add | Self::Subtract => 3,
            Self::Multiply | Self::Divide | Self::Remainder => 4,
        }
    }
}
