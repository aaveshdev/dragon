# Deploying Dragon on Windows

## Status: Documented (based on current implementation)

This document describes how to deploy Dragon applications on Windows systems.

## Installation

### From Binary Release
Download the Windows binary from the [Dragon releases page](https://github.com/aaveshdev/dragon/releases):
- Look for `dragon-x.y.z-windows.exe`
- Rename to `dragon.exe` if desired
- Place in a directory on your PATH (e.g., `C:\Windows\System32` or a custom bin directory)

### From Source
```powershell
# Install Rust if needed
# Visit https://www.rust-lang.org/tools/install

# Clone and build
git clone https://github.com/smitshetye/dragon.git
cd dragon
cargo build --release

# Install globally
cargo install --path .
```

## Running Applications

### Running Scripts
```cmd
dragon path\to\script.dragon
```

### Running with Arguments
```cmd
dragon script.dragon arg1 arg2
:: Access args in Dragon code via argv array
```

### Using the REPL
```cmd
dragon --repl
```

### Using PowerShell
```powershell
dragon script.dragon
```

## Deployment Considerations

### File Permissions
- Ensure the dragon executable has execute permissions (however Windows handles this)
- Script files need read permissions
- Ensure the application has permission to access required resources (files, network, etc.)

### PATH Environment Variable
To run dragon from any command prompt:
1. Locate the directory containing `dragon.exe`
2. Open System Properties → Advanced → Environment Variables
3. Under System Variables, find Path, click Edit
4. New → paste the directory path
5. OK all dialogs

### File Paths
Windows uses backslashes (`\`) as path separators, but Dragon strings use forward slashes (`/`) or escaped backslashes:
```dragon
// Both work in Dragon:
let path1 = "C:/Users/Name/Documents/file.txt"
let path2 = "C:\\Users\\Name\\Documents\\file.txt"
```

## Service Deployment

### Running as a Windows Service
To run a Dragon application as a Windows service, you'll need to use a wrapper like:
- [nssm](https://nssm.cc/) (Non-Sucking Service Manager)
- [winsw](https://github.com/winsw/winsw)
- Or create a Windows Service in C#/.NET that hosts the Dragon process

Example with nssm:
```cmd
nssm install DragonApp
# Set path to dragon.exe
# Set startup directory
# Set arguments: path/to/main.dragon
nssm start DragonApp
```

## Security Considerations

### Sandboxing
Dragon does not currently provide built-in sandboxing. Applications run with the same permissions as the user running the dragon executable.

### Capabilities to Consider
Dragon provides access to:
- File system read/write (where permitted)
- Network communication (HTTP, sockets)
- Process spawning
- Foreign function interface (if using std/ffi)
- Environment variables
- Command line arguments
- Registry access (through FFI or std/os if implemented)

### Best Practices
- Run applications with least privilege necessary
- Validate all external input
- Keep Dragon and dependencies updated
- Consider using containers or VMs for isolation
- Be careful with file paths and security implications

## Troubleshooting

### Common Issues
- **"'dragon' is not recognized as an internal or external command"**: Ensure dragon is in PATH
- **Access denied errors**: Check file permissions and run as administrator if needed
- **Missing DLL errors**: May need to install Visual C++ Redistributable
- **"The system cannot find the path specified"**: Check file paths and naming

### Getting Help
- Check the output of `dragon --version` to verify installation
- Look at error messages for clues
- Consult the [documentation](../docs/) for more information
- Ask in the Dragon community forums

## Related Files
- [README.md](./../README.md) - General Dragon usage
- [spec/standard-library/os.md](./spec/standard-library/os.md) - OS module specification
- [docs/deployment/](./../docs/deployment/) - End-user deployment guides