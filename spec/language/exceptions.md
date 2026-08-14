# Dragon Language Exceptions

## Status: Implementation-defined (based on current parser/implementation)

This document describes exception handling in the Dragon programming language.

## Exception Model

Dragon uses a traditional try-catch-finally model for exception handling. Exceptions are objects that represent error conditions or unusual situations that disrupt the normal flow of program execution.

## Throwing Exceptions

Exceptions are thrown using the `raise` statement:

```dragon
raise expression;
```

The expression should evaluate to a value that represents the error. In the current implementation, this is typically a string, but can be any value.

### Examples
```dragon
// Throw a string exception
raise "Invalid input";

// Throw with variable
let errorMsg = "File not found: " + filename;
raise errorMsg;

// Throw other types (though less common)
raise 42;
raise null;
raise True;
```

## Catching Exceptions

Exceptions are caught using try-catch blocks:

```dragon
try {
    // code that might throw an exception
} catch (variable) {
    // handle the exception
} finally {
    // cleanup code that always runs
}
```

### Try Block
- Contains code that might throw an exception
- If no exception occurs, the catch block is skipped
- If an exception occurs, execution jumps to the catch block

### Catch Block
- Declares a variable that receives the exception value
- Executes only if an exception was thrown in the try block
- After the catch block executes, the finally block runs (if present)

### Finally Block
- Optional block that executes after the try block
- Runs whether an exception occurred or not
- Runs after the catch block if there was an exception
- Used for cleanup resources that must always be released

## Exception Propagation

If an exception is not caught in the current function, it propagates up the call stack:
1. The current function stops executing
2. The exception is thrown to the caller
3. This continues until either:
   - A catch block handles the exception
   - The exception reaches the top level and terminates the program

## Examples

```dragon
// Basic exception handling
try {
    let data = json.parse(input);
    process(data);
} catch (err) {
    print("Error: " + err);
} finally {
    cleanup_resources();
}

// Multiple operations that might fail
try {
    let file = file.open("data.txt");
    let content = file.readAll();
    file.close();
    let data = json.parse(content);
    return data;
} catch (err) {
    print("Failed to process file: " + err);
    return null;
} finally {
    // This always runs
    print("File processing attempt finished");
}

// Re-throwing exceptions
try {
    // Some operation
    risky_operation();
} catch (err) {
    // Log the error
    log("Operation failed: " + err);
    // Re-throw to let caller handle it
    raise err;
}

// Nested try-catch
try {
    try {
        // Inner operation
        inner_operation();
    } catch (inner_err) {
        // Handle inner error
        handle_inner_error(inner_err);
        // Re-throw if needed
        raise inner_err;
    }
} catch (outer_err) {
    // Handle outer error
    handle_outer_error(outer_err);
}
```

## Implementation Notes

1. **Exception values**: In the current implementation, exceptions can be any value (string, number, object, etc.), but strings are most common for error messages.

2. **Stack traces**: The current implementation does not automatically provide stack traces with exceptions.

3. **Exception types**: Dragon does not currently have distinct exception types or exception hierarchies - all exceptions are treated uniformly.

4. **Uncaught exceptions**: If an exception reaches the top level without being caught, the program terminates and may print the exception value.

5. **Finally execution**: The finally block always executes, even if:
   - There is no catch block
   - An exception occurs in the catch block
   - A return statement is executed in the try or catch block

## Related Files
- `src/engine/parser.rs` - Try-catch parsing (lines 583-600)
- `src/engine/` - Runtime exception handling
- `spec/language/statements.md` - Try-catch statement specification