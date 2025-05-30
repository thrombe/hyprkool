#pragma once

#include <ctime>
#include <hyprland/src/helpers/WLClasses.hpp>
#include <hyprland/src/protocols/PresentationTime.hpp>
#include <regex>

class OverviewWorkspace {
  public:
    std::string name;
    CBox box;
    float scale;

    void render(CBox screen, const Time::steady_tp& time);
    void render_window(PHLWINDOW w, const Time::steady_tp& time);
    void render_layer(PHLLS layer, const Time::steady_tp& time);
    void render_hyprland_wallpaper();
    void render_bg_layers(const Time::steady_tp& time);
    void render_top_layers(const Time::steady_tp& time);
    void render_border(CBox bbox, CHyprColor col, int border_size);
};

class GridOverview {
  public:
    std::string activity;
    std::vector<OverviewWorkspace> workspaces;
    CBox box;
    CHyprColor cursor_ws_border;
    CHyprColor focus_border;
    int border_size;

    void init();
    void render();
};

extern std::regex overview_pattern;
extern GridOverview g_go;
extern const char* HOVER_BORDER_CONFIG_NAME;
extern const char* FOCUS_BORDER_CONFIG_NAME;
extern const char* GAP_SIZE_CONFIG_NAME;
extern const char* GAP_SIZE_CONFIG_NAME;
