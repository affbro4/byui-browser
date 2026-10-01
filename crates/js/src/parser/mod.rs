use crate::ast::{BinaryOperator, Expr, Program, Statement, UnaryOperator, VarKind};
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
            Some(Token::Let) | Some(Token::Const) => self.parse_declaration(),
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
            _ => unreachable!(),
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

    pub(super) fn parse_expression(&mut self, minimum_precedence: u8) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while let Some(operator) = self.peek().and_then(binary_operator) {
            if operator.precedence() < minimum_precedence {
                break;
            }
            self.advance();
            let right = self.parse_expression(operator.precedence() + 1)?;
            left = Expr::Binary {
                left: Box::new(left),
                operator,
                right: Box::new(right),
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
        match self.advance() {
            Some(Token::Number(number)) => Ok(Expr::Number(*number)),
            Some(Token::String(text)) => Ok(Expr::String(text.clone())),
            Some(Token::True) => Ok(Expr::Boolean(true)),
            Some(Token::False) => Ok(Expr::Boolean(false)),
            Some(Token::Null) => Ok(Expr::Null),
            Some(Token::Identifier(name)) => Ok(Expr::Identifier(name.clone())),
            Some(Token::LeftParen) => {
                let expression = self.parse_expression(0)?;
                self.expect(Token::RightParen)?;
                Ok(expression)
            }
            Some(token) => Err(format!("Expected an expression, found {token:?}")),
            None => Err("Expected an expression".into()),
        }
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
        Token::LessThan => BinaryOperator::Less,
        Token::GreaterThan => BinaryOperator::Greater,
        _ => return None,
    })
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
