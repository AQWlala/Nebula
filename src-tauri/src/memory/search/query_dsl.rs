//! T-E-B-14: Dataview-style query DSL.
//!
//! A hand-written recursive-descent parser (~350 LOC, no nom/pest)
//! that translates a small query language into a parameterised SQLite
//! `SELECT` against the `memories` table.
//!
//! ## Grammar (BNF)
//!
//! ```bnf
//! <query>    ::= <from_clause> [<where_clause>] [<order_clause>] [<limit_clause>]
//! <from>     ::= "FROM" ("L"<digit> | "*")
//! <where>    ::= "WHERE" <expr>
//! <expr>     ::= <expr> ("AND"|"OR") <expr> | "NOT" <expr> | "(" <expr> ")" | <cmp>
//! <cmp>      ::= <field> <op> <value> | <field> "IN" "(" <val> ("," <val>)* ")"
//! <op>       ::= "=" | "!=" | ">" | ">=" | "<" | "<="
//! <order>    ::= "ORDER" "BY" <field> ["ASC"|"DESC"]
//! <limit>    ::= "LIMIT" <number>
//! ```
//!
//! ## SQL injection prevention
//!
//! * Field names are validated against a fixed whitelist
//!   ([`Field::from_name`]). Unknown fields cause a parse error.
//! * Every value is bound as a positional `?` parameter — no value
//!   text is ever inlined into the SQL string.
//! * `LIKE` patterns (reserved for v2) are escaped via
//!   [`escape_like`] before being bound.
//!
//! ## Force-injected predicates
//!
//! `compressed_from IS NULL` is always AND-ed into the `WHERE` clause
//! so rows absorbed by the black-hole compression engine are hidden
//! from DSL queries.

use rusqlite::types::Value as SqlValue;
use std::str::FromStr;

use crate::memory::sqlite_store::MEMORY_COLUMNS;
use crate::memory::types::{MemoryLayer, MemoryType};

// ===========================================================================
// AST
// ===========================================================================

/// Top-level parsed query.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryAst {
    pub from: LayerSpec,
    pub where_clause: Option<Expr>,
    pub order: Option<OrderClause>,
    pub limit: Option<u32>,
}

/// `FROM L3` vs `FROM *`.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerSpec {
    All,
    Layer(MemoryLayer),
}

/// Boolean expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Cmp(Field, CmpOp, Value),
    In(Field, Vec<Value>),
    /// Bare boolean field: `pinned` ≡ `pinned=true`.
    Bool(Field),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

/// AST-level value (pre-SQL translation).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Number(f64),
    String(String),
    Bool(bool),
}

/// Whitelisted queryable field.
///
/// The variant set is the single source of truth for which column
/// names may appear in `WHERE` / `ORDER BY` clauses. Unknown names
/// are rejected at parse time, which is the primary SQL-injection
/// guard — an attacker cannot smuggle arbitrary SQL through a field
/// name because the name must match one of these variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Kind,
    Layer,
    Importance,
    AccessCount,
    LastAccess,
    CreatedAt,
    Source,
    Pinned,
    Archived,
    Id,
    Content,
    /// M2a #32: domain 字段(memory isolation)。配合 M2b ACL 的
    /// query-time 过滤,允许 DSL 查询显式限定域,例如
    /// `FROM * WHERE domain='agent_a'`。
    Domain,
}

