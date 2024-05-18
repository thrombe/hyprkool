#pragma once

#include <ctime>
#include <hyprland/src/helpers/WLClasses.hpp>
#include <regex>

class OverviewWorkspace {
  public:
    std::string name;
    CBox box;
    float scale;

    void render(CBox screen, timespec* time);
    void render_window(PHLWINDOW w, timespec* time);
    void render_layer(PHLLS layer, timespec* time);
    void render_hyprland_wallpaper();
    void render_bg_layers(timespec* time);
    void render_top_layers(timespec* time);
    void render_border(CBox bbox, CColor col, int border_size);
};

class GridOverview {
  public:
    std::string activity;
    std::vector<OverviewWorkspace> workspaces;
    CBox box;
    CColor cursor_ws_border;
    CColor focus_border;
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
