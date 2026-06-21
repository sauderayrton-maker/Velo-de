# Velo DE

A glass-and-steel desktop environment for Wayland.

## Install everything in one line

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/sauderayrton-maker/Velo-de/main/install.sh)
```

Or clone first:

```bash
git clone https://github.com/sauderayrton-maker/Velo-de.git && bash Velo-de/install.sh
```

The script clones and installs all Velo apps:

| App | What it does |
|-----|-------------|
| [velo-shell](https://github.com/sauderayrton-maker/Velo-shell) | Status bar / panel |
| [velo-launcher](https://github.com/sauderayrton-maker/Velo-launcher) | App launcher |
| [velo-osd](https://github.com/sauderayrton-maker/velo-osd) | Volume/brightness OSD + notifications |
| [velo-files](https://github.com/sauderayrton-maker/Velo-files) | File manager |
| [velo-browser](https://github.com/sauderayrton-maker/Velo-Browser) | Web browser |
| [velo-player](https://github.com/sauderayrton-maker/Velo-player) | Music player |
| [velo-assistant](https://github.com/sauderayrton-maker/velo-assistant) | AI assistant |
| [velo-paper](https://github.com/sauderayrton-maker/velo-paper) | Wallpaper tool |
| [velo-commit](https://github.com/sauderayrton-maker/velo-commit) | AI git commit tool |

## Options

```bash
# Install only specific apps
bash install.sh --only=velo-shell --only=velo-launcher

# Skip specific apps
bash install.sh --skip=velo-browser --skip=velo-player
```

## Requirements

- Linux (Arch/Debian/Fedora — detected automatically)
- Wayland compositor (niri or Hyprland)
- Rust — installed automatically if missing
- `git`, `sudo`
