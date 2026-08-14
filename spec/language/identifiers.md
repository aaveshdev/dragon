# Dragon Language Identifiers

## Status: Implementation-defined (based on current lexer/parser implementation)

This document describes the rules for identifiers in the Dragon programming language.

## Identifier Rules

Identifiers in Dragon follow these rules:

1. **First character**: Must be a letter (a-z, A-Z) or underscore (_)
2. **Subsequent characters**: Can be letters (a-z, A-Z), digits (0-9), or underscores (_)
3. **Case sensitivity**: Identifiers are case-sensitive (`var` and `Var` are different)
4. **Length**: No specific limit imposed by the language

## Regular Expression
```
[a-zA-Z_][a-zA-Z0-9_]*
```

## Examples

Valid identifiers:
```
myVariable
_privateVar
snake_case
CamelCase
under_score_123
_
```

Invalid identifiers:
```
123abc          (starts with digit)
my-var          (contains hyphen)
my var          (contains space)
@variable       (contains special symbol)
```

## Reserved Keywords

The following keywords are reserved and cannot be used as identifiers:

```
func, class, if, elif, else, while, return, True, False, null,
and, or, not, for, in, switch, case, default, break, continue,
import, as, try, catch, finally, raise, do, new, extends, this,
super, const, var, async, await
```

## Special Identifiers

Certain identifiers have special meaning in specific contexts:

- `this`: Refers to the current instance in class methods
- `super`: Refers to the parent class in class methods
- `_`: Conventionally used for unused variables (though not special to the language)

## Naming Conventions

While not enforced by the language, the following conventions are commonly used:

- **Variables and functions**: `snake_case` (e.g., `my_variable`, `calculate_total`)
- **Classes**: `PascalCase` (e.g., `MyClass`, `HttpClient`)
- **Constants**: `UPPER_SNAKE_CASE` (e.g., `MAX_VALUE`, `PI`)
- **Private members**: Prefixed with underscore (e.g., `_internal_state`)

## Implementation Notes

The lexer identifies identifiers after matching the pattern `[a-zA-Z_][a-zA-Z0-9_]*` and then checks if the matched text is a keyword. If it is, it produces a `Keyword` token; otherwise, it produces a `Name` token.

## Related Files
- `src/engine/lexer.rs` - Identifier recognition in the lexer (lines 215-227)
- `spec/language/lexical-structure.md` - Lexical structure specification