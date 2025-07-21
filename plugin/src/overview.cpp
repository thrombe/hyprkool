#include <hyprland/src/Compositor.hpp>
#include <hyprland/src/render/OpenGL.hpp>
#include <wlr-layer-shell-unstable-v1.hpp>
#include <hyprland/src/render/pass/RectPassElement.hpp>
#include <hyprland/src/render/pass/BorderPassElement.hpp>
#include <hyprland/src/render/pass/RendererHintsPassElement.hpp>
#include <hyprlang.hpp>
#include <hyprutils/utils/ScopeGuard.hpp>

#include "overview.hpp"
#include "utils.hpp"

std::regex overview_pattern("([a-zA-Z0-9-_]+):\\(([0-9]+) ([0-9]+)\\):overview");
GridOverview g_go;
const char* HOVER_BORDER_CONFIG_NAME = "plugin:hyprkool:overview:hover_border_color";
const char* FOCUS_BORDER_CONFIG_NAME = "plugin:hyprkool:overview:focus_border_color";
const char* GAP_SIZE_CONFIG_NAME = "plugin:hyprkool:overview:workspace_gap_size";
const char* BORDER_SIZE_CONFIG_NAME = "general:border_size";

void OverviewWorkspace::render(CBox screen, const Time::steady_tp& time) {
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

void OverviewWorkspace::render_window(PHLWINDOW w, const Time::steady_tp& time) {
    auto m = g_pCompositor->getMonitorFromCursor();

    CBox wbox = w->getFullWindowBoundingBox();

    auto o_ws = w->m_workspace;
    w->m_workspace = m->m_activeWorkspace;

    SRenderModifData renderModif;
    renderModif.enabled = true;

    renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});
    renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});

    g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{renderModif}));
    Hyprutils::Utils::CScopeGuard x([] {
        g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{SRenderModifData{}}));
        });

    // TODO: damaging window like this doesn't work very well :/
    //       maybe set the pos and size before damaging
    // g_pHyprRenderer->damageWindow(w);
    (*(FuncRenderWindow)renderWindow)(g_pHyprRenderer.get(), w, m, time, true, RENDER_PASS_MAIN, false, false);

    w->m_workspace = o_ws;
}

void OverviewWorkspace::render_layer(PHLLS layer, const Time::steady_tp& time) {
    auto monitor = g_pCompositor->getMonitorFromCursor();

    SRenderModifData renderModif;
    renderModif.enabled = true;

    renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});
    renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});

    g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{renderModif}));
    Hyprutils::Utils::CScopeGuard x([] {
        g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{SRenderModifData{}}));
        });

    (*(FuncRenderLayer)renderLayer)(g_pHyprRenderer.get(), layer, monitor, time, false, false);

    // static auto PBLUR = CConfigValue<Hyprlang::INT>("decoration:blur:enabled");
    // const auto                       REALPOS = pLayer->m_realPosition->value();
    // const auto                       REALSIZ = pLayer->m_realSize->value();

    // CSurfacePassElement::SRenderData renderdata = {pMonitor, time, REALPOS};
    // renderdata.fadeAlpha                        = pLayer->m_alpha->value();
    // renderdata.blur                             = pLayer->m_forceBlur && *PBLUR;
    // renderdata.surface                          = pLayer->m_surface->resource();
    // renderdata.decorate                         = false;
    // renderdata.w                                = REALSIZ.x;
    // renderdata.h                                = REALSIZ.y;
    // renderdata.pLS                              = pLayer;
    // renderdata.blockBlurOptimization            = pLayer->m_layer == ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM || pLayer->m_layer == ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND;

    // // renderdata.clipBox = box.scale(scale);
    // renderdata.clipBox = CBox(box.x, box.y, 100, 100);

    // if (renderdata.blur && pLayer->m_ignoreAlpha) {
    //     renderdata.discardMode |= DISCARD_ALPHA;
    //     renderdata.discardOpacity = pLayer->m_ignoreAlphaValue;
    // }

    //     pLayer->m_surface->resource()->breadthfirst(
    //         [this, &renderdata, &pLayer](SP<CWLSurfaceResource> s, const Vector2D& offset, void* data) {
    //             renderdata.localPos    = offset;
    //             renderdata.texture     = s->m_current.texture;
    //             renderdata.surface     = s;
    //             renderdata.mainSurface = s == pLayer->m_surface->resource();
    //             g_pHyprRenderer->m_renderPass.add(makeUnique<CSurfacePassElement>(renderdata));
    //             renderdata.surfaceCounter++;
    //         },
    //         &renderdata);
}

void OverviewWorkspace::render_hyprland_wallpaper() {
    SRenderModifData renderModif;
    renderModif.enabled = true;

    renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});
    renderModif.modifs.push_back(
        {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});

    g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{renderModif}));
    Hyprutils::Utils::CScopeGuard x([] {
        g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{SRenderModifData{}}));
        });

    g_pHyprOpenGL->clearWithTex();
}

void OverviewWorkspace::render_bg_layers(const Time::steady_tp& time) {
    auto m = g_pCompositor->getMonitorFromCursor();
    // for (auto& layer : m->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND]) {
    //     auto locked = layer.lock();
    //     render_layer(locked, time);
    // }
    for (auto& layer : m->m_layerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM]) {
        auto locked = layer.lock();
        render_layer(locked, time);
    }
}

void OverviewWorkspace::render_top_layers(const Time::steady_tp& time) {
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

    CBorderPassElement::SBorderData data;
    data.box = bbox;
    data.grad1 = grad;
    data.round = 0;
    data.a = 1.f;
    data.borderSize = border_size;
    g_pHyprRenderer->m_renderPass.add(makeUnique<CBorderPassElement>(data));
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
    const auto time = Time::steadyNow();

    // TODO: rounding
    // TODO: clicks should not go to the hidden layers (top layer)
    // TODO: draggable overlay windows
    // try to make dolphin render bg

    SRenderModifData renderModif;
    renderModif.enabled = true;

    g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{renderModif}));
    Hyprutils::Utils::CScopeGuard x([] {
        g_pHyprRenderer->m_renderPass.add(makeUnique<CRendererHintsPassElement>(CRendererHintsPassElement::SData{SRenderModifData{}}));
        });

    auto br = g_pHyprOpenGL->m_renderData.pCurrentMonData->blurFBShouldRender;

    g_pHyprOpenGL->m_renderData.pCurrentMonData->blurFBShouldRender = true;
    g_pHyprOpenGL->m_renderData.clipBox = box;

    g_pHyprOpenGL->renderRectWithBlur(box, CHyprColor(0.0, 0.0, 0.0, 1.0));

    for (auto& ow : workspaces) {
        ow.render(box, time);
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
}
