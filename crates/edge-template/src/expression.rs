use std::{collections::HashMap, fmt};

use crate::{Variable, Variables};

/// A value in an expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// Text, which is also how every setting arrives: `"220"` is a number
    /// when used as one and text when compared with text.
    Text(String),
}

impl Value {
    pub fn text(text: impl Into<String>) -> Self {
        Value::Text(text.into())
    }

    /// The value as a number, if it is one or reads as one.
    fn number(&self) -> Option<Value> {
        match self {
            Value::Int(_) | Value::Float(_) => Some(self.clone()),
            Value::Text(text) => {
                let text = text.trim();
                text.parse::<i64>()
                    .map(Value::Int)
                    .ok()
                    .or_else(|| text.parse::<f64>().ok().map(Value::Float))
            }
            Value::Bool(_) => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self.number()? {
            Value::Int(value) => Some(value as f64),
            Value::Float(value) => Some(value),
            _ => None,
        }
    }

    /// The value as a script writes it out or joins it to text: as
    /// [`fmt::Display`] does.
    pub(crate) fn brief(&self) -> String {
        self.to_string()
    }

    /// Whether the value counts as true in a condition.
    pub(crate) fn truthy(&self) -> Result<bool, String> {
        match self {
            Value::Bool(value) => Ok(*value),
            Value::Int(value) => Ok(*value != 0),
            Value::Float(value) => Ok(*value != 0.0),
            // Settings store switches as "1" and "0".
            Value::Text(text) => match text.trim() {
                "1" | "true" => Ok(true),
                "0" | "false" | "" => Ok(false),
                other => Err(format!("\"{other}\" is neither true nor false")),
            },
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(value) => write!(f, "{value}"),
            // As a C++ stream writes a double, which is how the reference
            // turns every number a template gives into text (PlaceholderParser.cpp
            // `expr::to_string`, Config.hpp `ConfigOptionFloat::serialize`): six
            // significant digits, no trailing zeros, an exponent below 1e-4
            // and from 1e6 (0.25 + 3 * 0.2 reads 0.85, a feed rate 523.843).
            Value::Float(value) => f.write_str(&stream_double(*value)),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Text(text) => f.write_str(text),
        }
    }
}

/// A double as `std::ostream << double` writes it with the stream's
/// default precision of six: `%g`.
pub fn stream_double(value: f64) -> String {
    if !value.is_finite() {
        return match value {
            v if v.is_nan() => "nan".into(),
            v if v > 0.0 => "inf".into(),
            _ => "-inf".into(),
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.into();
    }
    // The exponent once rounded to six significant digits.
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let trim = |text: &str| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text.to_owned()
        }
    };
    if !(-4..6).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exponent.abs())
    } else {
        let decimals = (5 - exponent).max(0) as usize;
        trim(&format!("{value:.decimals$}"))
    }
}

/// What a template has set as it ran, over the variables it was given.
#[derive(Debug, Default)]
pub(crate) struct Overlay {
    pub values: HashMap<String, Variable>,
    /// Names declared `local`, which end with the template.
    pub locals: Vec<String>,
}

/// The value of one expression.
pub(crate) fn evaluate(
    source: &str,
    variables: &Variables,
    overlay: &mut Overlay,
) -> Result<Value, String> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens: &tokens,
        position: 0,
        variables,
        overlay,
    };
    let value = parser.ternary()?;
    match parser.peek() {
        None => Ok(value),
        Some(token) => Err(format!("unexpected {token} in \"{}\"", source.trim())),
    }
}

/// Whether what is between a pair of braces is a script (statements, which
/// may set variables and branch) and not one expression.
pub(crate) fn is_script(source: &str) -> bool {
    tokenize(source).is_ok_and(|tokens| {
        tokens.iter().any(|token| {
            matches!(token, Token::Symbol(";" | "="))
                || matches!(token, Token::Name(name) if matches!(name.as_str(), "then" | "local" | "global"))
        })
    })
}