impl Field {
    /// Map a user-supplied name to a [`Field`]. Returns `None` for
    /// unknown names — callers must treat `None` as a hard error to
    /// keep the SQL-injection guard intact.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "kind" | "type" | "memory_type" => Some(Field::Kind),
            "layer" => Some(Field::Layer),
            "importance" => Some(Field::Importance),
            "access_count" => Some(Field::AccessCount),
            "last_access" => Some(Field::LastAccess),
            "created_at" => Some(Field::CreatedAt),
            "source" => Some(Field::Source),
            "pinned" => Some(Field::Pinned),
            "archived" => Some(Field::Archived),
            "id" => Some(Field::Id),
            "content" => Some(Field::Content),
            "domain" => Some(Field::Domain),
            _ => None,
        }
    }

    /// SQLite column name corresponding to this field.
    pub fn column(&self) -> &'static str {
        match self {
            Field::Kind => "memory_type",
            Field::Layer => "layer",
            Field::Importance => "importance",
            Field::AccessCount => "access_count",
            Field::LastAccess => "last_access",
            Field::CreatedAt => "created_at",
            Field::Source => "source",
            Field::Pinned => "pinned",
            Field::Archived => "archived",
            Field::Id => "id",
            Field::Content => "content",
            Field::Domain => "domain",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderClause {
    pub field: Field,
    pub direction: OrderDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDir {
    Asc,
    Desc,
}

// ===========================================================================
// Lexer
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
enum Token {
    // Keywords (case-insensitive).
    From,
    Where,
    And,
    Or,
    Not,
    In,
    Order,
    By,
    Asc,
    Desc,
    Limit,
    // Symbols.
    LParen,
    RParen,
    Comma,
    Star,
    // Operators.
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    // Literals.
    Number(f64),
    String(String),
    Ident(String),
    Eof,
}

struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Lexer { input, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => {
                    tokens.push(Token::Eof);
                    break;
                }
                Some('(') => {
                    self.advance();
                    tokens.push(Token::LParen);
                }
                Some(')') => {
                    self.advance();
                    tokens.push(Token::RParen);
                }
                Some(',') => {
                    self.advance();
                    tokens.push(Token::Comma);
                }
                Some('*') => {
                    self.advance();
                    tokens.push(Token::Star);
                }
                Some('=') => {
                    self.advance();
                    tokens.push(Token::Eq);
                }
                Some('!') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Ne);
                    } else {
                        return Err("expected '!=' after '!'".to_string());
                    }
                }
                Some('>') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Ge);
                    } else {
                        tokens.push(Token::Gt);
                    }
                }
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Le);
                    } else {
                        tokens.push(Token::Lt);
                    }
                }
                Some('\'') => {
                    let s = self.lex_string()?;
                    tokens.push(Token::String(s));
                }
                Some(c) if c.is_ascii_digit() => {
                    let n = self.lex_number()?;
                    tokens.push(Token::Number(n));
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    let ident = self.lex_ident();
                    let token = match ident.to_ascii_lowercase().as_str() {
                        "from" => Token::From,
                        "where" => Token::Where,
                        "and" => Token::And,
                        "or" => Token::Or,
                        "not" => Token::Not,
                        "in" => Token::In,
                        "order" => Token::Order,
                        "by" => Token::By,
                        "asc" => Token::Asc,
                        "desc" => Token::Desc,
                        "limit" => Token::Limit,
                        _ => Token::Ident(ident),
                    };
                    tokens.push(token);
                }
                Some(c) => {
                    return Err(format!("unexpected character: {c:?}"));
                }
            }
        }
        Ok(tokens)
    }

    /// Lex a single-quoted SQL-style string. `''` inside the literal
    /// is an escaped apostrophe.
    fn lex_string(&mut self) -> Result<String, String> {
        self.advance(); // consume opening quote
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string literal".to_string()),
                Some('\'') => {
                    self.advance();
                    if self.peek() == Some('\'') {
                        self.advance();
                        s.push('\'');
                    } else {
                        break;
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
        Ok(s)
    }

    fn lex_number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        let mut has_digits = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                has_digits = true;
                self.advance();
            } else {
                break;
            }
        }
        if self.peek() == Some('.') {
            self.advance();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    has_digits = true;
                    self.advance();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.advance();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.advance();
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        let s = &self.input[start..self.pos];
        if !has_digits {
            return Err(format!("invalid number: {s}"));
        }
        s.parse::<f64>()
            .map_err(|e| format!("invalid number {s:?}: {e}"))
    }

    fn lex_ident(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }
        self.input[start..self.pos].to_string()
    }
}

// ===========================================================================
// Parser (recursive descent)
// ===========================================================================

