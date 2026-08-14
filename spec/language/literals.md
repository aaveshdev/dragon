# Dragon Language Literals

## Status: Implementation-defined (based on current lexer/parser implementation)

This document describes the literal values supported in the Dragon programming language.

## String Literals

Dragon supports several forms of string literals:

### Regular Strings
Enclosed in double quotes (`"..."`) or single quotes (`'...'`):
```dragon
"Hello, world!"
'Hello, world!'
"Line 1\nLine 2"
'It\'s a beautiful day'
```

#### Escape Sequences
Regular strings support the following escape sequences:
- `\n` - newline
- `\t` - tab
- `\r` - carriage return
- `\0` - null character
- `\"` - double quote
- `\'` - single quote
- `\\` - backslash
- `\f` - form feed
- `\uXXXX` - Unicode character (where XXXX is 4 hexadecimal digits)

### Raw Strings
Prefixed with `r` and enclosed in quotes:
```dragon
r"C:\Users\Name"
r'\d+\.\d+'
r"Hello\nWorld"  // Literally: Hello\nWorld (no escape processing)
```

**Important**: Raw strings do NOT process ANY escape sequences, including `\n` or `\t`.

### Interpolated Strings (Backtick Strings)
Enclosed in backticks (``` `...` ```) with `${expression}` interpolation:
```dragon
let name = "World";
`Hello ${name}!`                    // "Hello World!"
`The sum is ${a + b}`               // "The sum is 5" (if a=2, b=3)
`Nested: ${outer ${inner}}`         // Nested interpolation
`Price: ${cost * 1.08} tax included` // With expressions
```

#### Interpolation Rules
1. The content is parsed as alternating literal and expression segments
2. Expressions are wrapped in `str()` calls and concatenated with `+` operator
3. Backtick strings support the same escape sequences as regular strings
4. Unicode escapes (`\uXXXX`) work inside interpolated strings

### Unicode Escapes
Both regular and interpolated strings support `\uXXXX` Unicode escape sequences:
```dragon
"Hello\u0020World"    // "Hello World" (space)
"\u00A9 2023"         // "© 2023" (copyright symbol)
"\u2603\u2600\u2603"  // "�☃�☀�☃" (snowman, sun, snowman)
```
The XXXX must be exactly 4 hexadecimal digits (0-9, A-F, a-f).

## Numeric Literals

Dragon supports integer and floating-point literals:

### Decimal Integers
```dragon
0
123
456789
```

### Decimal Floating-Point
```dragon
0.0
3.14159
0.5
123.456
```

### Scientific Notation
```dragon
1e10        // 1 × 10^10
1.23e-4     // 1.23 × 10^-4
6.02E23     // 6.02 × 10^23 (case-insensitive E)
```

### Hexadecimal Integers
Prefixed with `0x`:
```dragon
0x0
0xFF
0x1A2B
0xdeadbeef
```

**Note**: Hexadecimal floating-point literals are not supported.

## Boolean Literals
```dragon
True   // boolean true
False  // boolean false
```

## Null Literal
```dragon
null   // represents the absence of a value
```

## Examples

```dragon
// String examples
let greeting = "Hello, world!";
let filepath = r"C:\Program Files\App";
let message = `Hello ${user}! You have ${count} new messages.`;

 // Number examples
let integer = 42;
let pi = 3.14159;
let hex = 0xFF;
let scientific = 1.23e-4;

// Boolean examples
let isTrue = True;
let isFalse = False;

// Null example
let empty = null;
```

## Related Files
- `src/engine/lexer.rs` - Literal recognition in the lexer
- `spec/language/lexical-structure.md` - Lexical structure specification