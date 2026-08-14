# Dragon Language Lexical Structure

## Status: Implementation-defined (based on current lexer implementation)

This document describes the lexical structure of the Dragon programming language based on the current implementation in `src/engine/lexer.rs`.

## Tokens

Dragon produces the following tokens during lexical analysis:

### Keywords
The following reserved keywords are recognized:
```
func, class, if, elif, else, while, return, True, False, null,
and, or, not, for, in, switch, case, default, break, continue,
import, as, try, catch, finally, raise, do, new, extends, this,
super, const, var, async, await
```

### Literals
- **String literals**: Enclosed in double quotes (`"..."`) or single quotes (`'...'`) with standard escape sequences
- **Raw string literals**: Prefixed with `r` (`r"..."` or `r'...'`) - no escape processing
- **Backtick strings (interpolated)**: Enclosed in backticks (``` `...` ```) with `${expression}` interpolation
- **Number literals**: 
  - Decimal: `123`, `123.456`, `1.23e4`, `1.23E-4`
  - Hexadecimal: `0xFF`, `0x1a2b`

### Operators
- **Arithmetic**: `+`, `-`, `*`, `/`, `%`
- **Increment/Decrement**: `++`, `--`
- **Assignment**: `=`, `+=`, `-=`, `*=`, `/=`, `%=`
- **Comparison**: `==`, `!=`, `<`, `>`, `<=`, `>=`
- **Logical**: `&&`, `||`, `!` (represented as `and`, `or`, `not` keywords)
- **Null-coalescing**: `??`
- **Optional chaining**: `?.`
- **Bitwise**: `&`, `|`, `^`, `~`, `<<`, `>>`
- **Ternary**: `?:`
- **Range**: `...`
- **Statement separator**: `;`
- **Member access**: `.`
- **Function call**: `()`
- **Indexing**: `[]`
- **Block delimiters**: `{}`

### Separators
- **Comma**: `,`
- **Colon**: `:`
- **Semicolon**: `;` (statement terminator)
- **Parentheses**: `(` `)`
- **Brackets**: `[` `]`
- **Braces**: `{` `}`

### Identifiers
Identifiers follow the pattern: `[a-zA-Z_][a-zA-Z0-9_]*`

### Whitespace
Whitespace characters (space, tab, carriage return) are ignored except to separate tokens.

### Comments
- **Line comments**: `//` to end of line
- **Block comments**: `/* ... */` (can span multiple lines)

### Newline
Newline characters (`\n`) are significant and increase the line counter.

## Token Types

The lexer produces tokens with the following types:
- `Keyword` - Reserved keywords
- `Name` - Identifiers
- `String` - String literals (including raw and interpolated)
- `Number` - Numeric literals
- `Op` - Operators and punctuation
- `Inc` - Increment operator (`++`)
- `Dec` - Decrement operator (`--`)
- `Eof` - End of file

## Implementation Notes

1. **Raw strings**: Raw string literals (prefixed with `r`) do NOT process any escape sequences, including `\n` or `\t`.

2. **Backtick strings**: Backtick-delimited strings support interpolation using `${expression}` syntax. The content is processed as alternating literal and expression segments, with expressions wrapped in `str()` calls and concatenated.

3. **Unicode escapes**: Both regular strings and backtick strings support `\uXXXX` Unicode escape sequences where XXXX is a 4-digit hexadecimal number.

4. **Number parsing**: Numbers can be decimal or hexadecimal (prefixed with `0x`). Scientific notation is supported for decimal numbers.

5. **Error handling**: The lexer returns descriptive error messages with line and column information for unterminated literals and unexpected characters.

## Examples

```dragon
// String literals
"Hello, world!"
'Hello, world!'
"Line 1\nLine 2"
"She said \"Hello!\""

// Raw strings (no escape processing)
r"C:\Users\Name"
r'\d+\.\d+'

// Interpolated strings
let name = "World";
`Hello ${name}!`  // Results in "Hello World!"
`Price: ${cost * 1.08} tax included`

// Numbers
42
3.14159
0xFF
1.23e-4
```

## Related Files
- `src/engine/lexer.rs` - Implementation of the lexer
- `spec/language/syntax.md` - Syntactic grammar specification