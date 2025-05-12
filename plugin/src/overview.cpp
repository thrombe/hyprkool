#include <hyprland/src/Compositor.hpp>
#include <hyprland/src/render/OpenGL.hpp>
#include <wlr-layer-shell-unstable-v1.hpp>

#include "overview.hpp"
#include "utils.hpp"

std::regex overview_pattern("([a-zA-Z0-9-_]+):\\(([0-9]+) ([0-9]+)\\):overview");
GridOverview g_go;
const char* HOVER_BORDER_CONFIG_NAME = "plugin:hyprkool:overview:hover_border_color";
const char* FOCUS_BORDER_CONFIG_NAME = "plugin:hyprkool:overview:focus_border_color";
const char* GAP_SIZE_CONFIG_NAME = "plugin:hyprkool:overview:workspace_gap_size";
const char* BORDER_SIZE_CONFIG_NAME = "general:border_size";

void OverviewWorkspace::render(CBox screen, timespec* time) {
    render_hyprland_wallpaper();
    render_bg_layers(time);

    for (auto& w : g_pCompositor->m_windows) {
        if (!w) {
            continue;
        }
        auto& ws = w->m_workspace;
        if (!ws) {
            continue;
        }
        if (ws->m_name != name) {
            continue;
        }
        // TODO: special and pinned windows apparently go on top of everything in that order
        render_window(w, time);
    }

    render_top_layers(time);
}

void OverviewWorkspace::render_window(PHLWINDOW w, timespec* time) {
    auto m = g_pCompositor->getMonitorFromCursor();

    CBox wbox = w->getFullWindowBoundingBox();

    auto o_ws = w->m_workspace;

    w->m_workspace = m->m_activeWorkspace;
    g_pHyprOpenGL->m_renderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});
    g_pHyprOpenGL->m_renderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});

    // TODO: damaging window like this doesn't work very well :/
    //       maybe set the pos and size before damaging
    // g_pHyprRenderer->damageWindow(w);
    (*(FuncRenderWindow)renderWindow)(g_pHyprRenderer.get(), w, m, time, true, RENDER_PASS_MAIN, false, false);

    w->m_workspace = o_ws;
    g_pHyprOpenGL->m_renderData.renderModif.modifs.pop_back();
    g_pHyprOpenGL->m_renderData.renderModif.modifs.pop_back();
}

void OverviewWorkspace::render_layer(PHLLS layer, timespec* time) {
    auto m = g_pCompositor->getMonitorFromCursor();

    g_pHyprOpenGL->m_renderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});
    g_pHyprOpenGL->m_renderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});

    (*(FuncRenderLayer)renderLayer)(g_pHyprRenderer.get(), layer, m, time, false);

    g_pHyprOpenGL->m_renderData.renderModif.modifs.pop_back();
    g_pHyprOpenGL->m_renderData.renderModif.modifs.pop_back();
}

void OverviewWorkspace::render_hyprland_wallpaper() {
    g_pHyprOpenGL->m_renderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});
    g_pHyprOpenGL->m_renderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});

    g_pHyprOpenGL->clearWithTex();

    g_pHyprOpenGL->m_renderData.renderModif.modifs.pop_back();
    g_pHyprOpenGL->m_renderData.renderModif.modifs.pop_back();
}

void OverviewWorkspace::render_bg_layers(timespec* time) {
    auto m = g_pCompositor->getMonitorFromCursor();
    for (auto& layer : m->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND]) {
        auto locked = layer.lock();
        render_layer(locked, time);
    }
    for (auto& layer : m->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM]) {
        auto locked = layer.lock();
        render_layer(locked, time);
    }
}

void OverviewWorkspace::render_top_layers(timespec* time) {
    auto m = g_pCompositor->getMonitorFromCursor();
    for (auto& layer : m->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_TOP]) {
        auto locked = layer.lock();
        render_layer(locked, time);
    }
    // for (auto& layer : m->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY]) {
    //     render_layer(layer.get(), time);
    // }
}

void OverviewWorkspace::render_border(CBox bbox, CHyprColor col, int border_size) {
    bbox.expand(-border_size);
    bbox.round();
    bbox.w = std::max(bbox.w, 1.0);
    bbox.h = std::max(bbox.h, 1.0);
    CGradientValueData grad = {col};

    g_pHyprOpenGL->renderBorder(bbox, grad, 0, 0, border_size);
}

