//! Lexer token generator for fuzzing
//!
//! Generates valid and edge-case lexer tokens to stress test the Verum lexer.
//! Includes tests for:
//! - Unicode handling
//! - Number format edge cases
//! - String escape sequences
//! - Comment handling
//! - Whitespace variations

use super::{Generate, GeneratorConfig};
use rand::prelude::*;

/// Generator for lexer tokens and edge cases
pub struct LexerGenerator {
    config: GeneratorConfig,
}

impl LexerGenerator {
    /// Create a new lexer generator
    pub fn new(config: GeneratorConfig) -> Self {
        Self { config }
    }

    /// Generate a random valid token
    pub fn generate_token<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..15) {
            0 => self.generate_keyword(rng),
            1 => self.generate_identifier(rng),
            2 => self.generate_integer(rng),
            3 => self.generate_float(rng),
            4 => self.generate_string(rng),
            5 => self.generate_char(rng),
            6 => self.generate_operator(rng),
            7 => self.generate_delimiter(rng),
            8 => self.generate_comment(rng),
            9 => self.generate_whitespace(rng),
            10 => self.generate_unicode_identifier(rng),
            11 => self.generate_hex_literal(rng),
            12 => self.generate_binary_literal(rng),
            13 => self.generate_octal_literal(rng),
            _ => self.generate_scientific_notation(rng),
        }
    }

    /// Generate a keyword
    fn generate_keyword<R: Rng>(&self, rng: &mut R) -> String {
        let keywords = [
            "fn", "let", "if", "else", "match", "for", "while", "loop", "return", "break",
            "continue", "type", "use", "pub", "async", "await", "context", "provide", "using",
            "where", "impl", "trait", "self", "super", "in", "as", "mut", "ref", "static", "const",
            "true", "false", "None", "Some", "Ok", "Err",
        ];
        keywords[rng.random_range(0..keywords.len())].to_string()
    }

    /// Generate a valid identifier
    fn generate_identifier<R: Rng>(&self, rng: &mut R) -> String {
        let len = rng.random_range(1..=32);
        let mut result = String::with_capacity(len);

        // First character: letter or underscore
        let first = if rng.random_bool(0.9) {
            if rng.random_bool(0.5) {
                (b'a' + rng.random_range(0..26)) as char
            } else {
                (b'A' + rng.random_range(0..26)) as char
            }
        } else {
            '_'
        };
        result.push(first);

        // Rest: letters, digits, underscores
        for _ in 1..len {
            let c = match rng.random_range(0..4) {
                0 => (b'a' + rng.random_range(0..26)) as char,
                1 => (b'A' + rng.random_range(0..26)) as char,
                2 => (b'0' + rng.random_range(0..10)) as char,
                _ => '_',
            };
            result.push(c);
        }

        result
    }

    /// Generate a Unicode identifier (for stress testing)
    fn generate_unicode_identifier<R: Rng>(&self, rng: &mut R) -> String {
        let prefixes = [
            "\u{03B1}",  // alpha
            "\u{03B2}",  // beta
            "\u{03B3}",  // gamma
            "\u{03C0}",  // pi
            "\u{0394}",  // Delta
            "\u{03A9}",  // Omega
            "_\u{0410}", // _А (Cyrillic A)
        ];
        let prefix = prefixes[rng.random_range(0..prefixes.len())];
        format!("{}{}", prefix, rng.random_range(0..100))
    }

    /// Generate an integer literal
    fn generate_integer<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..10) {
            0 => "0".to_string(),
            1 => "1".to_string(),
            2 => "-1".to_string(),
            3 => format!("{}", rng.random_range(-1000..1000)),
            4 => format!("{}", i32::MAX),
            5 => format!("{}", i32::MIN),
            6 => format!("{}", i64::MAX),
            7 => format!("{}", i64::MIN),
            8 => format!("{}", rng.random::<i64>()),
            _ => format!(
                "{}_{}",
                rng.random_range(1..1000),
                rng.random_range(0..1000)
            ),
        }
    }

    /// Generate a hexadecimal literal
    fn generate_hex_literal<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..5) {
            0 => "0x0".to_string(),
            1 => "0xFF".to_string(),
            2 => "0xDEAD_BEEF".to_string(),
            3 => format!("0x{:X}", rng.random::<u32>()),
            _ => format!("0x{:x}", rng.random_range(0..65536)),
        }
    }

    /// Generate a binary literal
    fn generate_binary_literal<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..5) {
            0 => "0b0".to_string(),
            1 => "0b1".to_string(),
            2 => "0b1010_1010".to_string(),
            3 => format!("0b{:b}", rng.random_range(0..256)),
            _ => "0b1111_1111".to_string(),
        }
    }

    /// Generate an octal literal
    fn generate_octal_literal<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..3) {
            0 => "0o0".to_string(),
            1 => "0o777".to_string(),
            _ => format!("0o{:o}", rng.random_range(0..512)),
        }
    }

    /// Generate a float literal
    fn generate_float<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..12) {
            0 => "0.0".to_string(),
            1 => "-0.0".to_string(),
            2 => "1.0".to_string(),
            3 => "-1.0".to_string(),
            4 => format!("{:.6}", rng.random::<f64>()),
            5 => format!("{:.10}", rng.random::<f64>() * 1000.0),
            6 => "1.7976931348623157e308".to_string(), // f64::MAX
            7 => "-1.7976931348623157e308".to_string(),
            8 => "2.2250738585072014e-308".to_string(), // f64::MIN_POSITIVE
            9 => format!(
                "{}_{}e{}",
                rng.random_range(1..10),
                rng.random_range(0..100),
                rng.random_range(-10..10)
            ),
            10 => "inf".to_string(),
            _ => "nan".to_string(),
        }
    }

    /// Generate scientific notation
    fn generate_scientific_notation<R: Rng>(&self, rng: &mut R) -> String {
        let mantissa = rng.random_range(1..10);
        let decimal = rng.random_range(0..1000000);
        let exp = rng.random_range(-308..308);
        let sign = if rng.random_bool(0.5) { "e" } else { "E" };
        let exp_sign = if exp >= 0 { "+" } else { "" };
        format!("{}.{}{}{}{}", mantissa, decimal, sign, exp_sign, exp)
    }

    /// Generate a string literal
    fn generate_string<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..15) {
            0 => "\"\"".to_string(),
            1 => "\"hello world\"".to_string(),
            2 => "\"\\n\"".to_string(),
            3 => "\"\\t\"".to_string(),
            4 => "\"\\r\"".to_string(),
            5 => "\"\\\\\"".to_string(),
            6 => "\"\\\"\"".to_string(),
            7 => "\"\\0\"".to_string(),
            8 => "\"\\u{1F600}\"".to_string(), // Emoji
            9 => "\"\\u{0}\"".to_string(),
            10 => "\"\\u{10FFFF}\"".to_string(), // Max codepoint
            11 => {
                // Long string
                let len = rng.random_range(100..1000);
                let s: String = (0..len).map(|_| 'a').collect();
                format!("\"{}\"", s)
            }
            12 => {
                // String with escapes
                "\"line1\\nline2\\tindented\"".to_string()
            }
            13 => {
                // Unicode string
                "\"\u{0410}\u{0411}\u{0412}\u{0413}\"".to_string()
            }
            _ => {
                // Random string
                let len = rng.random_range(1..50);
                let s: String = (0..len)
                    .map(|_| {
                        let c = rng.random_range(32..127);
                        if c == 34 || c == 92 {
                            'x'
                        } else {
                            c as u8 as char
                        }
                    })
                    .collect();
                format!("\"{}\"", s)
            }
        }
    }

    /// Generate a character literal
    fn generate_char<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..10) {
            0 => "'a'".to_string(),
            1 => "'\\n'".to_string(),
            2 => "'\\t'".to_string(),
            3 => "'\\r'".to_string(),
            4 => "'\\\\'".to_string(),
            5 => "'\\''".to_string(),
            6 => "'\\0'".to_string(),
            7 => "'\\u{1F600}'".to_string(),
            8 => {
                let c = (b'a' + rng.random_range(0..26)) as char;
                format!("'{}'", c)
            }
            _ => "'\\u{0}'".to_string(),
        }
    }

    /// Generate an operator
    fn generate_operator<R: Rng>(&self, rng: &mut R) -> String {
        let operators = [
            "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||", "!", "&", "|",
            "^", "~", "<<", ">>", "=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=",
            ">>=", "->", "=>", "::", ".", "..", "..=", "...", "?", "@", "#",
        ];
        operators[rng.random_range(0..operators.len())].to_string()
    }

    /// Generate a delimiter
    fn generate_delimiter<R: Rng>(&self, rng: &mut R) -> String {
        let delimiters = ["(", ")", "[", "]", "{", "}", ",", ";", ":", "|"];
        delimiters[rng.random_range(0..delimiters.len())].to_string()
    }

    /// Generate a comment
    fn generate_comment<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..5) {
            0 => "// line comment".to_string(),
            1 => "/* block comment */".to_string(),
            2 => "/// doc comment".to_string(),
            3 => "//! inner doc comment".to_string(),
            _ => {
                let len = rng.random_range(1..100);
                let content: String = (0..len)
                    .map(|_| {
                        let c = rng.random_range(32..127);
                        c as u8 as char
                    })
                    .collect();
                format!("// {}", content)
            }
        }
    }

    /// Generate whitespace
    fn generate_whitespace<R: Rng>(&self, rng: &mut R) -> String {
        match rng.random_range(0..5) {
            0 => " ".to_string(),
            1 => "\t".to_string(),
            2 => "\n".to_string(),
            3 => "\r\n".to_string(),
            _ => {
                let count = rng.random_range(1..10);
                " ".repeat(count)
            }
        }
    }

    /// Generate a sequence of tokens forming a valid statement
    fn generate_token_sequence<R: Rng>(&self, rng: &mut R) -> String {
        let mut tokens = Vec::new();
        let count = rng.random_range(3..20);

        for _ in 0..count {
            tokens.push(self.generate_token(rng));
        }

        tokens.join(" ")
    }
}

