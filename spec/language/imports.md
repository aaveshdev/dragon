# Dragon Language Imports

## Status: Implementation-defined (based on current parser implementation)

This document describes the import statement in the Dragon programming language.

## Import Syntax

The import statement allows a module to use functionality from other modules:

### Basic Import
```dragon
import "module_name";
```

### Import with Alias
```dragon
import "module_name" as alias;
```

### Relative Imports
```dragon
import "./relative/path";
import "../parent/path";
```

## Import Resolution

When Dragon encounters an import statement, it resolves the module path as follows:

1. **Relative paths**: If the path starts with `./` or `../`, it is resolved relative to the importing file's directory.
2. **Absolute/module paths**: If the path does not start with `.` or `..`, it is resolved relative to the module search path.
3. **File extension**: The `.dg` extension is automatically appended if not present.
4. **Search paths**: Dragon searches in:
   - The same directory as the importing file
   - Standard library directories
   - Paths specified in environment variables or configuration (if implemented)

## Import Behavior

### Execution
When a module is imported:
1. The module's source code is loaded, parsed, and executed
2. All top-level statements in the module are executed (similar to running the file)
3. Functions, classes, constants, and variables defined at the top level become available
4. The module's exports are made available to the importing module

### Caching
Modules are typically cached after the first successful import. Subsequent imports of the same module return the cached module instead of re-executing it.

### Scope
Imported modules do not pollute the importing module's scope. To access imports:
- Without alias: `module_name.exported_item`
- With alias: `alias.exported_item`

## Examples

```dragon
// Import standard library modules
import "std/io";
import "std/math";

// Import with alias
import "std/io" as io;

// Import local modules
import "utils/helpers";
import "./config";
import "../shared/constants";

// Using imported items
let name = std.io.readLine();
let result = math.sqrt(16);
let helper_result = utils.helpers.formatName(first, last);
let io_result = io.readConfig("settings.json");
```

## Implementation Notes

1. **String literal requirement**: The module path must be a string literal (cannot be a variable or expression).

2. **No selective imports**: Dragon does not currently support importing specific items from a module - all top-level exports are available.

3. **No re-export**: Dragon does not currently support re-exporting items from imported modules.

4. **Error handling**: If a module cannot be found or fails to load, a runtime error is thrown.

5. **Circular imports**: Circular imports (where module A imports B and B imports A) are not handled specially and may cause issues.

6. **Path normalization**: Paths are normalized to handle `./`, `../`, and duplicate slashes.

## Related Files
- `src/engine/parser.rs` - Import statement parsing (lines 567-581)
- `src/engine/` - Runtime module loading and handling
- `spec/language/modules.md` - Module system specification