/// Parse a query string into a [`QueryAst`].
///
/// This is the main entry point used by the `memory_query_dsl` Tauri
/// command. Parse errors (unknown fields, unexpected tokens, trailing
/// garbage) are returned as a human-readable `String`.
///
/// Example:
/// ```
/// use nebula_lib::memory::query_dsl::parse_str;
/// # fn main() -> Result<(), String> {
/// let ast = parse_str("FROM L3 WHERE kind=fact AND importance>0.7")?;
/// # Ok(())
/// # }
/// ```
pub fn parse_str(input: &str) -> Result<QueryAst, String> {
    let tokens = Lexer::new(input).tokenize()?;
    Parser::new(tokens).parse()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    /// Lex + parse a query string into a [`QueryAst`].
    ///
    /// Kept as an associated function for callers that already hold a
    /// `Parser` reference (e.g. internal tests). External callers
    /// should prefer the free function [`parse_str`].
    #[cfg(test)]
    fn parse_str(input: &str) -> Result<QueryAst, String> {
        let tokens = Lexer::new(input).tokenize()?;
        Parser::new(tokens).parse()
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "expected {:?} but found {:?}",
                expected,
                self.peek()
            ))
        }
    }

    fn parse(&mut self) -> Result<QueryAst, String> {
        let from = self.parse_from()?;
        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        let order = if matches!(self.peek(), Token::Order) {
            Some(self.parse_order()?)
        } else {
            None
        };
        let limit = if matches!(self.peek(), Token::Limit) {
            Some(self.parse_limit()?)
        } else {
            None
        };
        if !matches!(self.peek(), Token::Eof) {
            return Err(format!("unexpected trailing token: {:?}", self.peek()));
        }
        Ok(QueryAst {
            from,
            where_clause,
            order,
            limit,
        })
    }

    fn parse_from(&mut self) -> Result<LayerSpec, String> {
        self.expect(&Token::From)?;
        match self.advance() {
            Token::Star => Ok(LayerSpec::All),
            Token::Ident(s) => {
                let layer = MemoryLayer::from_str(&s).map_err(|e| format!("FROM clause: {e}"))?;
                Ok(LayerSpec::Layer(layer))
            }
            other => Err(format!(
                "expected layer (L0-L7) or '*' after FROM, found {:?}",
                other
            )),
        }
    }

    /// `<expr> ::= <or_expr>`
    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    /// Lowest precedence: OR.
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Middle precedence: AND.
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), Token::And) {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Higher precedence: NOT.
    fn parse_not(&mut self) -> Result<Expr, String> {
        if matches!(self.peek(), Token::Not) {
            self.advance();
            let inner = self.parse_not()?;
            Ok(Expr::Not(Box::new(inner)))
        } else {
            self.parse_primary()
        }
    }

    /// Highest precedence: parenthesised expr, or a comparison.
    fn parse_primary(&mut self) -> Result<Expr, String> {
        if matches!(self.peek(), Token::LParen) {
            self.advance();
            let e = self.parse_expr()?;
            self.expect(&Token::RParen)?;
            return Ok(e);
        }
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let field = self.parse_field()?;
        // Bare boolean field: `pinned` with no operator.
        if matches!(
            self.peek(),
            Token::And | Token::Or | Token::RParen | Token::Eof | Token::Order | Token::Limit
        ) {
            return Ok(Expr::Bool(field));
        }
        // IN clause.
        if matches!(self.peek(), Token::In) {
            self.advance();
            self.expect(&Token::LParen)?;
            let mut values = Vec::new();
            values.push(self.parse_value()?);
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                values.push(self.parse_value()?);
            }
            self.expect(&Token::RParen)?;
            return Ok(Expr::In(field, values));
        }
        // Comparison operator.
        let op = self.parse_op()?;
        let value = self.parse_value()?;
        Ok(Expr::Cmp(field, op, value))
    }

    fn parse_field(&mut self) -> Result<Field, String> {
        match self.advance() {
            Token::Ident(s) => Field::from_name(&s).ok_or_else(|| format!("unknown field: {s}")),
            other => Err(format!("expected field name, found {:?}", other)),
        }
    }

    fn parse_op(&mut self) -> Result<CmpOp, String> {
        match self.advance() {
            Token::Eq => Ok(CmpOp::Eq),
            Token::Ne => Ok(CmpOp::Ne),
            Token::Gt => Ok(CmpOp::Gt),
            Token::Ge => Ok(CmpOp::Ge),
            Token::Lt => Ok(CmpOp::Lt),
            Token::Le => Ok(CmpOp::Le),
            other => Err(format!("expected comparison operator, found {:?}", other)),
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        match self.advance() {
            Token::Number(n) => Ok(Value::Number(n)),
            Token::String(s) => Ok(Value::String(s)),
            Token::Ident(s) => match s.to_ascii_lowercase().as_str() {
                "null" => Ok(Value::Null),
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Ok(Value::String(s)),
            },
            other => Err(format!("expected value, found {:?}", other)),
        }
    }

    fn parse_order(&mut self) -> Result<OrderClause, String> {
        self.expect(&Token::Order)?;
        self.expect(&Token::By)?;
        let field = self.parse_field()?;
        let direction = match self.peek() {
            Token::Asc => {
                self.advance();
                OrderDir::Asc
            }
            Token::Desc => {
                self.advance();
                OrderDir::Desc
            }
            _ => OrderDir::Desc,
        };
        Ok(OrderClause { field, direction })
    }

    fn parse_limit(&mut self) -> Result<u32, String> {
        self.expect(&Token::Limit)?;
        match self.advance() {
            Token::Number(n) if n >= 0.0 => Ok(n as u32),
            other => Err(format!(
                "expected non-negative number after LIMIT, found {:?}",
                other
            )),
        }
    }
}

// ===========================================================================
// Translator (AST → SQL + bound params)
// ===========================================================================