impl Generate for LexerGenerator {
    fn generate<R: Rng>(&mut self, rng: &mut R) -> String {
        let mut output = String::new();

        // Generate a mix of tokens forming statements
        let max_stmts = self.config.max_statements.max(6);
        let statement_count = rng.random_range(5..max_stmts);

        for _ in 0..statement_count {
            match rng.random_range(0..10) {
                0..=2 => {
                    // Let binding with various literal types
                    output.push_str("let ");
                    output.push_str(&self.generate_identifier(rng));
                    output.push_str(" = ");
                    match rng.random_range(0..5) {
                        0 => output.push_str(&self.generate_integer(rng)),
                        1 => output.push_str(&self.generate_float(rng)),
                        2 => output.push_str(&self.generate_string(rng)),
                        3 => output.push_str(&self.generate_hex_literal(rng)),
                        _ => output.push_str(&self.generate_binary_literal(rng)),
                    }
                    output.push_str(";\n");
                }
                3 => {
                    // Comment
                    output.push_str(&self.generate_comment(rng));
                    output.push('\n');
                }
                4 => {
                    // Unicode identifier
                    output.push_str("let ");
                    output.push_str(&self.generate_unicode_identifier(rng));
                    output.push_str(" = ");
                    output.push_str(&self.generate_integer(rng));
                    output.push_str(";\n");
                }
                5..=7 => {
                    // Expression with operators
                    output.push_str("let ");
                    output.push_str(&self.generate_identifier(rng));
                    output.push_str(" = ");
                    output.push_str(&self.generate_integer(rng));
                    output.push(' ');
                    output.push_str(&self.generate_operator(rng));
                    output.push(' ');
                    output.push_str(&self.generate_integer(rng));
                    output.push_str(";\n");
                }
                _ => {
                    // Random token sequence
                    output.push_str(&self.generate_token_sequence(rng));
                    output.push('\n');
                }
            }
        }

        // Wrap in a main function
        format!("fn main() {{\n{}}}\n", output)
    }

    fn name(&self) -> &'static str {
        "LexerGenerator"
    }

    fn description(&self) -> &'static str {
        "Generates programs with lexer edge cases"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_lexer_generator() {
        let config = GeneratorConfig::default();
        let mut generator = LexerGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let program = generator.generate(&mut rng);
        assert!(!program.is_empty());
        assert!(program.contains("fn main()"));
    }

    #[test]
    fn test_token_types() {
        let config = GeneratorConfig::default();
        let generator = LexerGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Test each token type
        for _ in 0..100 {
            let token = generator.generate_token(&mut rng);
            assert!(!token.is_empty());
        }
    }

    #[test]
    fn test_integer_edge_cases() {
        let config = GeneratorConfig::default();
        let generator = LexerGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Generate many integers to cover edge cases
        for _ in 0..100 {
            let int = generator.generate_integer(&mut rng);
            assert!(!int.is_empty());
        }
    }

    #[test]
    fn test_string_escapes() {
        let config = GeneratorConfig::default();
        let generator = LexerGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Generate many strings to cover escape sequences
        for _ in 0..100 {
            let s = generator.generate_string(&mut rng);
            assert!(s.starts_with('"'));
            assert!(s.ends_with('"'));
        }
    }
}
