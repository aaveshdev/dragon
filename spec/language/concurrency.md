# Dragon Language Concurrency

## Status: Implementation-defined (based on current parser/implementation)

This document describes concurrency features in the Dragon programming language.

## Concurrency Model

Dragon provides basic concurrency primitives for parallel execution. The current implementation supports:

1. **Threads**: Basic thread creation and management
2. **Mutexes**: Mutual exclusion for shared resource protection
3. **Condition variables**: Coordination between threads

## Thread Creation

Threads can be created using the `thread` function from the standard library:

```dragon
import "std/thread";

// Create a new thread
let thread_handle = std.thread.create(func() {
    // Code to run in the new thread
    print("Running in thread");
});

// Wait for thread to complete
std.thread.join(thread_handle);
```

## Thread Functions

The standard thread module provides:

- `std.thread.create(func)` - Creates a new thread running the given function
- `std.thread.join(handle)` - Waits for a thread to finish execution
- `std.thread.sleep(ms)` - Sleeps the current thread for specified milliseconds
- `std.thread.yield()` - Yields the current thread's remaining time slice

## Synchronization Primitives

### Mutexes
```dragon
import "std/sync";

// Create a mutex
let mutex = std.sync.Mutex.new();

// Lock the mutex
mutex.lock();
// Access shared resource
// ...
// Unlock the mutex
mutex.unlock();
```

### Condition Variables
```dragon
import "std/sync";

// Create condition variable
let cond = std.sync.CondVar.new();
// Associate with a mutex
let mutex = std.sync.Mutex.new();

// Wait on condition variable
mutex.lock();
while !condition_met {
    cond.wait(mutex);
}
// ...
mutex.unlock();

// Signal condition variable
cond.signal();      // Wake one waiting thread
cond.broadcast();   // Wake all waiting threads
```

## Examples

```dragon
import "std/thread";
import "std/sync";

// Shared counter
let counter = 0;
let mutex = std.sync.Mutex.new();

// Worker function
func worker(id) {
    for i in 1:1000 {
        mutex.lock();
        counter = counter + 1;
        mutex.unlock();
    }
    print("Worker " + string(id) + " finished");
}

// Create multiple threads
let threads = [];
for i in 1:5 {
    threads[i] = std.thread.create(func() { worker(i); });
}

// Wait for all threads to complete
for thread in threads {
    std.thread.join(thread);
}

print("Final counter value: " + string(counter));
```

## Implementation Notes

1. **Preemptive multitasking**: Dragon uses preemptive multitasking for threads - the OS scheduler determines thread execution order.

2. **Shared memory**: Threads share the same memory space, allowing efficient communication but requiring synchronization.

3. **No built-in parallelism constructs**: Dragon does not currently have parallel loops, parallel sections, or other high-level concurrency constructs.

4. **Thread safety**: The language does not enforce thread safety - programmers must use synchronization primitives correctly.

5. **Operating system threads**: Threads are implemented using native OS threads (via pthreads on Unix-like systems, Windows threads on Windows).

## Limitations

1. **No async/await for threads**: The current `async`/`await` keywords are not related to thread-based concurrency.

2. **No thread-local storage**: Standard library does not provide thread-local storage mechanisms.

3. **No race condition detection**: The language/runtime does not detect race conditions.

## Related Files
- `src/engine/` - Runtime threading support (if implemented)
- `examples/` - Example programs using threads
- `spec/language/async.md` - Asynchronous programming specification (different from thread concurrency)
- `spec/language/std-lib/thread.md` - Thread standard library specification (not separately created)