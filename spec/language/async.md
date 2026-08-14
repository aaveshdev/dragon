# Dragon Language Asynchronous Programming

## Status: Implementation-defined (based on current parser/implementation)

This document describes asynchronous programming features in the Dragon programming language.

## Async/Await Model

Dragon provides asynchronous programming capabilities through the `async` and `await` keywords, enabling non-blocking operations particularly useful for I/O-bound tasks.

## Async Functions

Functions can be declared as asynchronous using the `async` keyword:

```dragon
async func fetchData(url) {
    // This function returns a promise-like object
    let response = await http.get(url);
    return json.parse(response);
}
```

### Characteristics of Async Functions
1. **Return value**: An async function always returns a promise-like object, even if it doesn't explicitly use `await` or return a value.
2. **Await usage**: Only functions declared with `async` can use the `await` keyword inside their body.
3. **Execution**: When called, an async function starts execution immediately but may suspend at `await` points.

## Await Operator

The `await` operator is used to wait for the resolution of a promise-like value:

```dragon
async func example() {
    let result = await someAsyncOperation();
    // Execution resumes here when the operation completes
    return result;
}
```

### How Await Works
1. When `await expr` is encountered:
   - The async function is suspended
   - The expression `expr` is evaluated (should return a promise-like object)
   - Execution of the async function resumes when the promise settles
   - If the promise fulfills, `await` returns the fulfilled value
   - If the promise rejects, `await` throws the rejection reason as an exception

## Promise-like Objects

In Dragon's current implementation, async operations return objects that conform to a promise-like interface with:
- A `then` method for registering fulfillment handlers
- A `catch` method for registering rejection handlers
- Compatibility with the `await` operator

## Examples

```dragon
import "std/http";
import "std/json";

// Async function to fetch and parse JSON
async func fetchJson(url) {
    try {
        let response = await http.get(url);
        if response.status == 200 {
            return await json.parse(response.body);
        } else {
            raise "HTTP " + response.status + ": " + response.statusText;
        }
    } catch (err) {
        raise "Failed to fetch JSON: " + err;
    }
}

// Using the async function
async func main() {
    try {
        let data = await fetchJson("https://api.example.com/data");
        print("Received data: " + string(data));
    } catch (err) {
        print("Error: " + err);
    }
}

main();
```

## Sequential vs Parallel Execution

### Sequential (default)
```dragon
async func sequential() {
    let a = await operation1();  // Waits for completion
    let b = await operation2();  // Waits for completion after a finishes
    let c = await operation3();  // Waits for completion after b finishes
    return a + b + c;
}
```

### Parallel Execution
```dragon
async func parallel() {
    // Start all operations concurrently
    let op1 = operation1();  // Doesn't await yet
    let op2 = operation2();  // Doesn't await yet
    let op3 = operation3();  // Doesn't await yet
    
    // Wait for all to complete
    let a = await op1;
    let b = await op2;
    let c = await op3;
    
    return a + b + c;
}
```

## Error Handling

Exceptions in async functions can be handled with try-catch:

```dragon
async func riskyOperation() {
    try {
        let result = await dangerousAsyncCall();
        return result;
    } catch (err) {
        // Handle error from the async operation
        log("Operation failed: " + err);
        // Could re-throw or return a default value
        return null;
    }
}
```

## Implementation Notes

1. **Single-threaded concurrency**: Dragon's async/await is designed for single-threaded asynchronous I/O, not parallel computation.

2. **Event loop**: The implementation relies on an event loop to manage asynchronous operations and callbacks.

3. **No multi-threading**: Async/await does not create additional threads - it enables non-blocking I/O within a single thread.

4. **Promise interoperability**: Async functions work with standard promise-like objects that follow the Promise/A+ specification conceptually.

5. **Stack traces**: Async stack traces may not be as detailed as synchronous ones due to the nature of event-loop-based execution.

## Related Files
- `src/engine/parser.rs` - Async function parsing (lines 254-257, 742-746)
- `src/engine/` - Runtime async/await handling
- `spec/language/functions.md` - Function specification (includes async functions)
- `spec/language/concurrency.md` - Thread-based concurrency (different model)