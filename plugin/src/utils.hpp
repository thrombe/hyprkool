#pragma once

#include <ctime>
#include <thread>
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/desktop/Workspace.hpp>
#include <hyprland/src/managers/animation/DesktopAnimationManager.hpp>

enum Animation {
    None = 0,
    Left = 1,
    Right = 2,
    Up = 3,
    Down = 4,
    Fade = 5,
};
enum PluginEvent {
    AnimationNone = 0,
    AnimationLeft = 1,
    AnimationRight = 2,
    AnimationUp = 3,
    AnimationDown = 4,
    AnimationFade = 5,
};
extern Animation anim_dir;

extern HANDLE PHANDLE;
extern std::string sock_path;
extern bool exit_flag;
extern int sockfd;
extern std::thread sock_thread;

void err_notif(std::string msg);
void throw_err_notif(std::string msg);

struct KoolConfig {
    int workspaces_x;
    int workspaces_y;
};
extern KoolConfig g_KoolConfig;

void _set_config();
void set_config();
std::string get_socket_path();