void GridOverview::init() {
    static auto* const* CURSOR_WS_BORDER =
        (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, HOVER_BORDER_CONFIG_NAME)->getDataStaticPtr();
    static auto* const* FOCUS_BORDER =
        (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, FOCUS_BORDER_CONFIG_NAME)->getDataStaticPtr();
    static auto* const* BORDER_SIZE =
        (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, BORDER_SIZE_CONFIG_NAME)->getDataStaticPtr();
    static auto* const* GAP_SIZE =
        (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, GAP_SIZE_CONFIG_NAME)->getDataStaticPtr();
    cursor_ws_border = CHyprColor(**CURSOR_WS_BORDER);
    focus_border = CHyprColor(**FOCUS_BORDER);
    border_size = **BORDER_SIZE;

    auto m = g_pCompositor->getMonitorFromCursor();
    auto& w = m->m_activeWorkspace;

    if (std::regex_match(w->m_name, overview_pattern)) {
        auto ss = std::istringstream(w->m_name);
        std::getline(ss, activity, ':');
    } else {
        throw_err_notif("can't display overview when not in a hyprkool activity");
    }
    box.x = m->m_position.x;
    box.y = m->m_position.y;
    box.w = m->m_size.x * m->m_scale;
    box.h = m->m_size.y * m->m_scale;

    float gap_size = **GAP_SIZE/2.0;
    float dx = g_KoolConfig.workspaces_x;
    float dy = g_KoolConfig.workspaces_y;

    float scalex = ((box.w - (dx + 1) * gap_size)/box.w) / dx;
    float scaley = ((box.h - (dy + 1) * gap_size)/box.h) / dy;
    float scale = std::min(scalex, scaley);

    float w_gap = (box.w * (1.0 - scale * dx)) / (dx + 1);
    float h_gap = (box.h * (1.0 - scale * dy)) / (dy + 1);

    for (int y = 0; y < g_KoolConfig.workspaces_y; y++) {
        for (int x = 0; x < g_KoolConfig.workspaces_x; x++) {
            auto ow = OverviewWorkspace();
            ow.name = activity + ":(" + std::to_string(x + 1) + " " + std::to_string(y + 1) + ")";
            ow.box = box;
            ow.box.x += box.w * x;
            ow.box.y += box.h * y;
            ow.box.scale(scale);

            ow.box.x += w_gap * (x + 1);
            ow.box.y += h_gap * (y + 1);

            ow.scale = scale;
            workspaces.push_back(ow);
        }
    }
}

void GridOverview::render() {
    timespec time;
    clock_gettime(CLOCK_MONOTONIC, &time);

    // TODO: rounding
    // TODO: clicks should not go to the hidden layers (top layer)
    // TODO: draggable overlay windows
    // try to make dolphin render bg

    auto br = g_pHyprOpenGL->m_renderData.pCurrentMonData->blurFBShouldRender;
    auto o_modif = g_pHyprOpenGL->m_renderData.renderModif.enabled;

    g_pHyprOpenGL->m_renderData.pCurrentMonData->blurFBShouldRender = true;
    g_pHyprOpenGL->m_renderData.clipBox = box;
    g_pHyprOpenGL->m_renderData.renderModif.enabled = true;

    g_pHyprOpenGL->renderRectWithBlur(box, CHyprColor(0.0, 0.0, 0.0, 1.0));

    for (auto& ow : workspaces) {
        ow.render(box, &time);
    }

    auto m = g_pCompositor->getMonitorFromCursor();
    auto& aw = m->m_activeWorkspace;

    auto mouse = g_pInputManager->getMouseCoordsInternal() * m->m_scale;
    bool did_render_focus_ws_border = false;
    bool did_render_cursor_ws_border = false;
    for (auto& w : g_pCompositor->m_windows) {
        if (!w) {
            continue;
        }
        auto& ws = w->m_workspace;
        if (!ws) {
            continue;
        }
        for (auto& ow : workspaces) {
            if (ws->m_name == ow.name) {
                CBox wbox = w->getFullWindowBoundingBox();
                wbox.scale(ow.scale * m->m_scale);
                wbox.translate(ow.box.pos());
                wbox.expand(-1);
                wbox.round();

                if (aw->m_name.starts_with(ow.name) && ws->getLastFocusedWindow().get() == w.get()) {
                    ow.render_border(wbox, g_go.focus_border, g_go.border_size);
                    did_render_focus_ws_border = true;
                }

                if (wbox.containsPoint(mouse)) {
                    ow.render_border(wbox, g_go.cursor_ws_border, g_go.border_size);
                    did_render_cursor_ws_border = true;
                }
            }
        }
    }

    if (!did_render_focus_ws_border) {
        for (auto& ow : workspaces) {
            if (aw->m_name.starts_with(ow.name)) {
                ow.render_border(ow.box.copy().expand(border_size), focus_border, border_size);
            }
        }
    }

    if (!did_render_cursor_ws_border) {
        for (auto& ow : workspaces) {
            if (ow.box.containsPoint(mouse)) {
                ow.render_border(ow.box.copy().expand(border_size), cursor_ws_border, border_size);
            }
        }
    }

    g_pHyprOpenGL->m_renderData.pCurrentMonData->blurFBShouldRender = br;
    g_pHyprOpenGL->m_renderData.clipBox = CBox();
    g_pHyprOpenGL->m_renderData.renderModif.enabled = o_modif;
}
