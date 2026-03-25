use std::ops::Range;
use tree_sitter::{Language, Node, Parser, Tree};

// ─── Token kinds (shared across languages) ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Type,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Identifier,
    Function,
    Macro,
    Attribute,
    Lifetime,
    Error,
    Plain,
}

#[derive(Debug, Clone)]
pub struct SyntaxToken {
    pub byte_range: Range<usize>,
    pub kind: TokenKind,
}

// ─── Supported languages ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxLanguage {
    Sql,
    Rust,
}

impl SyntaxLanguage {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Sql => "SQL",
            Self::Rust => "Rust",
        }
    }
}

// ─── Highlighter ──────────────────────────────────────────────────────────────

pub struct Highlighter {
    parser: Parser,
    tree: Option<Tree>,
    pub tokens: Vec<SyntaxToken>,
    pub language: SyntaxLanguage,
}

impl Highlighter {
    pub fn new(language: SyntaxLanguage) -> Self {
        let mut parser = Parser::new();
        let ts_lang = match language {
            SyntaxLanguage::Sql => Language::from(tree_sitter_sql::LANGUAGE),
            SyntaxLanguage::Rust => Language::from(tree_sitter_rust::LANGUAGE),
        };
        parser.set_language(&ts_lang).expect("failed to set language");
        Self {
            parser,
            tree: None,
            tokens: Vec::new(),
            language,
        }
    }

    pub fn parse(&mut self, text: &str) {
        self.tree = self.parser.parse(text, None);
        self.rehighlight();
    }

    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    fn rehighlight(&mut self) {
        self.tokens.clear();
        if let Some(ref tree) = self.tree {
            self.walk(tree.root_node());
        }
    }

    fn walk(&mut self, node: Node) {
        if node.child_count() == 0 {
            let kind = match self.language {
                SyntaxLanguage::Sql => classify_sql(&node),
                SyntaxLanguage::Rust => classify_rust(&node),
            };
            self.tokens.push(SyntaxToken {
                byte_range: node.byte_range(),
                kind,
            });
        } else {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    self.walk(child);
                }
            }
        }
    }
}

// ─── SQL classifier ───────────────────────────────────────────────────────────

fn classify_sql(node: &Node) -> TokenKind {
    let kind = node.kind();
    if node.is_error() || node.is_missing() {
        return TokenKind::Error;
    }
    if kind.starts_with("keyword_") {
        return TokenKind::Keyword;
    }

    match kind.to_uppercase().as_str() {
        "SELECT" | "FROM" | "WHERE" | "INSERT" | "UPDATE" | "DELETE" | "CREATE" | "DROP"
        | "ALTER" | "TABLE" | "INDEX" | "INTO" | "VALUES" | "SET" | "JOIN" | "LEFT" | "RIGHT"
        | "INNER" | "OUTER" | "CROSS" | "ON" | "AND" | "OR" | "NOT" | "IN" | "IS" | "NULL"
        | "AS" | "ORDER" | "BY" | "GROUP" | "HAVING" | "LIMIT" | "OFFSET" | "UNION"
        | "EXCEPT" | "INTERSECT" | "ALL" | "DISTINCT" | "EXISTS" | "BETWEEN" | "LIKE"
        | "CASE" | "WHEN" | "THEN" | "ELSE" | "END" | "BEGIN" | "COMMIT" | "ROLLBACK"
        | "TRANSACTION" | "IF" | "REPLACE" | "WITH" | "RECURSIVE" | "ASC" | "DESC"
        | "PRIMARY" | "KEY" | "FOREIGN" | "REFERENCES" | "CONSTRAINT" | "DEFAULT"
        | "UNIQUE" | "CHECK" | "CASCADE" | "RETURNING" | "USING" | "OVER" | "PARTITION"
        | "WINDOW" | "ROWS" | "RANGE" | "UNBOUNDED" | "PRECEDING" | "FOLLOWING" | "CURRENT"
        | "ROW" | "GRANT" | "REVOKE" | "TRUE" | "FALSE" | "VIEW" | "TRIGGER" | "FUNCTION"
        | "PROCEDURE" | "SCHEMA" | "DATABASE" | "USE" | "SHOW" | "DESCRIBE" | "EXPLAIN"
        | "ANALYZE" | "VACUUM" | "TRUNCATE" | "RENAME" | "TO" | "ADD" | "COLUMN" | "TEMP"
        | "TEMPORARY" | "MATERIALIZED" | "LATERAL" | "NATURAL" | "FULL" | "ILIKE"
        | "SIMILAR" | "ANY" | "SOME" | "COALESCE" | "NULLIF" | "CAST" | "EXTRACT"
        | "POSITION" | "SUBSTRING" | "TRIM" | "OVERLAY" | "PLACING" | "COLLATE" => {
            return TokenKind::Keyword;
        }
        _ => {}
    }
    match kind.to_uppercase().as_str() {
        "INT" | "INTEGER" | "BIGINT" | "SMALLINT" | "TINYINT" | "FLOAT" | "DOUBLE" | "REAL"
        | "DECIMAL" | "NUMERIC" | "BOOLEAN" | "BOOL" | "CHAR" | "VARCHAR" | "TEXT" | "BLOB"
        | "DATE" | "TIME" | "TIMESTAMP" | "DATETIME" | "INTERVAL" | "UUID" | "JSON"
        | "JSONB" | "SERIAL" | "BIGSERIAL" | "BYTEA" | "ARRAY" | "MONEY" | "INET"
        | "TIMESTAMPTZ" | "TIMETZ" | "INT2" | "INT4" | "INT8" | "FLOAT4" | "FLOAT8" => {
            return TokenKind::Type;
        }
        _ => {}
    }
    match kind {
        "string" | "literal_string" | "single_quoted_string" | "double_quoted_string"
        | "dollar_quoted_string" => TokenKind::String,
        "number" | "literal_number" | "integer" | "float" | "numeric" => TokenKind::Number,
        "comment" | "line_comment" | "block_comment" | "marginalia" => TokenKind::Comment,
        "(" | ")" | "," | ";" | "." | "[" | "]" | "{" | "}" | "::" => TokenKind::Punctuation,
        "=" | "!=" | "<>" | "<" | ">" | "<=" | ">=" | "+" | "-" | "*" | "/" | "%" | "||"
        | "~" | "&" | "|" | "^" | "=>" | "->" | "->>" | "@>" | "<@" => TokenKind::Operator,
        "identifier" | "object_reference" | "field" | "column" | "table" | "column_name"
        | "table_name" | "schema_name" | "alias" => TokenKind::Identifier,
        "invocation" | "function_call" | "function_name" => TokenKind::Function,
        "type" | "type_name" | "data_type" | "column_type" => TokenKind::Type,
        "ERROR" => TokenKind::Error,
        _ => {
            if let Some(p) = node.parent() {
                match p.kind() {
                    "invocation" | "function_call" | "function_name" => TokenKind::Function,
                    "type" | "type_name" | "data_type" | "column_type" => TokenKind::Type,
                    "string" | "literal_string" | "single_quoted_string"
                    | "double_quoted_string" | "dollar_quoted_string" => TokenKind::String,
                    "comment" | "line_comment" | "block_comment" => TokenKind::Comment,
                    k if k.starts_with("keyword_") => TokenKind::Keyword,
                    _ => TokenKind::Plain,
                }
            } else {
                TokenKind::Plain
            }
        }
    }
}

