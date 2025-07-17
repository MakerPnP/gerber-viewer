use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::str::Chars;

use gerber_types::{MacroBoolean, MacroDecimal, MacroInteger};
use thiserror::Error;

/// Gerber spec 2024.05 - 4.5.4.3 - "The undefined variables are 0".
#[derive(Debug, Default)]
pub struct MacroContext {
    variables: HashMap<u32, f64>,
}

impl MacroContext {
    pub fn get(&self, variable: &u32) -> f64 {
        self.variables
            .get(&variable)
            .copied()
            .unwrap_or(0.0)
    }

    pub fn put(&mut self, variable: u32, decimal: f64) -> Result<&mut f64, MacroContextError> {
        match self.variables.entry(variable) {
            Entry::Occupied(_) => Err(MacroContextError::AlreadyDefined(variable)),
            Entry::Vacant(entry) => Ok(entry.insert(decimal)),
        }
    }
}

#[derive(Error, Debug)]
pub enum MacroContextError {
    /// Gerber spec (2024.05) - 4.5.4.3 - "Macro variables cannot be redefined"
    #[error("Already defined. variable: {0}")]
    AlreadyDefined(u32),
}

pub fn macro_decimal_to_f64(
    macro_decimal: &MacroDecimal,
    context: &MacroContext,
) -> Result<f64, ExpressionEvaluationError> {
    match macro_decimal {
        MacroDecimal::Value(value) => Ok(*value),
        MacroDecimal::Variable(id) => Ok(context.get(id)),
        MacroDecimal::Expression(args) => evaluate_expression(args, context),
    }
}

pub fn macro_boolean_to_bool(
    macro_boolean: &MacroBoolean,
    context: &MacroContext,
) -> Result<bool, ExpressionEvaluationError> {
    match macro_boolean {
        MacroBoolean::Value(value) => Ok(*value),
        MacroBoolean::Variable(id) => Ok(context.get(id) == 1.0),
        MacroBoolean::Expression(args) => evaluate_expression(args, context).map(|value| value != 0.0),
    }
}

pub fn macro_integer_to_u32(
    macro_integer: &MacroInteger,
    context: &MacroContext,
) -> Result<u32, ExpressionEvaluationError> {
    match macro_integer {
        MacroInteger::Value(value) => Ok(*value),
        MacroInteger::Variable(id) => Ok(context.get(id) as u32),
        MacroInteger::Expression(args) => evaluate_expression(args, context).map(|value| value as u32),
    }
}

pub fn macro_decimal_pair_to_f64(
    input: &(MacroDecimal, MacroDecimal),
    context: &MacroContext,
) -> Result<(f64, f64), ExpressionEvaluationError> {
    let (x, y) = (
        macro_decimal_to_f64(&input.0, context)?,
        macro_decimal_to_f64(&input.1, context)?,
    );
    Ok((x, y))
}

#[derive(Error, Debug)]
pub enum ExpressionEvaluationError {
    #[error("Unexpected character: {0}")]
    UnexpectedChar(char),
    #[error("Unexpected end of input")]
    UnexpectedEnd,
    #[error("Invalid number")]
    InvalidNumber,
}

/// Evaluates a Gerber macro expression using a recursive descent parser.
pub fn evaluate_expression(expr: &String, ctx: &MacroContext) -> Result<f64, ExpressionEvaluationError> {
    let mut parser = Parser::new(expr, ctx);
    let result = parser.parse_expression()?;
    if parser.peek().is_some() {
        Err(ExpressionEvaluationError::UnexpectedChar(parser.peek().unwrap()))
    } else {
        Ok(result)
    }
}

/// Tokenizer and Parser
///
/// Initially Generated via ChatGPT - AI: https://chatgpt.com/share/68124813-8ec4-800f-ad20-797f57d6af18
struct Parser<'a> {
    chars: Chars<'a>,
    lookahead: Option<char>,
    ctx: &'a MacroContext,
}

impl<'a> Parser<'a> {
    fn new(expr: &'a str, ctx: &'a MacroContext) -> Self {
        let mut chars = expr.chars();
        let lookahead = chars.next();
        Self {
            chars,
            lookahead,
            ctx,
        }
    }

