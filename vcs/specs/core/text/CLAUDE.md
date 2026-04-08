# core/text Test Suite

Test coverage for Verum's text processing module.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `text_test.vr` | `text/text` | Basic Text operations, construction, comparison, search, slicing | ~30 |
| `char_test.vr` | `text/char` | ASCII classification, case conversion, digit conversion, Unicode categories, UTF encoding, escape sequences | 53 |
| `text_advanced_test.vr` | `text/text` | Iterators, SSO, UTF-8 validation, memory operations, error handling | ~55 |
| `text_production_test.vr` | `text/text` | Production text methods: strip, split_once, pad, trim_matches, eq_ignore_case, splitn, rsplitn | 115 |
| `format_test.vr` | `text/format` | FormatSpec, Formatter, Debug/Display, format strings | ~50 |
| `text_utils_test.vr` | `text/text` | from_char, split_at, parse_int_radix, From<Char>, FromIterator<Char> | 35 |
| `text_extra_test.vr` | `text/text` | char_count, trim_matches, split_ascii_whitespace, byte_index_of_char | 38 |
| `text_trim_methods_test.vr` | `text/text` | trim_start bugfix, insert_str, replace_range, AddAssign (+=) | 33 |
| `text_errors_test.vr` | `text/text` | Utf8Error Display, ParseError Display, Debug verification | 15 |

## Key Types Tested

### Text Type
- **Construction**: literals, empty, with_capacity, from_utf8
- **Properties**: len, is_empty, capacity, char_count
- **Access**: char_at, byte_at, nth_char, as_bytes
- **Comparison**: Eq, Ord, Hash
- **Search**: contains, starts_with, ends_with, find, rfind, find_char
- **Modification**: concat, push, push_str, trim, replace, split
- **Case conversion**: to_uppercase, to_lowercase
- **Parsing**: parse_int, parse_float, parse_bool

### Text Iterators
- `Chars` - Character iterator over Text
- `ByteIter` - Byte iterator over raw UTF-8 bytes
- `CharIndices` - Iterator yielding (byte_index, char) tuples
- `Lines` - Line iterator (handles \n and \r\n)

### Char Utilities
- **ASCII Classification**: is_ascii, is_ascii_alphabetic, is_ascii_uppercase, is_ascii_lowercase, is_ascii_digit, is_ascii_hexdigit, is_ascii_alphanumeric, is_ascii_whitespace, is_ascii_control, is_ascii_punctuation, is_ascii_graphic
- **Case Conversion**: to_ascii_uppercase, to_ascii_lowercase, eq_ignore_ascii_case
- **Digit Conversion**: is_digit, to_digit, from_digit
- **UTF-8/UTF-16**: len_utf8, len_utf16, encode_utf8

### Unicode Categories (GeneralCategory)
- **Letters**: Lu (uppercase), Ll (lowercase), Lt (titlecase), Lm (modifier), Lo (other)
- **Numbers**: Nd (decimal), Nl (letter), No (other)
- **Punctuation**: Pc (connector), Pd (dash), Ps (open), Pe (close), Po (other)
- **Symbols**: Sm (math), Sc (currency), Sk (modifier), So (other)
- **Separators**: Zs (space), Zl (line), Zp (paragraph)
- **Other**: Cc (control), Cf (format), Cs (surrogate), Co (private), Cn (unassigned)

### CharPattern Protocol
- `Char` - matches single character
- `fn(Char) -> Bool` - matches via predicate
- `AnyChar` - matches any in set
- `CharRange` - matches characters in range

### Formatting System
- **FormatSpec**: fill, align (Left/Center/Right), sign (Minus/Plus/Space), width, precision, alternate
- **Formatter**: write_str, write_char, write_int, write_float
- **Debug Helpers**: DebugStruct, DebugTuple, DebugList, DebugMap
- **Alignment**: Left, Center, Right
- **Sign**: Minus, Plus, Space

### Error Types
- `Utf8Error` - UTF-8 validation error with valid_up_to position
- `ParseError` - Parsing error with message
- `WriteError` - Write operation error (Io, BufferFull, Encoding)

## Tests by Category