// ─── Rust classifier ──────────────────────────────────────────────────────────

fn classify_rust(node: &Node) -> TokenKind {
    let kind = node.kind();
    if node.is_error() || node.is_missing() {
        return TokenKind::Error;
    }

    match kind {
        // Keywords
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn"
        | "else" | "enum" | "extern" | "fn" | "for" | "if" | "impl" | "in" | "let"
        | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self"
        | "Self" | "static" | "struct" | "super" | "trait" | "type" | "unsafe" | "use"
        | "where" | "while" | "yield" | "true" | "false" => TokenKind::Keyword,

        // Literals
        "string_literal" | "raw_string_literal" | "string_content" | "char_literal"
        | "escape_sequence" => TokenKind::String,

        "integer_literal" | "float_literal" => TokenKind::Number,

        "line_comment" | "block_comment" => TokenKind::Comment,

        // Attributes
        "attribute_item" | "inner_attribute_item" => TokenKind::Attribute,

        // Lifetimes
        "lifetime" => TokenKind::Lifetime,

        // Macros
        "macro_invocation" | "macro_definition" => TokenKind::Macro,
        "!" if node.parent().map(|p| p.kind()) == Some("macro_invocation") => TokenKind::Macro,

        // Types
        "type_identifier" | "primitive_type" | "generic_type" | "scoped_type_identifier" => {
            TokenKind::Type
        }

        // Identifiers — disambiguate by parent
        "identifier" => {
            if let Some(p) = node.parent() {
                match p.kind() {
                    "function_item" | "call_expression" => TokenKind::Function,
                    "macro_invocation" | "macro_definition" => TokenKind::Macro,
                    "type_identifier" | "struct_item" | "enum_item" | "trait_item"
                    | "type_item" | "impl_item" | "use_declaration" => TokenKind::Type,
                    "attribute_item" | "inner_attribute_item" => TokenKind::Attribute,
                    _ => TokenKind::Identifier,
                }
            } else {
                TokenKind::Identifier
            }
        }

        // Punctuation
        "(" | ")" | "[" | "]" | "{" | "}" | "," | ";" | "." | "::" | ":" | "->" | "=>"
        | ".." | "..=" => TokenKind::Punctuation,

        // Operators
        "=" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "+" | "-" | "*" | "/" | "%" | "&"
        | "|" | "^" | "!" | "~" | "<<" | ">>" | "&&" | "||" | "+=" | "-=" | "*=" | "/="
        | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "?" => TokenKind::Operator,

        "mutable_specifier" => TokenKind::Keyword,
        "field_identifier" => TokenKind::Identifier,

        "ERROR" => TokenKind::Error,

        _ => {
            // Fallback: check parent
            if let Some(p) = node.parent() {
                match p.kind() {
                    "string_literal" | "raw_string_literal" | "char_literal" => TokenKind::String,
                    "line_comment" | "block_comment" => TokenKind::Comment,
                    "attribute_item" | "inner_attribute_item" => TokenKind::Attribute,
                    "macro_invocation" | "macro_definition" => TokenKind::Macro,
                    _ => TokenKind::Plain,
                }
            } else {
                TokenKind::Plain
            }
        }
    }
}