    fn peek(&self) -> Option<char> {
        self.lookahead
    }

    fn bump(&mut self) -> Option<char> {
        let curr = self.lookahead;
        self.lookahead = self.chars.next();
        curr
    }

    fn eat_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn parse_expression(&mut self) -> Result<f64, ExpressionEvaluationError> {
        let mut value = self.parse_term()?;
        loop {
            self.eat_whitespace();
            match self.peek() {
                Some('+') => {
                    self.bump();
                    value += self.parse_term()?;
                }
                Some('-') => {
                    self.bump();
                    value -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<f64, ExpressionEvaluationError> {
        let mut value = self.parse_factor()?;
        loop {
            self.eat_whitespace();
            match self.peek() {
                Some('/') => {
                    self.bump();
                    value /= self.parse_factor()?;
                }
                // gerber spec uses 'x' for multiplication (why Camco, why...)
                Some('x') => {
                    self.bump();
                    value *= self.parse_factor()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<f64, ExpressionEvaluationError> {
        self.eat_whitespace();
        match self.peek() {
            Some('(') => {
                self.bump(); // consume '('
                let value = self.parse_expression()?;
                self.eat_whitespace();
                if self.bump() != Some(')') {
                    return Err(ExpressionEvaluationError::UnexpectedEnd);
                }
                Ok(value)
            }
            Some('$') => self.parse_variable(),
            Some(c) if c.is_ascii_digit() || c == '.' || c == '-' => self.parse_number(),
            Some(c) => Err(ExpressionEvaluationError::UnexpectedChar(c)),
            None => Err(ExpressionEvaluationError::UnexpectedEnd),
        }
    }

    fn parse_number(&mut self) -> Result<f64, ExpressionEvaluationError> {
        let mut s = String::new();
        if self.peek() == Some('-') {
            s.push('-');
            self.bump();
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }

        s.parse::<f64>()
            .map_err(|_| ExpressionEvaluationError::InvalidNumber)
    }

    fn parse_variable(&mut self) -> Result<f64, ExpressionEvaluationError> {
        self.bump(); // consume '$'
        let mut s = String::new();

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }

        let id: u32 = s
            .parse()
            .map_err(|_| ExpressionEvaluationError::InvalidNumber)?;
        Ok(self.ctx.get(&id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_addition_same_variable() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 5.0).unwrap();

        let expr = "$1+$1".to_string();
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 10.0);
    }

    #[test]
    fn test_division_two_variables() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 5.0).unwrap();
        ctx.put(2, 2.0).unwrap();

        let expr = "$1/$2".to_string();
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 2.5);
    }

    #[test]
    fn test_multiplication_two_variables_using_x() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 5.0).unwrap();
        ctx.put(2, 2.0).unwrap();

        let expr = "$1x$2".to_string();
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 10.0);
    }

    #[test]
    fn test_multiplication_decimal_literal_and_variable_using_x() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 0.25).unwrap();

        let expr = "10.0x$1".to_string();
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 2.5);
    }

    #[test]
    fn test_multiplication_integer_literal_and_variable_using_x() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 0.25).unwrap();

        let expr = "10x$1".to_string();
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 2.5);
    }

    #[test]
    fn test_multiplication_variable_and_decimal_literal_using_x() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 10.0).unwrap();

        let expr = "$1x0.25".to_string();
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 2.5);
    }

    #[test]
    fn test_multiplication_variable_and_integer_literal_using_x() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 10.0).unwrap();

        let expr = "$1x25".to_string();
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 250.0);
    }

    #[test]
    fn test_subtraction_and_division() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 5.0).unwrap();
        ctx.put(2, 2.0).unwrap();

        let expr = "$1-$2/$2".to_string(); // 5 - (2 / 2) = 4
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 4.0);
    }

    #[test]
    fn test_parentheses_with_sub_and_div() {
        let mut ctx = MacroContext::default();
        ctx.put(1, 5.0).unwrap();
        ctx.put(2, 2.0).unwrap();

        let expr = "($1-$2)/$2".to_string(); // (5 - 2) / 2 = 1.5
        let result = evaluate_expression(&expr, &ctx).unwrap();
        assert_eq!(result, 1.5);
    }
}
