// cj-lexer: Cangjie lexer (source -> Token stream)
pub mod lexer;
pub mod token;

#[cfg(test)]
mod token_tests;

pub use lexer::{lex_all, LexError, Lexer, Position, Token};
pub use token::{lookup_keyword, TokenKind};
