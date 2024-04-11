# Hyprkool
An opinionated [Hyprland](https://github.com/hyprwm/Hyprland) IPC plugin that tries to replicate the feel of KDE activities and grid layouts.

# Features
- ability to switch desktops when cursor touches screen edges
- grid layout
- info commands for tools like eww and waybar
- an optional daemon for stateful commands

# Limitations
- Hyprland IPC plugins can not yet control animation directionality

# Installation
## Cargo
```zsh
cargo install --locked hyprkool
```

## Install from source
```zsh
git clone https://github.com/thrombe/hyprkool
cd hyprkool
cargo install --path .
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
    ...

    # define flake input
    hyprkool.url = "github:thrombe/hyprkool";
  };

  ...

    # then add it to your environment packages
    packages = [
      inputs.hyprkool.packages."${system}".default
    ];

  ...
}
```

# Example Configs
## Configure hyprkool
~/.config/hypr/hyprkool.toml
```toml
# activity names (first activity is treated as default)
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

  # animations work fine, but afaik there is no way to control
  # which side the workspaces slide from as a hyprland IPC plugin
  # so i recommend either turning off animations for workspaces
  # or using animation styles that do not have directionality. (eg fade)
  animation = workspaces, 0
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
Example eww config can be found in [my dotfiles](https://github.com/thrombe/dotfiles-promax/blob/372a47c0a7ed3c3280e110755803ee422c7c4977/configma/tools/home/.config/eww/eww.yuck)


### Waybar config
it simply uses the unicode Full block characters '█' to show activities.
it looks something like this
<br>
![activity status indicator](./screenshots/activity_status.png)

~/.config/waybar/config
```json
{
  ...

  "custom/hyprkool-workspaces": {
    "format": "{}",
    "return-type": "json",
    "exec": "hyprkool info -m waybar-activity-status"
  },
  "custom/hyprkool-window": {
    "format": "{}",
    "return-type": "json",
    "exec": "hyprkool info -m waybar-active-window",
  },
}
```

~/.config/waybar/style.css
```css
#custom-hyprkool-workspaces {
  border: none;
  font-size: 7px;
  color: #ebdbb2;
  background: #7c6f64;
  border-radius: 3px;
  padding: 2px;
  margin: 1px;
}
```
