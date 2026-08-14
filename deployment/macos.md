# Deploying Dragon on macOS

## Status: Documented (based on current implementation)

This document describes how to deploy Dragon applications on macOS systems.

## Installation

### From Binary Release
Download the macOS binary from the [Dragon releases page](https://github.com/aaveshdev/dragon/releases):
- Look for `dragon-x.y.z-macos`
- Rename to `dragon` if desired
- Make executable: `chmod +x dragon`
- Place in a directory on your PATH (e.g., `/usr/local/bin` or `~/bin`)

### Using Homebrew (if available)
```bash
# If Dragon is available via Homebrew
brew install dragon
```

### From Source
```bash
# Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone https://github.com/smitshetye/dragon.git
cd dragon
cargo build --release

# Install globally
cargo install --path .
```

## Running Applications

### Running Scripts
```bash
dragon path/to/script.dragon
```

### Running with Arguments
```bash
dragon script.dragon arg1 arg2
# Access args in Dragon code via argv array
```

### Using the REPL
```bash
dragon --repl
```

## Deployment Considerations

### File Permissions
- Ensure the dragon executable has execute permissions: `chmod +x dragon`
- Script files need read permissions
- Ensure the application has permission to access required resources (files, network, etc.)

### Gatekeeper
On newer macOS versions, you may need to bypass Gatekeeper:
```bash
# If you see "dragon cannot be opened because the developer cannot be verified"
xattr -d com.apple.quarantine dragon
# Or allow in System Preferences → Security & Privacy
```

### File Paths
macOS uses forward slashes (`/`) as path separators, consistent with Dragon strings:
```dragon
let path = "/Users/Name/Documents/file.txt"
```

## Service Deployment

### Running as a LaunchAgent
To run a Dragon application as a macOS launch agent:

Create `~/Library/LaunchAgents/com.example.dragonapp.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.example.dragonapp</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/dragon</string>
        <string>/path/to/main.dragon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
```

Then load it:
```bash
launchctl load ~/Library/LaunchAgents/com.example.dragonapp.plist
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

### Best Practices
- Run applications with least privilege necessary
- Validate all external input
- Keep Dragon and dependencies updated
- Consider using containers or VMs for isolation
- Monitor application resource usage

## Troubleshooting

### Common Issues
- **"command not found"**: Ensure dragon is in PATH or use full path
- **"permission denied"**: Check file permissions
- **"dragon: command not found" after installation**: May need to restart terminal or source profile
- **Gatekeeper blocking**: Use `xattr` to remove quarantine attribute or adjust security settings

### Getting Help
- Check the output of `dragon --version` to verify installation
- Look at error messages for clues
- Consult the [documentation](../docs/) for more information
- Ask in the Dragon community forums

## Related Files
- [README.md](./../README.md) - General Dragon usage
- [spec/standard-library/os.md](./spec/standard-library/os.md) - OS module specification
- [docs/deployment/](./../docs/deployment/) - End-user deployment guides