#include <hyprland/src/Compositor.hpp>
#include <hyprland/src/managers/input/InputManager.hpp>
#include <hyprland/src/plugins/PluginAPI.hpp>

#include "overview.hpp"
#include "utils.hpp"

std::regex overview_pattern("([a-zA-Z0-9-_]+):\\(([0-9]+) ([0-9]+)\\):overview");
GridOverview g_go;
const char* CURSOR_WS_BORDER_CONFIG_NAME = "plugin:hyprkool:overview:cursor_ws_border";
const char* FOCUS_WS_BORDER_CONFIG_NAME = "plugin:hyprkool:overview:focus_ws_border";
const char* BORDER_SIZE_CONFIG_NAME = "general:border_size";

bool OverviewWorkspace::render(CBox screen, timespec* time) {
    render_hyprland_wallpaper();
    render_bg_layers(time);

    auto mouse = g_pInputManager->getMouseCoordsInternal();
    auto did_render_border = false;
    for (auto& w : g_pCompositor->m_vWindows) {
        if (!w) {
            continue;
        }
        auto& ws = w->m_pWorkspace;
        if (!ws) {
            continue;
        }
        if (ws->m_szName != name) {
            continue;
        }
        // TODO: special and pinned windows apparently go on top of everything in that order
        render_window(w.get(), time);

        CBox wbox = w->getFullWindowBoundingBox();
        wbox.translate(box.pos());
        wbox.scale(scale);
        wbox.expand(-1);
        wbox.round();
        if (wbox.containsPoint(mouse)) {
            // TODO: grab border size and colors
            render_border(wbox, CColor(1.0, 0.0, 0.0, 1.0), 1);
            did_render_border = true;
        }
    }

    render_top_layers(time);
    return did_render_border;
}

void OverviewWorkspace::render_window(CWindow* w, timespec* time) {
    auto& m = g_pCompositor->m_vMonitors[0];

    CBox wbox = w->getFullWindowBoundingBox();

    auto o_ws = w->m_pWorkspace;

    w->m_pWorkspace = m->activeWorkspace;
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});

    // TODO: damaging window like this doesn't work very well :/
    //       maybe set the pos and size before damaging
    // g_pHyprRenderer->damageWindow(w);
    (*(FuncRenderWindow)renderWindow)(g_pHyprRenderer.get(), w, m.get(), time, true, RENDER_PASS_MAIN, false, false);

    w->m_pWorkspace = o_ws;
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
}

void OverviewWorkspace::render_layer(SLayerSurface* layer, timespec* time) {
    auto& m = g_pCompositor->m_vMonitors[0];

    g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});

    (*(FuncRenderLayer)renderLayer)(g_pHyprRenderer.get(), layer, m.get(), time, false);

    g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
}

void OverviewWorkspace::render_hyprland_wallpaper() {
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});

    g_pHyprOpenGL->clearWithTex();

    g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
    g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
}

void OverviewWorkspace::render_bg_layers(timespec* time) {
    auto& m = g_pCompositor->m_vMonitors[0];
    for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND]) {
        render_layer(layer.get(), time);
    }
    for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM]) {
        render_layer(layer.get(), time);
    }
}

void OverviewWorkspace::render_top_layers(timespec* time) {
    auto& m = g_pCompositor->m_vMonitors[0];
    for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_TOP]) {
        render_layer(layer.get(), time);
    }
    // for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY]) {
    //     render_layer(layer.get(), time);
    // }
}

void OverviewWorkspace::render_border(CBox bbox, CColor col, int border_size) {
    bbox.expand(-border_size);
    bbox.round();
    CGradientValueData grad = {col};

    g_pHyprOpenGL->renderBorder(&bbox, grad, 0, border_size);
}

void GridOverview::init() {
    static auto* const* CURSOR_WS_BORDER =
        (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, CURSOR_WS_BORDER_CONFIG_NAME)->getDataStaticPtr();
    static auto* const* FOCUS_BORDER =
        (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, FOCUS_WS_BORDER_CONFIG_NAME)->getDataStaticPtr();
    static auto* const* BORDER_SIZE =
        (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, BORDER_SIZE_CONFIG_NAME)->getDataStaticPtr();
    cursor_ws_border = CColor(**CURSOR_WS_BORDER);
    focus_border = CColor(**FOCUS_BORDER);
    border_size = **BORDER_SIZE;

    auto& m = g_pCompositor->m_vMonitors[0];
    auto& w = m->activeWorkspace;

    if (std::regex_match(w->m_szName, overview_pattern)) {
        auto ss = std::istringstream(w->m_szName);
        std::getline(ss, activity, ':');
    } else {
        throw_err_notif("can't display overview when not in a hyprkool activity");
    }
    box.x = m->vecPosition.x;
    box.y = m->vecPosition.y;
    box.w = m->vecSize.x;
    box.h = m->vecSize.y;

    float scale = 1.0 / (float)std::max(g_KoolConfig.workspaces_x, g_KoolConfig.workspaces_y);

    for (int y = 0; y < g_KoolConfig.workspaces_y; y++) {
        for (int x = 0; x < g_KoolConfig.workspaces_x; x++) {
            auto ow = OverviewWorkspace();
            ow.name = activity + ":(" + std::to_string(x + 1) + " " + std::to_string(y + 1) + ")";
            ow.box = box;
            ow.box.x += box.w * x;
            ow.box.y += box.h * y;
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

    auto br = g_pHyprOpenGL->m_RenderData.pCurrentMonData->blurFBShouldRender;
    auto o_modif = g_pHyprOpenGL->m_RenderData.renderModif.enabled;

    g_pHyprOpenGL->m_RenderData.pCurrentMonData->blurFBShouldRender = true;
    g_pHyprOpenGL->m_RenderData.clipBox = box;
    g_pHyprOpenGL->m_RenderData.renderModif.enabled = true;

    // g_pHyprOpenGL->renderRectWithBlur(&box, CColor(0.0, 0.0, 0.0, 1.0));

    auto did_render_border = false;
    for (auto& ow : workspaces) {
        auto r = ow.render(box, &time);
        did_render_border = did_render_border || r;
    }

    auto& m = g_pCompositor->m_vMonitors[0];
    auto& w = m->activeWorkspace;
    auto mouse = g_pInputManager->getMouseCoordsInternal();
    for (auto& ow : workspaces) {
        if (w->m_szName.starts_with(ow.name)) {
            ow.render_border(ow.box.copy().scale(ow.scale), focus_border, border_size);
        }
        if (ow.box.copy().scale(ow.scale).containsPoint(mouse) && !did_render_border) {
            ow.render_border(ow.box.copy().scale(ow.scale), cursor_ws_border, border_size);
        }
    }

    g_pHyprOpenGL->m_RenderData.pCurrentMonData->blurFBShouldRender = br;
    g_pHyprOpenGL->m_RenderData.clipBox = CBox();
    g_pHyprOpenGL->m_RenderData.renderModif.enabled = o_modif;
}
