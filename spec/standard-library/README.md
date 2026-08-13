# Dragon Language Standard Library

This directory contains the specification of the Dragon standard library.

## Files

- `math.md` - Mathematical functions and constants
- `json.md` - JSON parsing and serialization
- `file.md` - File system operations
- `path.md` - Path manipulation utilities
- `os.md` - Operating system interface
- `time.md` - Time and date functions
- `net.md` - Networking operations
- `crypto.md` - Cryptographic functions
- `zip.md` - ZIP archive handling
- `xml.md` - XML parsing and manipulation
- `uuid.md` - UUID generation and parsing
- `pdf.md` - PDF generation and manipulation
- `font.md` - Font handling and text rendering
- `ffi.md` - Foreign Function Interface
- `struct.md` - Structured data handling
- `mem.md` - Manual memory management

## Source of Truth

The standard library specification describes the available modules and their APIs. Where the specification is incomplete or marked as implementation-defined, refer to the reference implementation for current behavior.

## Stability

Standard library modules may have different stability levels:
- **Stable**: API is guaranteed not to break in backward-incompatible ways
- **Experimental**: API may change based on feedback
- **Deprecated**: API is discouraged and may be removed in future versions

Check individual module documentation for stability information.