/// Runs a script and gives what it writes out: the value of every
/// statement that is an expression, one after another.
pub(crate) fn run(
    source: &str,
    variables: &Variables,
    overlay: &mut Overlay,
) -> Result<String, String> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens: &tokens,
        position: 0,
        variables,
        overlay,
    };
    let mut out = String::new();
    parser.statements(&mut out)?;
    match parser.peek() {
        None => Ok(out),
        Some(token) => Err(format!("unexpected {token}")),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(Value),
    Text(String),
    Name(String),
    /// An operator or bracket, such as `<=` or `(`.
    Symbol(&'static str),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Number(value) => write!(f, "{value}"),
            Token::Text(text) => write!(f, "\"{text}\""),
            Token::Name(name) => f.write_str(name),
            Token::Symbol(symbol) => write!(f, "\"{symbol}\""),
        }
    }
}

/// Longest first, so `<=` is not read as `<` then `=`.
const SYMBOLS: [&str; 24] = [
    "==", "!=", "<>", "<=", ">=", "&&", "||", "=~", "!~", "<", ">", "+", "-", "*", "/", "%", "(",
    ")", "[", "]", "!", ",", ";", "=",
];

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut rest = source.trim_start();
    while let Some(first) = rest.chars().next() {
        let length = if first.is_ascii_digit()
            || (first == '.' && rest[1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            let length = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            let text = &rest[..length];
            let number = text
                .parse::<i64>()
                .map(Value::Int)
                .or_else(|_| text.parse::<f64>().map(Value::Float))
                .map_err(|_| format!("\"{text}\" is not a number"))?;
            tokens.push(Token::Number(number));
            length
        } else if first == '"' {
            let end = rest[1..].find('"').ok_or("a string is never closed")?;
            tokens.push(Token::Text(rest[1..=end].to_owned()));
            end + 2
        } else if first.is_ascii_alphabetic() || first == '_' {
            let length = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            tokens.push(Token::Name(rest[..length].to_owned()));
            length
        } else if let Some(symbol) = SYMBOLS.iter().find(|symbol| rest.starts_with(**symbol)) {
            tokens.push(Token::Symbol(symbol));
            symbol.len()
        } else if first == '?' || first == ':' {
            tokens.push(Token::Symbol(if first == '?' { "?" } else { ":" }));
            1
        } else {
            return Err(format!("\"{first}\" is not part of an expression"));
        };
        rest = rest[length..].trim_start();
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
    variables: &'a Variables,
    overlay: &'a mut Overlay,
}

/// Words that end a run of statements.
const CLOSERS: [&str; 3] = ["elsif", "else", "endif"];

impl Parser<'_> {
    /// A variable's value, as the template last set it or as it was given.
    fn lookup(&self, name: &str, index: Option<i64>) -> Result<Value, String> {
        match self.overlay.values.get(name) {
            Some(variable) => variable.read(name, index),
            None => self.variables.get(name, index),
        }
    }

    fn at_name(&self, wanted: &[&str]) -> bool {
        matches!(self.peek(), Some(Token::Name(name)) if wanted.contains(&name.as_str()))
    }

    fn name(&mut self) -> Result<String, String> {
        match self.peek().cloned() {
            Some(Token::Name(name)) => {
                self.position += 1;
                Ok(name)
            }
            Some(token) => Err(format!("expected a name but found {token}")),
            None => Err("expected a name at the end".into()),
        }
    }

    /// Statements up to the end, or to a word that closes a branch.
    fn statements(&mut self, out: &mut String) -> Result<(), String> {
        while self.peek().is_some() && !self.at_name(&CLOSERS) {
            if self.take(&[";"]).is_some() {
                continue;
            }
            self.statement(out)?;
            // A statement ends at a `;`, or where its branch or the script does.
            if self.peek().is_some() && !self.at_name(&CLOSERS) && !self.ended_block() {
                self.expect(";")?;
            }
        }
        Ok(())
    }

    /// Whether the statement just read was an `if` block, which needs no `;`.
    fn ended_block(&self) -> bool {
        matches!(self.tokens.get(self.position.wrapping_sub(1)), Some(Token::Name(name)) if name == "endif")
    }

    fn statement(&mut self, out: &mut String) -> Result<(), String> {
        if self.at_name(&["if"]) {
            return self.branch(out);
        }
        if self.at_name(&["local", "global"]) {
            let local = self.name()? == "local";
            let name = self.name()?;
            self.expect("=")?;
            let value = self.ternary()?;
            self.overlay
                .values
                .insert(name.clone(), Variable::One(value));
            if local {
                self.overlay.locals.push(name);
            }
            return Ok(());
        }
        // `name = value` or `name[index] = value`; anything else is an
        // expression, whose value the script writes out.
        let start = self.position;
        if let Some(Token::Name(name)) = self.peek().cloned() {
            self.position += 1;
            let index = if self.take(&["["]).is_some() {
                let index = self.ternary()?;
                self.expect("]")?;
                Some(index)
            } else {
                None
            };
            if self.take(&["="]).is_some() {
                let value = self.ternary()?;
                return self.assign(&name, index, value);
            }
            self.position = start;
        }
        let value = self.ternary()?;
        out.push_str(&value.brief());
        Ok(())
    }

    fn assign(&mut self, name: &str, index: Option<Value>, value: Value) -> Result<(), String> {
        let current = match self.overlay.values.get(name) {
            Some(variable) => variable.clone(),
            None => self
                .variables
                .variable(name)
                .cloned()
                .ok_or_else(|| format!("\"{name}\" is set but was never declared"))?,
        };
        let updated = match (current, index) {
            (Variable::One(_), None) => Variable::One(value),
            (Variable::Many(mut values), Some(index)) => {
                let slot = match index.number() {
                    Some(Value::Int(at)) => usize::try_from(at).ok(),
                    _ => None,
                }
                .filter(|at| *at < values.len())
                .ok_or_else(|| {
                    format!(
                        "\"{name}\" has {} entries; there is no entry {index}",
                        values.len()
                    )
                })?;
                values[slot] = value;
                Variable::Many(values)
            }
            (Variable::One(_), Some(_)) => {
                return Err(format!(
                    "\"{name}\" is a single value and cannot be indexed"
                ));
            }
            (Variable::Many(_), None) => {
                return Err(format!("\"{name}\" is a list; set one entry of it"));
            }
        };
        self.overlay.values.insert(name.to_owned(), updated);
        Ok(())
    }

    /// `if … then … elsif … then … else … endif`. Branches not taken are
    /// passed over unread, so what they mention need not exist.
    fn branch(&mut self, out: &mut String) -> Result<(), String> {
        self.position += 1;
        let mut taken = false;
        loop {
            let condition = self.ternary()?.truthy()?;
            self.expect("then")?;
            if condition && !taken {
                taken = true;
                self.statements(out)?;
            } else {
                self.skip_branch()?;
            }
            if self.take(&["elsif"]).is_some() {
                // Once a branch is taken the rest are passed over, their
                // conditions included.
                if taken {
                    self.skip_to_then()?;
                    self.skip_branch()?;
                    while self.take(&["elsif"]).is_some() {
                        self.skip_to_then()?;
                        self.skip_branch()?;
                    }
                } else {
                    continue;
                }
            }
            if self.take(&["else"]).is_some() {
                if taken {
                    self.skip_branch()?;
                } else {
                    self.statements(out)?;
                }
            }
            return self.expect("endif");
        }
    }

    fn skip_to_then(&mut self) -> Result<(), String> {
        while let Some(token) = self.peek() {
            let found = matches!(token, Token::Name(name) if name == "then");
            self.position += 1;
            if found {
                return Ok(());
            }
        }
        Err("an elsif has no then".into())
    }

    /// Passes over statements to the word that closes this branch, minding
    /// any `if … endif` inside it.
    fn skip_branch(&mut self) -> Result<(), String> {
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            if let Token::Name(name) = token {
                match name.as_str() {
                    "if" => depth += 1,
                    "endif" if depth > 0 => depth -= 1,
                    "elsif" | "else" | "endif" if depth == 0 => return Ok(()),
                    _ => {}
                }
            }
            self.position += 1;
        }
        Err("an if is never closed with endif".into())
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    /// Consumes the next token when it is one of `symbols` or, for the
    /// word operators, a name spelled that way.
    fn take(&mut self, symbols: &[&'static str]) -> Option<&'static str> {
        let found = match self.peek()? {
            Token::Symbol(symbol) => symbols.iter().find(|wanted| *wanted == symbol),
            Token::Name(name) => symbols.iter().find(|wanted| *wanted == name),
            _ => None,
        }?;
        self.position += 1;
        Some(found)
    }

    fn expect(&mut self, symbol: &'static str) -> Result<(), String> {
        self.take(&[symbol])
            .map(|_| ())
            .ok_or_else(|| match self.peek() {
                Some(token) => format!("expected \"{symbol}\" but found {token}"),
                None => format!("expected \"{symbol}\" at the end"),
            })
    }

    fn ternary(&mut self) -> Result<Value, String> {
        let condition = self.or()?;
        if self.take(&["?"]).is_none() {
            return Ok(condition);
        }
        let (yes, _) = (self.ternary()?, self.expect(":")?);
        let no = self.ternary()?;
        Ok(if condition.truthy()? { yes } else { no })
    }

    fn or(&mut self) -> Result<Value, String> {
        let mut left = self.and()?;
        while self.take(&["or", "||"]).is_some() {
            let right = self.and()?;
            left = Value::Bool(left.truthy()? || right.truthy()?);
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Value, String> {
        let mut left = self.comparison()?;
        while self.take(&["and", "&&"]).is_some() {
            let right = self.comparison()?;
            left = Value::Bool(left.truthy()? && right.truthy()?);
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Value, String> {
        let left = self.sum()?;
        let Some(operator) = self.take(&["==", "!=", "<>", "<=", ">=", "<", ">", "=~", "!~"])
        else {
            return Ok(left);
        };
        let right = self.sum()?;
        if matches!(operator, "=~" | "!~") {
            return Err("regular-expression matching is not supported".into());
        }
        // Numbers compare as numbers; anything else compares as text.
        let ordering = match (left.as_f64(), right.as_f64()) {
            (Some(a), Some(b)) => a.partial_cmp(&b),
            _ if matches!(operator, "==" | "!=" | "<>") => {
                Some(left.to_string().cmp(&right.to_string()))
            }
            _ => return Err(format!("cannot order \"{left}\" and \"{right}\"")),
        };
        let ordering = ordering.ok_or("cannot compare a value that is not a number")?;
        Ok(Value::Bool(match operator {
            "==" => ordering.is_eq(),
            "!=" | "<>" => ordering.is_ne(),
            "<" => ordering.is_lt(),
            "<=" => ordering.is_le(),
            ">" => ordering.is_gt(),
            _ => ordering.is_ge(),
        }))
    }

    fn sum(&mut self) -> Result<Value, String> {
        let mut left = self.product()?;
        while let Some(operator) = self.take(&["+", "-"]) {
            let right = self.product()?;
            left = match (left.number(), right.number()) {
                (Some(a), Some(b)) => arithmetic(operator, &a, &b)?,
                // Adding text to anything joins them.
                _ if operator == "+" => Value::Text(format!("{}{}", left.brief(), right.brief())),
                _ => return Err(format!("cannot subtract \"{right}\" from \"{left}\"")),
            };
        }
        Ok(left)
    }

    fn product(&mut self) -> Result<Value, String> {
        let mut left = self.unary()?;
        while let Some(operator) = self.take(&["*", "/", "%"]) {
            let right = self.unary()?;
            let (a, b) = left
                .number()
                .zip(right.number())
                .ok_or_else(|| format!("\"{left}\" {operator} \"{right}\" needs two numbers"))?;
            left = arithmetic(operator, &a, &b)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Value, String> {
        match self.take(&["-", "+", "!", "not"]) {
            Some("-") => {
                let value = self.unary()?;
                let number = value
                    .number()
                    .ok_or_else(|| format!("cannot negate \"{value}\""))?;
                arithmetic("-", &Value::Int(0), &number)
            }
            Some("+") => self.unary(),
            Some(_) => Ok(Value::Bool(!self.unary()?.truthy()?)),
            None => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Value, String> {
        let token = self.peek().cloned().ok_or("the expression ends too soon")?;
        self.position += 1;
        match token {
            Token::Number(value) => Ok(value),
            Token::Text(text) => Ok(Value::Text(text)),
            Token::Symbol("(") => {
                let value = self.ternary()?;
                self.expect(")")?;
                Ok(value)
            }
            Token::Name(name) => match name.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ if self.take(&["("]).is_some() => self.call(&name),
                _ if self.take(&["["]).is_some() => {
                    let index = self.ternary()?;
                    self.expect("]")?;
                    let Some(Value::Int(index)) = index.number() else {
                        return Err(format!(
                            "the index of \"{name}\" is \"{index}\", not a whole number"
                        ));
                    };
                    self.lookup(&name, Some(index))
                }
                _ => self.lookup(&name, None),
            },
            other => Err(format!("unexpected {other}")),
        }
    }

    fn call(&mut self, name: &str) -> Result<Value, String> {
        let mut arguments = Vec::new();
        if self.take(&[")"]).is_none() {
            loop {
                arguments.push(self.ternary()?);
                if self.take(&[","]).is_none() {
                    break;
                }
            }
            self.expect(")")?;
        }
        let numbers: Vec<f64> = arguments
            .iter()
            .map(|argument| {
                argument
                    .as_f64()
                    .ok_or_else(|| format!("{name}() needs numbers, not \"{argument}\""))
            })
            .collect::<Result<_, _>>()?;
        let all_whole = arguments
            .iter()
            .all(|argument| matches!(argument.number(), Some(Value::Int(_))));
        let whole = |value: f64| Value::Int(value as i64);
        match (name, numbers.as_slice()) {
            ("int", [value]) => Ok(whole(value.trunc())),
            ("round", [value]) => Ok(whole(value.round())),
            ("floor", [value]) => Ok(whole(value.floor())),
            ("ceil", [value]) => Ok(whole(value.ceil())),
            ("abs", [value]) if all_whole => Ok(whole(value.abs())),
            ("abs", [value]) => Ok(Value::Float(value.abs())),
            ("min" | "max", [a, b]) => {
                let chosen = if name == "min" { a.min(*b) } else { a.max(*b) };
                Ok(if all_whole {
                    whole(chosen)
                } else {
                    Value::Float(chosen)
                })
            }
            ("int" | "round" | "floor" | "ceil" | "abs" | "min" | "max", _) => {
                Err(format!("{name}() was given {} arguments", numbers.len()))
            }
            _ => Err(format!("unknown function \"{name}\"")),
        }
    }
}

/// Whole numbers stay whole, division included, as in the reference.
fn arithmetic(operator: &str, a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(a), Value::Int(b)) = (a, b) {
        let result = match operator {
            "+" => a.checked_add(*b),
            "-" => a.checked_sub(*b),
            "*" => a.checked_mul(*b),
            "/" => a.checked_div(*b),
            _ => a.checked_rem(*b),
        };
        return result
            .map(Value::Int)
            .ok_or_else(|| format!("{a} {operator} {b} cannot be computed"));
    }
    let (a, b) = (a.as_f64().expect("a number"), b.as_f64().expect("a number"));
    Ok(Value::Float(match operator {
        "+" => a + b,
        "-" => a - b,
        "*" => a * b,
        "/" => a / b,
        _ => a % b,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(source: &str) -> Result<Value, String> {
        let mut variables = Variables::default();
        variables.set("layer_num", Value::Int(4));
        variables.set("max_layer_z", Value::Float(77.85));
        variables.set("bed_type", Value::text("Cool Plate"));
        variables.set("enabled", Value::text("1"));
        variables.set("speed", Value::text("500"));
        variables.set_list(
            "temperature",
            vec![Value::text("220"), Value::text("240.5")],
        );
        evaluate(source, &variables, &mut Overlay::default())
    }

    fn shown(source: &str) -> String {
        value(source).unwrap().to_string()
    }

    #[test]
    fn arithmetic_keeps_whole_numbers_whole() {
        assert_eq!(shown("layer_num+1"), "5");
        assert_eq!(shown("temperature[0] - 90"), "130");
        assert_eq!(shown("temperature[layer_num - 3] - 90"), "150.5");
        assert_eq!(shown("max_layer_z+2"), "79.85");
        assert_eq!(shown("0.25 + 3 * 0.2"), "0.85");
        assert_eq!(shown("0.1 + 0.2"), "0.3");
        assert_eq!(shown("7 / 2"), "3");
        assert_eq!(shown("7 / 2.0"), "3.5");
        assert_eq!(shown("7 % 3 * -2"), "-2");
        assert_eq!(shown("2 + 3 * (4 - 1)"), "11");
        assert_eq!(shown("speed * 60"), "30000");
        assert!(value("1 / 0").is_err());
    }

    #[test]
    fn numbers_are_written_as_a_cpp_stream_writes_them() {
        // Bambu Lab's start G-code: F{filament_max_volumetric_speed / 2.4053 * 60}.
        assert_eq!(shown("21 / 2.4053 * 60"), "523.843");
        assert_eq!(stream_double(6_033.274_122_871_84), "6033.27");
        assert_eq!(stream_double(1234567.0), "1.23457e+06");
        assert_eq!(stream_double(0.00001234), "1.234e-05");
        assert_eq!(stream_double(0.0001234), "0.0001234");
        assert_eq!(stream_double(-2.5), "-2.5");
        assert_eq!(stream_double(100.0), "100");
        assert_eq!(stream_double(999999.6), "1e+06");
    }

    #[test]
    fn comparisons_and_logic() {
        assert_eq!(shown("bed_type==\"Cool Plate\""), "true");
        assert_eq!(
            shown("bed_type != \"Cool Plate\" or layer_num >= 4"),
            "true"
        );
        assert_eq!(shown("not (layer_num < 0) && enabled"), "true");
        assert_eq!(shown("!enabled || speed > 499.5"), "true");
        // Text that reads as numbers compares as numbers.
        assert_eq!(shown("speed == 500.0"), "true");
        assert_eq!(shown("temperature[0] <> temperature[1]"), "true");
        assert!(value("bed_type < 3").is_err());
        assert!(value("bed_type =~ \"Cool\"").is_err());
    }

    #[test]
    fn text_joins_and_ternaries_choose() {
        assert_eq!(
            shown("\"M109 S\" + temperature[0] + \" T\" + layer_num"),
            "M109 S220 T4"
        );
        assert_eq!(shown("layer_num > 2 ? \"high\" : \"low\""), "high");
        assert_eq!(shown("enabled ? 1 : 0"), "1");
    }

    #[test]
    fn functions() {
        assert_eq!(shown("min(layer_num, 2)"), "2");
        assert_eq!(shown("max(max_layer_z, 2)"), "77.85");
        assert_eq!(shown("round(max_layer_z)"), "78");
        assert_eq!(shown("int(max_layer_z) + ceil(0.2) + floor(-0.2)"), "77");
        assert_eq!(shown("abs(-3)"), "3");
        assert!(value("sqrt(4)").unwrap_err().contains("unknown function"));
        assert!(value("min(1)").unwrap_err().contains("1 arguments"));
    }

    #[test]
    fn malformed_expressions_are_errors() {
        for source in [
            "",
            "1 +",
            "(1",
            "1 2",
            "layer_num[",
            "\"open",
            "x = 1",
            "local a; b",
            "layer_num; 2",
            "temperature[1.5]",
        ] {
            assert!(value(source).is_err(), "{source:?} was accepted");
        }
        assert!(value("bed_type + 1 * \"x\"").is_err());
    }
}
