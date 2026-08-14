# Dragon Language Expressions

## Status: Implementation-defined (based on current parser implementation)

This document describes the expression syntax and evaluation in the Dragon programming language.

## Expression Categories

Dragon expressions are categorized into the following types:

### Primary Expressions
- **Literals**: numbers, strings, booleans, null
- **Variables**: identifiers referencing variables or constants
- **This**: `this` keyword referring to current instance
- **Parenthesized expressions**: `(expr)`
- **Object literals**: `{key: value, ...}`
- **Array literals**: `[item1, item2, ...]`
- **Function expressions**: `func(params) { body }`

### Postfix Expressions
- **Member access**: `expression.identifier`
- **Optional chaining**: `expression?.identifier`
- **Index access**: `expression[expr]`
- **Function call**: `expression(args...)`
- **Increment/Decrement**: `expression++`, `expression--`
- **Await**: `await expression` (only inside async functions)

### Unary Expressions
- **Plus**: `+expression`
- **Minus**: `-expression`
- **Logical NOT**: `!expression`, `not expression`
- **New**: `new Constructor(args...)`

### Multiplicative Expressions
- **Multiplication**: `expr * expr`
- **Division**: `expr / expr`
- **Modulo**: `expr % expr`

### Additive Expressions
- **Addition**: `expr + expr`
- **Subtraction**: `expr - expr`
- **Bitwise shift left**: `expr << expr`
- **Bitwise shift right**: `expr >> expr`
- **Bitwise AND**: `expr & expr`
- **Bitwise OR**: `expr | expr`
- **Bitwise XOR**: `expr ^ expr`

### Comparative Expressions
- **Equality**: `expr == expr`
- **Inequality**: `expr != expr`
- **Less than**: `expr < expr`
- **Greater than**: `expr > expr`
- **Less than or equal**: `expr <= expr`
- **Greater than or equal**: `expr >= expr`
- **In operator**: `expr in expr` (for checking membership in arrays/strings)

### Logical Expressions
- **Logical AND**: `expr and expr`
- **Logical OR**: `expr or expr`

### Null-Coalescing Expressions
- **Null-coalescing**: `expr ?? expr` (returns left if not null, otherwise right)

### Conditional Expressions
- **Ternary**: `condition ? expr_if_true : expr_if_false`

### Assignment Expressions
- **Simple assignment**: `lvalue = expr`
- **Compound assignment**: `lvalue += expr`, `-=` , `*=` , `/=` , `%=`

## Expression Evaluation

Expressions are evaluated according to operator precedence and associativity:

1. **Primary** (highest precedence)
2. **Postfix** (left to right)
3. **Unary** (right to left)
4. **Multiplicative** (left to right)
5. **Additive** (left to right)
6. **Comparative** (left to right)
7. **Logical AND** (left to right)
8. **Logical OR** (left to right)
9. **Null-coalescing** (left to right)
10. **Conditional** (right to left)
11. **Assignment** (right to left)

## Implementation Notes

1. **Short-circuit evaluation**: 
   - Logical AND (`and`) and OR (`or`) evaluate left to right and stop when the result is determined.
   - Null-coalescing (`??`) returns the left operand if it is not null, otherwise evaluates and returns the right operand.
   - Ternary operator (`?:`) evaluates only the chosen branch.

2. **Lvalue requirements**: 
   - Assignment operators require an lvalue on the left-hand side (variable, property access, or index access).

3. **Await expressions**: 
   - Only valid inside functions declared with `async`.
   - Used to wait for the resolution of a promise-like value.

4. **New expressions**: 
   - Create a new instance of a class by calling its constructor.

5. **Function expressions**: 
   - Can be used inline and have access to the surrounding scope (closure).

6. **Object and array literals**: 
   - Created at runtime with the specified properties/elements.

## Examples

```dragon
// Primary expressions
42                    // number literal
"hello"               // string literal
True                  // boolean literal
null                  // null literal
x                     // variable reference
this                  // current instance
(point.x)             // parenthesized expression
{name: "Alice", age: 30}  // object literal
[1, 2, 3]             // array literal
func(x) { return x * 2; } // function expression

// Postfix expressions
obj.property          // member access
obj?.[prop]           // optional chaining
arr[0]                // index access
func()                // function call
counter++             // postfix increment
await fetchData()     // await (in async func)

// Unary expressions
+5                    // unary plus
-3                    // unary minus
!flag                 // logical not
not flag              // alternative logical not
new Date()            // new expression

// Multiplicative expressions
6 * 7                 // multiplication
10 / 2                // division
15 % 4                // modulo

// Additive expressions
5 + 3                 // addition
10 - 4                // subtraction
1 << 3                // left shift (8)
8 >> 1                // right shift (4)
5 & 3                 // bitwise AND (1)
5 | 2                 // bitwise OR (7)
5 ^ 3                 // bitwise XOR (6)

// Comparative expressions
5 == 5                // equality (True)
5 != 3                // inequality (True)
2 < 3                 // less than (True)
3 > 1                 // greater than (True)
2 <= 2                // less than or equal (True)
3 >= 3                // greater than or equal (True)
2 in [1,2,3]          // in operator (True)

// Logical expressions
True and False        // logical AND (False)
True or False         // logical OR (True)

// Null-coalescing expressions
null ?? "default"     // "default"
"value" ?? "default"  // "value"

// Conditional expressions
x > 0 ? "positive" : "non-positive"

// Assignment expressions
x = 10                // simple assignment
x += 5                // compound addition (x = x + 5)
```

## Related Files
- `src/engine/parser.rs` - Expression parsing implementation
- `spec/language/lexical-structure.md` - Lexical structure specification
- `spec/language/syntax.md` - Syntactic grammar specification