/// Translate a parsed [`QueryAst`] into a parameterised SQL `SELECT`
/// against the `memories` table, together with the bound parameter
/// values in positional order.
///
/// Invariants:
/// * No user-supplied text is ever inlined into the SQL string —
///   every value becomes a `?` placeholder.
/// * `compressed_from IS NULL` is always AND-ed into the `WHERE`
///   clause.
/// * When no `ORDER BY` is present, `ORDER BY created_at DESC` is
///   synthesised.
/// * When no `LIMIT` is present, `LIMIT 100` is synthesised.
pub fn translate(ast: &QueryAst) -> (String, Vec<SqlValue>) {
    let mut params: Vec<SqlValue> = Vec::new();
    let mut where_parts: Vec<String> = Vec::new();

    // Layer filter from FROM clause.
    if let LayerSpec::Layer(layer) = &ast.from {
        params.push(SqlValue::Text(layer.as_str().to_string()));
        where_parts.push("layer = ?".to_string());
    }

    // Force-inject compressed_from IS NULL to hide black-hole rows.
    where_parts.push("compressed_from IS NULL".to_string());

    // User WHERE clause.
    if let Some(expr) = &ast.where_clause {
        let sql = translate_expr(expr, &mut params);
        where_parts.push(format!("({sql})"));
    }

    // ORDER BY (default: created_at DESC).
    let order_sql = match &ast.order {
        Some(OrderClause { field, direction }) => {
            let dir = match direction {
                OrderDir::Asc => "ASC",
                OrderDir::Desc => "DESC",
            };
            format!(" ORDER BY {} {}", field.column(), dir)
        }
        None => " ORDER BY created_at DESC".to_string(),
    };

    // LIMIT (default: 100).
    let limit = ast.limit.unwrap_or(100);
    params.push(SqlValue::Integer(limit as i64));

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_parts.join(" AND "))
    };

    let sql = format!(
        "SELECT {} FROM memories{}{} LIMIT ?",
        MEMORY_COLUMNS, where_sql, order_sql
    );

    (sql, params)
}

fn translate_expr(expr: &Expr, params: &mut Vec<SqlValue>) -> String {
    match expr {
        Expr::And(a, b) => {
            let a_sql = translate_expr(a, params);
            let b_sql = translate_expr(b, params);
            format!("({a_sql} AND {b_sql})")
        }
        Expr::Or(a, b) => {
            let a_sql = translate_expr(a, params);
            let b_sql = translate_expr(b, params);
            format!("({a_sql} OR {b_sql})")
        }
        Expr::Not(e) => {
            let sql = translate_expr(e, params);
            format!("NOT ({sql})")
        }
        Expr::Cmp(field, op, value) => translate_cmp(field, *op, value, params),
        Expr::In(field, values) => {
            let placeholders: Vec<String> = values
                .iter()
                .map(|v| {
                    params.push(value_to_param(field, v));
                    "?".to_string()
                })
                .collect();
            format!("{} IN ({})", field.column(), placeholders.join(", "))
        }
        Expr::Bool(field) => {
            params.push(SqlValue::Integer(1));
            format!("{} = ?", field.column())
        }
    }
}

fn translate_cmp(field: &Field, op: CmpOp, value: &Value, params: &mut Vec<SqlValue>) -> String {
    let col = field.column();
    match value {
        Value::Null => match op {
            CmpOp::Eq => format!("{col} IS NULL"),
            CmpOp::Ne => format!("{col} IS NOT NULL"),
            // NULL ordering comparisons are always unknown in SQL →
            // collapse to `1 = 0` (always false).
            _ => "1 = 0".to_string(),
        },
        _ => {
            params.push(value_to_param(field, value));
            let op_str = match op {
                CmpOp::Eq => "=",
                CmpOp::Ne => "!=",
                CmpOp::Gt => ">",
                CmpOp::Ge => ">=",
                CmpOp::Lt => "<",
                CmpOp::Le => "<=",
            };
            format!("{col} {op_str} ?")
        }
    }
}

/// Convert an AST [`Value`] to a SQLite-bound [`SqlValue`], applying
/// field-specific coercions:
/// * `kind` strings are aliased (`fact` → `semantic`, …).
/// * `pinned` / `archived` booleans become `0` / `1` integers.
fn value_to_param(field: &Field, value: &Value) -> SqlValue {
    match (field, value) {
        (Field::Kind, Value::String(s)) => {
            // Bind the lowercased string to a local so the match arms
            // can return `&str` borrows without dangling.
            let lower = s.to_ascii_lowercase();
            let mapped: &str = match lower.as_str() {
                "fact" => MemoryType::Semantic.as_str(),
                "event" => MemoryType::Episodic.as_str(),
                "skill" => MemoryType::Procedural.as_str(),
                "feeling" => MemoryType::Emotional.as_str(),
                "meta" => MemoryType::Metacognitive.as_str(),
                other => other,
            };
            SqlValue::Text(mapped.to_string())
        }
        (Field::Pinned | Field::Archived, Value::Bool(b)) => {
            SqlValue::Integer(if *b { 1 } else { 0 })
        }
        (Field::Pinned | Field::Archived, Value::String(s)) => {
            let lower = s.to_ascii_lowercase();
            let b = matches!(lower.as_str(), "true" | "1" | "yes");
            SqlValue::Integer(if b { 1 } else { 0 })
        }
        (Field::Pinned | Field::Archived, Value::Number(n)) => {
            SqlValue::Integer(if *n != 0.0 { 1 } else { 0 })
        }
        _ => match value {
            Value::Null => SqlValue::Null,
            Value::Number(n) => SqlValue::Real(*n),
            Value::String(s) => SqlValue::Text(s.clone()),
            Value::Bool(b) => SqlValue::Integer(if *b { 1 } else { 0 }),
        },
    }
}

