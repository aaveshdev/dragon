# Deploying Dragon on Linux

## Status: Documented (based on current implementation)

This document describes how to deploy Dragon applications on Linux systems.

## Installation

### From Package Manager
Some Linux distributions may package Dragon:
```bash
# Debian/Ubuntu (hypothetical)
sudo apt install dragon

# Fedora (hypothetical)
sudo dnf install dragon

# Arch Linux (hypothetical)
sudo pacman -S dragon
```

### From Binary Release
Download the Linux binary from the [Dragon releases page](https://github.com/aaveshdev/dragon/releases) and make it executable:
```bash
chmod +x dragon-linux
sudo mv dragon-linux /usr/local/bin/dragon
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

### Environment Variables
Dragon may use environment variables for configuration:
- `DRAGON_HOME` - Alternative home directory for Dragon
- `DRAGON_PATH` - Additional module search paths
- `DRAGON_ENABLE_NATIVE` - Enable experimental native features

### System Dependencies
Some Dragon features require system dependencies:
- **Native compilation** (`--native`): Requires gcc/clang and make
- **HTTP requests**: Requires working network connection
- **File system operations**: Requires appropriate file permissions
- **Child processes**: Requires ability to spawn processes

## Service Deployment

### Running as a Service
To run a Dragon application as a systemd service:

Create `/etc/systemd/system/dragon-app.service`:
```ini
[Unit]
Description=Dragon Application
After=network.target

[Service]
Type=simple
User=dragonuser
WorkingDirectory=/opt/dragon-app
ExecStart=/usr/local/bin/dragon /opt/dragon-app/main.dragon
Restart=always
Environment=PATH=/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=multi-user.target
```

Then enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable dragon-app
sudo systemctl start dragon-app
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
- **"permission denied"**: Check file permissions and SELinux/AppArmor
- **"module not found"**: Check module paths and installation
- **"dragon: command not found" after installation**: May need to restart shell or source profile

### Getting Help
- Check the output of `dragon --version` to verify installation
- Look at error messages for clues
- Consult the [documentation](../docs/) for more information
- Ask in the Dragon community forums

## Related Files
- [README.md](./../README.md) - General Dragon usage
- [spec/standard-library/os.md](./spec/standard-library/os.md) - OS module specification
- [docs/deployment/](./../docs/deployment/) - End-user deployment guides