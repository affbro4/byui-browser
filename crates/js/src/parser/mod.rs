use crate::ast::{Expr, UnaryOperator};
use crate::lexer::Token;

mod expressions;
mod statements;

/// Parses one complete expression from a token stream.
///
/// The currently available token set supports numeric literals, arithmetic
/// operators, and unary negation. Other AST forms remain unavailable until the
/// lexer supplies tokens for them.
pub fn parse(tokens: &[Token]) -> Result<Expr, String> {
    let mut parser = Parser::new(tokens);
    let expression = parser.parse_expression(0)?;

    if let Some(token) = parser.peek() {
        return Err(format!("Unexpected token after expression: {token:?}"));
    }

    Ok(expression)
}

struct Parser<'tokens> {
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

    fn parse_expression(&mut self, minimum_precedence: u8) -> Result<Expr, String> {
        expressions::parse(self, minimum_precedence)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if matches!(self.peek(), Some(Token::Subtract)) {
            self.advance();
            return Ok(Expr::Unary {
                operator: UnaryOperator::Negate,
                operand: Box::new(self.parse_unary()?),
            });
        }

        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.advance() {
            Some(Token::Number(number)) => Ok(Expr::Number(*number)),
            Some(token) => Err(format!("Expected an expression, found {token:?}")),
            None => Err(String::from("Expected an expression")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOperator, UnaryOperator};

    #[test]
    fn parses_unary_negation() {
        let expression = parse(&[Token::Subtract, Token::Number(7.0)]).unwrap();

        assert_eq!(
            expression,
            Expr::Unary {
                operator: UnaryOperator::Negate,
                operand: Box::new(Expr::Number(7.0)),
            }
        );
    }

    #[test]
    fn unary_negation_binds_tighter_than_binary_operators() {
        let expression = parse(&[
            Token::Number(1.0),
            Token::Subtract,
            Token::Subtract,
            Token::Number(2.0),
        ])
        .unwrap();

        assert_eq!(
            expression,
            Expr::Binary {
                left: Box::new(Expr::Number(1.0)),
                operator: BinaryOperator::Subtract,
                right: Box::new(Expr::Unary {
                    operator: UnaryOperator::Negate,
                    operand: Box::new(Expr::Number(2.0)),
                }),
            }
        );
    }
}