/// Escape a `LIKE` pattern by prefixing `%`, `_`, and `\` with a
/// backslash. Intended to be used with `ESCAPE '\'` in the generated
/// SQL. Reserved for v2 `LIKE` operator support; included now so the
/// escape semantics are tested and ready.
#[allow(dead_code)]
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Lexer tests ----

    #[test]
    fn test_lexer_basic() {
        let tokens = Lexer::new("FROM L3 WHERE kind=fact")
            .tokenize()
            .expect("create should succeed");
        // FROM, L3, WHERE, kind, =, fact, Eof → 7 tokens.
        assert_eq!(tokens.len(), 7);
        assert!(matches!(tokens[0], Token::From));
        assert!(matches!(tokens[6], Token::Eof));
    }

    #[test]
    fn test_lexer_keywords_case_insensitive() {
        let tokens = Lexer::new("from L3 where kind=fact")
            .tokenize()
            .expect("create should succeed");
        assert!(matches!(tokens[0], Token::From));
        assert!(matches!(tokens[2], Token::Where));

        let tokens = Lexer::new("FROM L3 WHERE KIND=FACT")
            .tokenize()
            .expect("create should succeed");
        assert!(matches!(tokens[0], Token::From));
        assert!(matches!(tokens[2], Token::Where));
    }

    #[test]
    fn test_lexer_operators() {
        let tokens = Lexer::new("a = b != c > d >= e < f <= g")
            .tokenize()
            .expect("test op should succeed");
        let ops: Vec<_> = tokens
            .iter()
            .filter(|t| {
                matches!(
                    t,
                    Token::Eq | Token::Ne | Token::Gt | Token::Ge | Token::Lt | Token::Le
                )
            })
            .collect();
        assert_eq!(ops.len(), 6);
    }

    #[test]
    fn test_lexer_string_literal() {
        let tokens = Lexer::new("'hello world'")
            .tokenize()
            .expect("create should succeed");
        match &tokens[0] {
            Token::String(s) => assert_eq!(s, "hello world"),
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn test_lexer_string_escape() {
        // SQL-style: '' inside a string is a literal apostrophe.
        let tokens = Lexer::new("'it''s'")
            .tokenize()
            .expect("create should succeed");
        match &tokens[0] {
            Token::String(s) => assert_eq!(s, "it's"),
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn test_lexer_number() {
        let tokens = Lexer::new("0.7 42 1e5")
            .tokenize()
            .expect("create should succeed");
        match &tokens[0] {
            Token::Number(n) => assert!((n - 0.7).abs() < 1e-9),
            other => panic!("expected Number, got {other:?}"),
        }
        match &tokens[1] {
            Token::Number(n) => assert_eq!(*n as i64, 42),
            other => panic!("expected Number, got {other:?}"),
        }
        match &tokens[2] {
            Token::Number(n) => assert_eq!(*n as i64, 100000),
            other => panic!("expected Number, got {other:?}"),
        }
    }

    #[test]
    fn test_lexer_unterminated_string_errors() {
        let result = Lexer::new("'unterminated").tokenize();
        assert!(result.is_err());
    }

    #[test]
    fn test_lexer_unexpected_char_errors() {
        let result = Lexer::new("FROM L3 @").tokenize();
        assert!(result.is_err());
    }

    // ---- Parser tests ----

    #[test]
    fn test_parser_simple_query() {
        let ast = Parser::parse_str("FROM L3").expect("parse should succeed");
        assert_eq!(ast.from, LayerSpec::Layer(MemoryLayer::L3));
        assert!(ast.where_clause.is_none());
        assert!(ast.order.is_none());
        assert!(ast.limit.is_none());
    }

    #[test]
    fn test_parser_from_star() {
        let ast = Parser::parse_str("FROM *").expect("parse should succeed");
        assert_eq!(ast.from, LayerSpec::All);
    }

    #[test]
    fn test_parser_where_clause() {
        let ast = Parser::parse_str("FROM L3 WHERE kind=fact").expect("parse should succeed");
        let expr = ast.where_clause.expect("where clause");
        match expr {
            Expr::Cmp(Field::Kind, CmpOp::Eq, Value::String(s)) => {
                assert_eq!(s, "fact");
            }
            other => panic!("expected Cmp(Kind, Eq, String), got {other:?}"),
        }
    }

    #[test]
    fn test_parser_in_clause() {
        let ast =
            Parser::parse_str("FROM L3 WHERE layer IN (L3, L4, L5)").expect("parse should succeed");
        let expr = ast.where_clause.expect("where clause");
        match expr {
            Expr::In(Field::Layer, values) => {
                assert_eq!(values.len(), 3);
            }
            other => panic!("expected In(Field::Layer, ...), got {other:?}"),
        }
    }

    #[test]
    fn test_parser_in_clause_with_strings() {
        let ast = Parser::parse_str("FROM * WHERE kind IN ('fact', 'event')")
            .expect("parse should succeed");
        let expr = ast.where_clause.expect("where clause");
        match expr {
            Expr::In(Field::Kind, values) => {
                assert_eq!(values.len(), 2);
            }
            other => panic!("expected In(Field::Kind, ...), got {other:?}"),
        }
    }

    #[test]
    fn test_parser_not_and_or_precedence() {
        // NOT a AND b OR c  ==  ((NOT a) AND b) OR c
        let ast = Parser::parse_str("FROM * WHERE NOT pinned AND archived OR kind=fact")
            .expect("parse should succeed");
        let expr = ast.where_clause.expect("where clause");
        match expr {
            Expr::Or(left, right) => {
                match *left {
                    Expr::And(not_expr, _) => {
                        assert!(matches!(*not_expr, Expr::Not(_)));
                    }
                    other => panic!("expected And(Not, ...) on the left, got {other:?}"),
                }
                assert!(matches!(*right, Expr::Cmp(Field::Kind, CmpOp::Eq, _)));
            }
            other => panic!("expected Or at the top, got {other:?}"),
        }
    }

    #[test]
    fn test_parser_parens_override_precedence() {
        // a OR (b AND c) — the AND must be grouped inside the OR's right.
        let ast = Parser::parse_str("FROM * WHERE kind=fact OR (kind=event AND importance>0.5)")
            .expect("parse should succeed");
        let expr = ast.where_clause.expect("where clause");
        match expr {
            Expr::Or(_, right) => {
                assert!(matches!(*right, Expr::And(_, _)));
            }
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn test_parser_order_limit() {
        let ast = Parser::parse_str("FROM L3 ORDER BY created_at ASC LIMIT 50")
            .expect("create should succeed");
        let order = ast.order.expect("order clause");
        assert_eq!(order.field, Field::CreatedAt);
        assert_eq!(order.direction, OrderDir::Asc);
        assert_eq!(ast.limit, Some(50));
    }

    #[test]
    fn test_parser_order_default_desc() {
        let ast = Parser::parse_str("FROM L3 ORDER BY importance").expect("parse should succeed");
        let order = ast.order.expect("order clause");
        assert_eq!(order.field, Field::Importance);
        assert_eq!(order.direction, OrderDir::Desc);
    }

    #[test]
    fn test_parser_bare_bool_field() {
        let ast = Parser::parse_str("FROM * WHERE pinned").expect("parse should succeed");
        match ast.where_clause.expect("where clause") {
            Expr::Bool(Field::Pinned) => {}
            other => panic!("expected Bool(Pinned), got {other:?}"),
        }
    }

    #[test]
    fn test_parser_null_value() {
        let ast = Parser::parse_str("FROM * WHERE last_access=NULL").expect("parse should succeed");
        match ast.where_clause.expect("where clause") {
            Expr::Cmp(Field::LastAccess, CmpOp::Eq, Value::Null) => {}
            other => panic!("expected Cmp(LastAccess, Eq, Null), got {other:?}"),
        }
    }

    #[test]
    fn test_parser_unknown_layer_errors() {
        let result = Parser::parse_str("FROM L9");
        assert!(result.is_err());
    }

    #[test]
    fn test_parser_missing_from_errors() {
        let result = Parser::parse_str("WHERE kind=fact");
        assert!(result.is_err());
    }

    #[test]
    fn test_parser_unknown_field_errors() {
        let result = Parser::parse_str("FROM L3 WHERE evil_col=1");
        assert!(result.is_err());
    }

    #[test]
    fn test_parser_trailing_garbage_errors() {
        let result = Parser::parse_str("FROM L3 garbage");
        assert!(result.is_err());
    }

    // ---- Translator tests ----

    #[test]
    fn test_translate_sql_generation() {
        let ast = Parser::parse_str("FROM L3 WHERE kind=fact AND importance>0.7")
            .expect("parse should succeed");
        let (sql, params) = translate(&ast);
        assert!(sql.starts_with("SELECT "));
        assert!(sql.contains("FROM memories"));
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("layer = ?"));
        assert!(sql.contains("compressed_from IS NULL"));
        assert!(sql.contains("memory_type = ?"));
        assert!(sql.contains("importance > ?"));
        assert!(sql.contains("ORDER BY created_at DESC"));
        assert!(sql.contains("LIMIT ?"));
        // params: ["L3", "semantic", 0.7, 100]
        assert_eq!(params.len(), 4);
        assert_eq!(params[0], SqlValue::Text("L3".to_string()));
        assert_eq!(params[1], SqlValue::Text("semantic".to_string()));
        assert!(matches!(params[2], SqlValue::Real(r) if (r - 0.7).abs() < 1e-9));
        assert_eq!(params[3], SqlValue::Integer(100));
    }

    #[test]
    fn test_translate_params_binding() {
        let ast = Parser::parse_str("FROM L1 LIMIT 25").expect("parse should succeed");
        let (_, params) = translate(&ast);
        // layer + limit
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], SqlValue::Text("L1".to_string()));
        assert_eq!(params[1], SqlValue::Integer(25));
    }

    #[test]
    fn test_translate_injects_compressed_from_null() {
        let ast = Parser::parse_str("FROM L3").expect("parse should succeed");
        let (sql, _) = translate(&ast);
        assert!(sql.contains("compressed_from IS NULL"));
    }

    #[test]
    fn test_translate_injects_compressed_from_null_with_where() {
        let ast = Parser::parse_str("FROM L3 WHERE kind=fact").expect("parse should succeed");
        let (sql, _) = translate(&ast);
        assert!(sql.contains("compressed_from IS NULL"));
        assert!(sql.contains("memory_type = ?"));
    }

    #[test]
    fn test_translate_kind_alias() {
        let cases = [
            ("fact", "semantic"),
            ("event", "episodic"),
            ("skill", "procedural"),
            ("feeling", "emotional"),
            ("meta", "metacognitive"),
        ];
        for (alias, expected) in cases {
            let ast = Parser::parse_str(&format!("FROM * WHERE kind={alias}"))
                .expect("parse should succeed");
            let (_, params) = translate(&ast);
            // params: [kind_value, limit]
            assert_eq!(
                params[0],
                SqlValue::Text(expected.to_string()),
                "alias {alias} should map to {expected}"
            );
        }
    }

    #[test]
    fn test_translate_kind_passthrough() {
        // Direct memory type names pass through unchanged.
        let ast = Parser::parse_str("FROM * WHERE kind=semantic").expect("parse should succeed");
        let (_, params) = translate(&ast);
        assert_eq!(params[0], SqlValue::Text("semantic".to_string()));
    }

    #[test]
    fn test_translate_default_order_and_limit() {
        let ast = Parser::parse_str("FROM *").expect("parse should succeed");
        let (sql, params) = translate(&ast);
        assert!(sql.contains("ORDER BY created_at DESC"));
        assert!(sql.contains("LIMIT ?"));
        assert_eq!(params.last(), Some(&SqlValue::Integer(100)));
    }

    #[test]
    fn test_translate_explicit_order_and_limit() {
        let ast = Parser::parse_str("FROM * ORDER BY importance ASC LIMIT 10")
            .expect("parse should succeed");
        let (sql, params) = translate(&ast);
        assert!(sql.contains("ORDER BY importance ASC"));
        assert!(sql.contains("LIMIT ?"));
        assert_eq!(params.last(), Some(&SqlValue::Integer(10)));
    }

    #[test]
    fn test_translate_pinned_bool_to_int() {
        let ast = Parser::parse_str("FROM * WHERE pinned=true").expect("parse should succeed");
        let (_, params) = translate(&ast);
        // params: [pinned_int, limit]
        assert_eq!(params[0], SqlValue::Integer(1));
    }

    #[test]
    fn test_translate_pinned_false_to_int() {
        let ast = Parser::parse_str("FROM * WHERE archived=false").expect("parse should succeed");
        let (_, params) = translate(&ast);
        assert_eq!(params[0], SqlValue::Integer(0));
    }

    #[test]
    fn test_translate_bare_bool() {
        let ast = Parser::parse_str("FROM * WHERE pinned").expect("parse should succeed");
        let (_, params) = translate(&ast);
        assert_eq!(params[0], SqlValue::Integer(1));
    }

    #[test]
    fn test_translate_in_clause_params() {
        let ast =
            Parser::parse_str("FROM * WHERE layer IN (L3, L4, L5)").expect("parse should succeed");
        let (sql, params) = translate(&ast);
        assert!(sql.contains("IN (?, ?, ?)"));
        // params: [L3, L4, L5, limit]
        assert_eq!(params.len(), 4);
        assert_eq!(params[0], SqlValue::Text("L3".to_string()));
        assert_eq!(params[1], SqlValue::Text("L4".to_string()));
        assert_eq!(params[2], SqlValue::Text("L5".to_string()));
    }

    #[test]
    fn test_translate_null_handling() {
        // = NULL → IS NULL
        let ast = Parser::parse_str("FROM * WHERE last_access=NULL").expect("parse should succeed");
        let (sql, _) = translate(&ast);
        assert!(sql.contains("last_access IS NULL"));

        // != NULL → IS NOT NULL
        let ast =
            Parser::parse_str("FROM * WHERE last_access!=NULL").expect("parse should succeed");
        let (sql, _) = translate(&ast);
        assert!(sql.contains("last_access IS NOT NULL"));

        // > NULL → 1 = 0 (always false)
        let ast = Parser::parse_str("FROM * WHERE last_access>NULL").expect("parse should succeed");
        let (sql, _) = translate(&ast);
        assert!(sql.contains("1 = 0"));
    }

    #[test]
    fn test_translate_not_and_or_sql() {
        let ast = Parser::parse_str("FROM * WHERE NOT pinned AND archived OR kind=fact")
            .expect("parse should succeed");
        let (sql, _) = translate(&ast);
        // NOT pinned → NOT (pinned = ?)
        assert!(sql.contains("NOT"));
        assert!(sql.contains("AND"));
        assert!(sql.contains("OR"));
    }

    // ---- SQL injection prevention ----

    #[test]
    fn test_sql_injection_prevention_unknown_field() {
        // A field name containing SQL keywords must be rejected.
        let result = Parser::parse_str("FROM L3 WHERE content; DROP TABLE memories-- =1");
        assert!(result.is_err());
    }

    #[test]
    fn test_sql_injection_prevention_value_is_parametrised() {
        let ast = Parser::parse_str("FROM L3 WHERE content='; DROP TABLE memories--'")
            .expect("delete should succeed");
        let (sql, params) = translate(&ast);
        // The dangerous text must NOT appear in the SQL string.
        assert!(!sql.contains("DROP TABLE"));
        assert!(!sql.contains(";"));
        // It must be bound as a parameter.
        assert!(sql.contains("?"));
        // The params vector must contain the dangerous text verbatim.
        let found = params
            .iter()
            .any(|p| matches!(p, SqlValue::Text(s) if s == "; DROP TABLE memories--"));
        assert!(found, "dangerous text must be in params, got: {params:?}");
    }

    #[test]
    fn test_sql_injection_prevention_layer_from() {
        // FROM clause only accepts L0-L7 — arbitrary SQL is rejected.
        let result = Parser::parse_str("FROM L3; DROP TABLE memories");
        assert!(result.is_err());
    }

    // ---- LIKE escaping (utility tested for future v2 LIKE support) ----

    #[test]
    fn test_like_escaping() {
        // % and _ are prefixed with a backslash.
        let pattern = escape_like("50%_off");
        assert_eq!(pattern, r"50\%\_off");
    }

    #[test]
    fn test_like_escaping_backslash() {
        // A single backslash in the input is doubled.
        // Input string literal "a\\b" represents the 3-char string a\b.
        let pattern = escape_like("a\\b");
        // Expected output: a\\b (4 chars: a, \, \, b).
        // In Rust source that 4-char string is written "a\\\\b".
        assert_eq!(pattern, "a\\\\b");
    }

    #[test]
    fn test_like_escaping_no_special_chars() {
        let pattern = escape_like("hello world");
        assert_eq!(pattern, "hello world");
    }

    // ---- End-to-end translator tests ----

    #[test]
    fn test_translate_from_star_no_layer_filter() {
        let ast = Parser::parse_str("FROM *").expect("parse should succeed");
        let (sql, params) = translate(&ast);
        // FROM * must NOT add a layer = ? predicate.
        assert!(!sql.contains("layer = ?"));
        // But compressed_from IS NULL must still be present.
        assert!(sql.contains("compressed_from IS NULL"));
        // Only the LIMIT param.
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_translate_preserves_field_whitelist_in_order() {
        // ORDER BY only accepts whitelisted fields.
        let result = Parser::parse_str("FROM * ORDER BY evil_col");
        assert!(result.is_err());
    }

    #[test]
    fn test_translate_full_query() {
        let ast = Parser::parse_str(
            "FROM L4 WHERE kind=event AND importance>=0.5 OR pinned ORDER BY created_at ASC LIMIT 50",
        )
        .expect("test op should succeed");
        let (sql, params) = translate(&ast);
        assert!(sql.contains("layer = ?"));
        assert!(sql.contains("compressed_from IS NULL"));
        assert!(sql.contains("memory_type = ?"));
        assert!(sql.contains("importance >= ?"));
        assert!(sql.contains("pinned = ?"));
        assert!(sql.contains("ORDER BY created_at ASC"));
        assert!(sql.contains("LIMIT ?"));
        // params: [L4, episodic, 0.5, 1 (pinned=true via Bool), 50]
        assert_eq!(params.len(), 5);
        assert_eq!(params[0], SqlValue::Text("L4".to_string()));
        assert_eq!(params[1], SqlValue::Text("episodic".to_string()));
        assert!(matches!(params[2], SqlValue::Real(r) if (r - 0.5).abs() < 1e-9));
        assert_eq!(params[3], SqlValue::Integer(1));
        assert_eq!(params[4], SqlValue::Integer(50));
    }
}
