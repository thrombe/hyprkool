# Hyprkool
An opinionated [Hyprland](https://github.com/hyprwm/Hyprland) plugin that tries to replicate the feel of KDE activities and grid layouts.

# Features
- ability to switch desktops when cursor touches screen edges
- grid layout
- a super simple workspace indicator using waybar

# Limitations
- fixed 9 workspaces per activity
- hyprland plugins can not yet control animation directionality

# todo
- [ ] desktop grid overview
- [ ] better animations support

# Installation
## Cargo
```zsh
git clone https://github.com/thrombe/waykool
cd waykool
cargo install --path .
```

## Nix
Try it out
```
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

# Example Config
~/.config/hypr/hyprkool.toml
```toml
# activity names (first activity is treated as default)
activities = ["my-default-activity", "my-activity"]

# how often to poll for cursor position
polling_rate = 300 # in ms

# number of pixels to consider as edge
edge_width = 0

# number of pixels to push cursor inside when it loops around
edge_margin = 2
```

~/.config/hypr/hyprland.conf
```conf
animations {
  ...

  # animations work fine, but afaik there is no way to control
  # which side the workspaces slide from as a hyprland plugin
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

# switch workspaces when mouse touches any of the edges
exec-once = hyprkool mouse-loop
```

# Waybar config for a simple workspace indicator
it simply uses the unicode Full block characters '█' to show activities.
it looks something like this
![activity status indicator](./screenshots/activity_status.png)

~/.config/waybar/config
```json
{
  ...

	"custom/hyprkool-workspaces": {
		"format": "{}",
		"return-type": "json",
		"exec": "hyprkool print-activity-status"
	}
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
