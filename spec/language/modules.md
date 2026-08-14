# Dragon Language Modules

## Status: Implementation-defined (based on current parser/implementation and project structure)

This document describes the module system in the Dragon programming language.

## Module System Overview

Dragon uses a file-based module system where each Dragon source file (.dg) is a module. Modules can import and export functionality to enable code organization and reuse.

## File Organization

### Module Files
- Each `.dg` file represents a module
- The module name is derived from the filename (without extension)
- Modules are organized in directories that reflect their namespace

### Example Structure
```
project/
├── main.dg
├── utils/
│   ├── string.dg
│   └── math.dg
�└── services/
    └── http.dg
```

In this structure:
- `main.dg` is the main module
- `utils.string` is the module from `utils/string.dg`
- `utils.math` is the module from `utils/math.dg`
- `services.http` is the module from `services/http.dg`

## Importing Modules

Modules can import other modules using the `import` statement:

### Basic Import
```dragon
import "utils/string";
```
- Imports the module and makes its exports available under the module name
- To use an exported function: `utils.string.functionName()`

### Import with Alias
```dragon
import "utils/string" as str;
```
- Imports the module but makes it available under the alias `str`
- To use an exported function: `str.functionName()`

### Relative Imports
```dragon
import "./utils/string";        // Relative to current file
import "../utils/string";       // Parent directory
```

## Exporting from Modules

By default, all top-level declarations in a module are exported:

### Automatic Exports
In `utils/string.dg`:
```dragon
func reverse(text) {
    // implementation
}

func capitalize(text) {
    // implementation
}

const VERSION = "1.0.0";
```
All of `reverse`, `capitalize`, and `VERSION` are automatically exported.

### Selective Exports (Not Currently Supported)
Dragon does not currently support selective exports - all top-level declarations are exported.

## Module Execution

When a module is imported:
1. The module's source code is loaded and parsed
2. All top-level statements are executed (similar to running the file as a script)
3. Functions, classes, and variables defined at the top level become available for export
4. The module's exports are made available to the importing module

## Circular Dependencies

Dragon does not currently have special handling for circular dependencies. Importing a module that (directly or indirectly) imports back the original module may lead to undefined behavior or runtime errors.

## Standard Library Modules

Dragon's standard library is organized as modules:
- `std/io` - Input/output operations
- `std/math` - Mathematical functions and constants
- `std/json` - JSON parsing and serialization
- `std/file` - File system operations
- `std/net` - Networking operations
- `std/crypto` - Cryptographic functions

## Examples

```dragon
// main.dg
import "std/io";
import "utils/helpers" as helpers;

func main() {
    print("Enter your name: ");
    let name = std.io.readLine();
    let greeting = helpers.formatGreeting(name);
    print("Hello, " + greeting + "!");
}

main();
```

```dragon
// utils/helpers.dg
func formatGreeting(name) {
    return name.trim().toUpperCase();
}

// This function is automatically exported and can be used as:
// helpers.formatGreeting()
```

## Implementation Notes

1. **File Extension**: Dragon modules use the `.dg` file extension.

2. **Path Resolution**: Module paths are resolved relative to the importing file or from standard library paths.

3. **Execution Order**: Modules are executed in dependency order - dependencies are loaded and executed before the importing module.

4. **Caching**: Modules are typically cached after first import to avoid re-execution.

5. **Global Scope**: Each module has its own scope - top-level variables are not globally visible unless exported and imported.

6. **Main Module**: The module passed to the Dragon interpreter is the main module and serves as the entry point.

## Related Files
- `src/engine/parser.rs` - Import statement parsing (lines 567-581)
- `src/engine/` - Runtime module loading and handling
- `examples/` - Example Dragon programs showing module usage
- `spec/language/imports.md` - Import specification (complementary to this file)