### Basic Text Tests (~30 tests)
- Literal construction and empty text
- Length operations (len, is_empty)
- Equality and ordering comparison
- String concatenation
- Substring and slicing
- Search operations (contains, starts_with, ends_with, find, rfind)
- Case conversion (to_uppercase, to_lowercase)
- Trimming (trim, trim_start, trim_end)
- Splitting (split, split_whitespace)
- Character access (char_at, chars iterator)
- Parsing (parse_int, parse_float)
- Format string interpolation
- Repeat operation

### Char Classification Tests (~45 tests)
- All ASCII classification methods
- ASCII case conversion and comparison
- Digit conversion for bases 2, 10, 16, 36
- UTF-8/UTF-16 byte length
- GeneralCategory classification methods
- CharPattern implementations (Char, AnyChar, CharRange)
- Valid Unicode checks
- Edge cases (null, newline, space, boundary values)

### Advanced Text Tests (~55 tests)
- **ByteIter**: ASCII iteration, empty, multi-byte UTF-8
- **byte_at**: valid access, out of bounds
- **CharIndices**: ASCII iteration, empty, single char
- **Lines**: basic \n, CRLF \r\n, empty lines, trailing newline
- **SSO**: below threshold, at threshold (23 bytes), above threshold, transition via concat
- **nth_char**: valid access, out of bounds, empty
- **char_count**: ASCII, empty
- **Capacity**: with_capacity, capacity preservation
- **Push operations**: push char, push_str, repeated push
- **Parse errors**: error messages, empty input, whitespace handling
- **UTF-8 validation**: valid ASCII, empty, invalid continuation, truncated sequences
- **Join**: basic, empty separator, single element, empty parts
- **find_char**: found, not found, first occurrence
- **as_bytes**: basic, empty
- **Clone and Default**: cloning, default construction
- **Hash**: equal strings equal hash, different strings different hash

### Format System Tests (~50 tests)
- **Alignment variants**: Left, Center, Right
- **Sign variants**: Minus, Plus, Space
- **FormatSpec**: default values, width, precision, fill, alternate mode
- **Formatter**: new, write_str, write_char, write_int, write_float
- **format_display**: Int, Float, Bool, Char, Text
- **format_debug**: Int, Bool, Char (with escapes), Text (with escapes)
- **DebugStruct**: basic with fields, empty
- **DebugTuple**: basic with fields, empty
- **DebugList**: basic with entries, empty
- **DebugMap**: basic with entries, empty
- **Format strings**: basic interpolation, int, multiple, float, bool, nested
- **WriteError**: error kind variants
- **Edge cases**: empty string, special chars, sequential writes, large int, negative zero

## Known Limitations

- Unicode multi-byte characters beyond ASCII not extensively tested (requires Unicode literals)
- Surrogate code point handling not directly testable without unsafe construction
- UTF-16 surrogate pair encoding not tested
- Character composition/decomposition not tested
- Escape sequences for format strings limited to basic cases
- Large text (>1GB) stress testing not included
- SSO to heap transition performance not benchmarked

### New Test Files
| `text_mutations_test.vr` | `text/text` | truncate, insert, remove, matches, match_indices, replacen, ASCII case, retain, clear, pop | 50 |
| `text_classification_test.vr` | `text/text` | is_whitespace, is_blank, capitalize, to_title_case, swapcase, is_uppercase, is_lowercase, expand_tabs, zfill | 55 |
| `from_str_test.vr` | `text/text`, `base/protocols` | FromStr for Int/Float/Bool/Text, Extend<Char>/Extend<Text> for Text | 32 |

## Test Count: 673 tests total (14 test files)

## Architecture Notes

### Small String Optimization (SSO)
- Strings ≤ 23 bytes stored inline on stack
- No heap allocation for small strings
- Transparent transition to heap when exceeding threshold
- Tests verify behavior at 22, 23, and 24 byte boundaries

### UTF-8 Validation
- Validates continuation bytes (10xxxxxx)
- Detects overlong sequences
- Rejects surrogate code points (U+D800-U+DFFF)
- Reports error position via valid_up_to field

### Formatting Architecture
- FormatSpec carries all formatting options
- Formatter wraps output buffer with spec
- Debug vs Display distinction for quoting/escaping
- Debug helpers provide structured output for complex types
