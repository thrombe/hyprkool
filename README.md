# Hyprkool
An opinionated [Hyprland](https://github.com/hyprwm/Hyprland) plugin that tries to replicate the feel of KDE activities and grid layouts.

### Demo Video
Check out our [demo video](https://youtu.be/tim5r6Yo6TA) to see Hyprkool in action:

# Features
- switch desktops when cursor touches screen edges
- grid layout
- info commands for tools like eww and waybar
- an optional daemon for stateful commands
- ~a grid overview~ [overview feature is no longer supported](https://github.com/thrombe/hyprkool/issues/27#issuecomment-2940452377)
- [harpoon](https://github.com/ThePrimeagen/harpoon) but for hyprland workspaces

# Usage
Hyprkool consists of two main components: a CLI + daemon written in Rust and a C++ plugin.
The CLI and daemon collectively provide most of the functionality.
Additionally, there's an optional C++ plugin that offers a couple of features.
- Changing workspace animations based on movement direction.

The daemon component of Hyprkool is also optional but required for certain features, including:
- Desktop switching when the cursor touches screen edges.
- Remembering the last workspace per activity.
- Harpoon for workspaces (named-focus)

# Version Compatibility
The plugin is tested and compatible with the following versions of Hyprland. While the daemon and cli should work with any reasonably new version of Hyprland.

| Hyprland version      | hyprkool version      |
| ------------- | ------------- |
| [v0.39.x](https://github.com/hyprwm/Hyprland/releases/tag/v0.39.1) | [v0.5.x](https://github.com/thrombe/hyprkool/releases/tag/0.5.3) |
| [v0.40.x](https://github.com/hyprwm/Hyprland/releases/tag/v0.40.0) | [v0.6.x](https://github.com/thrombe/hyprkool/releases/tag/0.6.0) |
| [v0.41.0](https://github.com/hyprwm/Hyprland/releases/tag/v0.41.0), [v0.41.1](https://github.com/hyprwm/Hyprland/releases/tag/v0.41.1) | [v0.7.0](https://github.com/thrombe/hyprkool/releases/tag/0.7.0) |
| [v0.41.2](https://github.com/hyprwm/Hyprland/releases/tag/v0.41.2) | [v0.7.1](https://github.com/thrombe/hyprkool/releases/tag/0.7.1) |
| [v0.42.0](https://github.com/hyprwm/Hyprland/releases/tag/v0.42.0), [v0.43.0](https://github.com/hyprwm/Hyprland/releases/tag/v0.43.0), [v0.44.x](https://github.com/hyprwm/Hyprland/releases/tag/v0.44.1) | [v0.7.2](https://github.com/thrombe/hyprkool/releases/tag/0.7.2) |
| [v0.45.x](https://github.com/hyprwm/Hyprland/releases/tag/v0.45.2) | [v0.7.3](https://github.com/thrombe/hyprkool/releases/tag/0.7.3) |
| [v0.46.x](https://github.com/hyprwm/Hyprland/releases/tag/v0.46.2) | [v0.7.4](https://github.com/thrombe/hyprkool/releases/tag/0.7.4) |
| [v0.47.x](https://github.com/hyprwm/Hyprland/releases/tag/v0.47.1) | [v0.7.5](https://github.com/thrombe/hyprkool/releases/tag/0.7.5) |
| [v0.49.0](https://github.com/hyprwm/Hyprland/releases/tag/v0.49.0) | [v0.7.6](https://github.com/thrombe/hyprkool/releases/tag/0.7.6) |
| [v0.50.0](https://github.com/hyprwm/Hyprland/releases/tag/v0.50.0) | [v0.7.7](https://github.com/thrombe/hyprkool/releases/tag/0.7.7) |
| [v0.50.1](https://github.com/hyprwm/Hyprland/releases/tag/v0.50.1) | [v0.8.0](https://github.com/thrombe/hyprkool/releases/tag/0.8.0) |

# Installing Cli/Daemon
<!-- enable when new version of hyprland-rs drops -->
<!-- ### Cargo -->
<!-- ```zsh -->
<!-- cargo install --locked hyprkool -->
<!-- ``` -->

## Install from source
```zsh
git clone https://github.com/thrombe/hyprkool
cd hyprkool
cargo install --locked --path .
```

## Nix
Try it out
```nix
nix run github:thrombe/hyprkool
```

Else add the following to your nix flake
```nix
{
  inputs = {
    # ...
    # define flake input
    hyprkool.url = "github:thrombe/hyprkool";
  };

  # ...

    # then add it to your environment packages
    packages = [
      inputs.hyprkool.packages."${system}".default
    ];

  # ...
}
```

## Installing the Plugin
### using [hyprpm](https://wiki.hyprland.org/0.39.0/Plugins/Using-Plugins/#hyprpm)
```zsh
hyprpm add https://github.com/thrombe/hyprkool
hyprpm enable hyprkool
```

### Nix
It is recommended that you are using Hyprland flake.
You can install hyprkool plugin just like other [hyprland plugins](https://github.com/hyprwm/hyprland-plugins?tab=readme-ov-file#nix).

#### with hyprland as a flake
```nix
{
  inputs = {
    # ...
    hyprland.url = "github:hyprwm/Hyprland";
    hyprkool = {
      url = "github:thrombe/hyprkool";
      inputs.hyprland.follows = "hyprland";
    };
  };

  # ...

    # then, you can use the plugins with the Home Manager module
    {inputs, pkgs, ...}: {
      wayland.windowManager.hyprland = {
        enable = true;
        # ...
        plugins = [
          inputs.hyprkool.packages.${pkgs.system}.hyprkool-plugin
          # ...
        ];
      };
    }

  # ...
}
```

#### with hyprland from nixpkgs
```nix
{
  inputs = {
    # ...
    hyprkool.url = "github:thrombe/hyprkool";
  };

  # ...

    # then, you can use the plugins with the Home Manager module
    {inputs, pkgs, ...}: {
      wayland.windowManager.hyprland = {
        enable = true;
        # ...
        plugins = [
          inputs.hyprkool.packages.${pkgs.system}.hyprkool-plugin.override {
            hyprland = pkgs.hyprland;
          }
          # ...
        ];
      };
    }

  # ...
}
```

# Example Configs
## Configure hyprkool
~/.config/hypr/hyprkool.toml
```toml
# activity names (first activity is treated as default)
# note: only a-z A-Z 0-9 - _ characters are allowed in the name
activities = ["my-default-activity", "my-activity"]

# number of workspaces in x and y dimensions
workspaces = [2, 2]

[daemon]
# remember last focused workspace in an activity
remember_activity_focus = true

# execute fallback commands if daemon cannot be reached
fallback_commands = true

[daemon.mouse]
switch_workspace_on_edge = true

# how often to poll for cursor position
polling_rate = 300 # in ms

# number of pixels to consider as edge
edge_width = 0

# number of pixels to push cursor inside when it loops around
edge_margin = 2
```

## Hyprland config
~/.config/hypr/hyprland.conf
```conf
animations {
  ...

  # i recommend setting workspace animations to fade by default
  # hyprkool plugin will set the animation to slide with appropriate
  # direction when you switch between workspaces
  animation = workspaces, 1, 2, default, fade
}

# Switch activity
bind = $mainMod, TAB, exec, hyprkool next-activity -c

# Move active window to a different acitvity
bind = $mainMod CTRL, TAB, exec, hyprkool next-activity -c -w

# Relative workspace jumps
bind = $mainMod, h, exec, hyprkool move-left -c
bind = $mainMod, l, exec, hyprkool move-right -c
bind = $mainMod, j, exec, hyprkool move-down -c
bind = $mainMod, k, exec, hyprkool move-up -c

# Move active window to a workspace
bind = $mainMod CTRL, h, exec, hyprkool move-left -c -w
bind = $mainMod CTRL, l, exec, hyprkool move-right -c -w
bind = $mainMod CTRL, j, exec, hyprkool move-down -c -w
bind = $mainMod CTRL, k, exec, hyprkool move-up -c -w

# toggle special workspace
bind = $mainMod, SPACE, exec, hyprkool toggle-special-workspace -n minimized
# move active window to special workspace without switching to that workspace
bind = $mainMod, s, exec, hyprkool toggle-special-workspace -n minimized -w -s

# harpoon for workspaces (previously known as named-focus :P)
# switch to named focus
bind = $mainMod, 1, exec, hyprkool switch-named-focus -n 1
bind = $mainMod, 2, exec, hyprkool switch-named-focus -n 2
bind = $mainMod, 3, exec, hyprkool switch-named-focus -n 3
# set / delete named focus
bind = $mainMod SHIFT, 1, exec, hyprkool set-named-focus -n 1
bind = $mainMod SHIFT, 2, exec, hyprkool set-named-focus -n 2
bind = $mainMod SHIFT, 3, exec, hyprkool set-named-focus -n 3

# this is optional, but it can provide features like
# - remembering the last focused workspace in an activity
# - switch workspaces when mouse touches screen edges
# - named focus
exec-once = hyprkool daemon -m

# to load the plugin at startup: https://wiki.hyprland.org/0.39.0/Plugins/Using-Plugins/#hyprpm
exec-once = hyprpm reload -n
```

## Troubleshooting
#### hyprkool move-xxx does not work
For some of the hyprkool commands to work correctly, you need to switch to a hyprkool activity

#### Hyprkool can't find icons?
If hyprkool can't find icons, you can specify the name of the icon pack for hyprkool to use. for example
```zsh
# assuming the Papirus icons are installed
hyprkool info -m active-workspace-windows -t Papirus
```

#### Some command does not work
If a command does not work when using keybinds, try executing the same command in a terminal. Sometimes the error messages
will give you a clue into what could be wrong.
Also try using `--force-no-daemon` flag to check if something is wrong with the running daemon.

#### Hyprkool does not do anything when run using Hyprland keybinds
depending on how you install hyprkool cli, hyprland's `exec` dispatch might have some trouble finding your hyprkool binary.
in such cases, i recommend doing something like this:
first run `which hyprkool` in your terminal and copy the path.
then make the following changes to hyprland.conf
```conf
$hyprkool = "/absolute/path/to/hyprkool"

# then set up any keybinds using this variable
bind = $mainMod, l, exec, $hyprkool move-right
```
## Info commands
Hyprkool supports some additional info commands that help you to build widgets using applications like
[waybar](https://github.com/Alexays/Waybar) and [eww](https://github.com/elkowar/eww).

for example, ```hyprkool info -m active-window``` prints the active window information.

Note: the --monitor or -m flag makes this info print in an infinite loop. this however is very efficient
as it is event based and not polling based.
eww (using [`deflisten`](https://github.com/elkowar/eww/blob/f1ec00a1c9a24c0738fb5d5ac309d6af16e67415/docs/src/configuration.md#adding-dynamic-content))
and waybar (using [`exec`](https://github.com/Alexays/Waybar/wiki/Module:-Custom#continuous-script)) both support
this kind of efficient updates.

### Eww config
Example eww config can be found in [my dotfiles](https://github.com/thrombe/dotfiles-promax/blob/87593cb6ef9718475a3b57ce6a4a2a9727ba2eee/configma/tools/home/.config/eww/eww.yuck).

# Contributing
Contributions are welcome to Hyprkool! If you're fixing a bug, adding a feature, or making an improvement, feel free to submit a pull request (PR) to help enhance the plugin.

### Guidelines:
- Target the `dev` branch for all contributions. This ensures the `master` branch remains stable while we continue to work on new features and fixes.
- Provide clear reproduction steps when fixing a bug. If you're resolving an issue, include detailed instructions on how to reproduce the bug so the fix can be verified.
- Test your changes. If you're introducing new functionality or addressing a bug, ensure everything works as expected.

If you have any questions or concerns about your contribution, don't hesitate to open an issue or ask for feedback.
