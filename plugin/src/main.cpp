
#include <any>
#include <cerrno>
#include <cstdio>
#include <ctime>
#include <exception>
#include <filesystem>
#include <hyprland/src/Compositor.hpp>
#include <hyprland/src/SharedDefs.hpp>
#include <hyprland/src/config/ConfigDataValues.hpp>
#include <hyprland/src/config/ConfigManager.hpp>
#include <hyprland/src/debug/Log.hpp>
#include <hyprland/src/desktop/Workspace.hpp>
#include <hyprland/src/helpers/AnimatedVariable.hpp>
#include <hyprland/src/helpers/Box.hpp>
#include <hyprland/src/helpers/Color.hpp>
#include <hyprland/src/helpers/Monitor.hpp>
#include <hyprland/src/helpers/WLClasses.hpp>
#include <hyprland/src/managers/input/InputManager.hpp>
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <hyprland/src/render/OpenGL.hpp>
#include <hyprland/src/render/Renderer.hpp>
#include <hyprlang.hpp>
#include <netinet/in.h>
#include <poll.h>
#include <pthread.h>
#include <regex>
#include <sstream>
#include <stdexcept>
#include <string>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <thread>
#include <toml++/toml.hpp>
#include <unistd.h>

typedef void (*FuncRenderWindow)(void*, CWindow*, CMonitor*, timespec*, bool, eRenderPassMode, bool, bool);
void* renderWindow;
typedef void (*FuncRenderLayer)(void*, SLayerSurface*, CMonitor*, timespec*, bool);
void* renderLayer;

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
Animation anim_dir = Animation::None;

inline HANDLE PHANDLE = nullptr;
std::string sock_path;
bool exit_flag = false;
int sockfd = -1;
std::thread sock_thread;

// I DON'T KNOW HOW TO DO CPP ERROR HANDLING
void err_notif(std::string msg) {
    msg = "[hyprkool] " + msg;
    std::cerr << msg << std::endl;
    HyprlandAPI::addNotification(PHANDLE, msg, CColor{1.0, 0.2, 0.2, 1.0}, 5000);
}
void throw_err_notif(std::string msg) {
    err_notif(msg);
    throw std::runtime_error(msg);
}

struct KoolConfig {
    int workspaces_x;
    int workspaces_y;
};
KoolConfig g_KoolConfig;

void _set_config() {
    const auto HOME = getenv("HOME");
    auto path = std::string(HOME) + "/.config/hypr/hyprkool.toml";
    auto manifest = toml::parse_file(path);
    auto workspaces = manifest["workspaces"].as_array();
    if (!workspaces) {
        g_KoolConfig.workspaces_x = 2;
        g_KoolConfig.workspaces_y = 2;
        return;
    }
    auto ws = *workspaces;
    if (!ws.at(0).is_integer() || !ws.at(1).is_integer()) {
        throw_err_notif("workspaces should be (int int) in hyprkool.toml");
    }
    g_KoolConfig.workspaces_x = ws.at(0).as_integer()->value_or(2);
    g_KoolConfig.workspaces_y = ws.at(1).as_integer()->value_or(2);
}

void set_config() {
    try {
        _set_config();
    } catch (const std::exception& e) {
        throw_err_notif(e.what());
    }
}

std::string get_socket_path() {
    const auto ISIG = getenv("HYPRLAND_INSTANCE_SIGNATURE");
    if (!ISIG) {
        throw_err_notif("HYPRLAND_INSTANCE_SIGNATURE not set! (is hyprland running?)");
    }
    auto sock_path = "/tmp/hyprkool/" + std::string(ISIG);
    if (!std::filesystem::exists(sock_path)) {
        if (!std::filesystem::create_directories(sock_path)) {
            throw_err_notif("could not create directory");
        }
    }
    sock_path += "/plugin.sock";
    return sock_path;
}

bool overview_enabled = false;
void handle_plugin_event(PluginEvent e) {
    switch (e) {
        default: {
            anim_dir = static_cast<Animation>(e);
        } break;
    }
}

void socket_connect(int clientfd) {
    char buffer[1024];
    std::string partial_line;
    while (true) {
        ssize_t bytes_read = read(clientfd, buffer, sizeof(buffer));
        if (bytes_read < 0) {
            std::cerr << "Error reading from socket" << std::endl;
            break;
        } else if (bytes_read == 0) {
            // End of stream (socket closed)
            break;
        }

        std::istringstream iss(partial_line + std::string(buffer, bytes_read));
        std::string line;
        while (std::getline(iss, line)) {
            try {
                auto e = static_cast<PluginEvent>(std::stoi(line));
                handle_plugin_event(e);
            } catch (const std::exception& e) {
                std::cerr << "Error parsing socket data: " << e.what() << std::endl;
                continue;
            }
        }

        partial_line = line;
    }
}

void socket_serve() {
    sockfd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (sockfd < 0) {
        throw_err_notif("Error creating socket");
    }

    if (std::filesystem::exists(sock_path)) {
        auto _ = std::filesystem::remove(sock_path);
    }

    // Bind the socket to a file path (socket file)
    struct sockaddr_un addr;
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, sock_path.c_str(), sizeof(addr.sun_path) - 1);
    if (bind(sockfd, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        close(sockfd);
        throw_err_notif("Error binding socket");
    }
    // first call does not block for some reason. without it poll returns POLLHUP. and i don't know
    // what to do other than this.
    listen(sockfd, 5);

    pollfd fd;
    fd.fd = sockfd;
    fd.events = POLLIN;
    while (!exit_flag) {
        int ret = poll(&fd, 1, 100);
        if (ret < 0) {
            throw_err_notif("Error polling on socket");
        } else if (ret == 0) {
            // timeout
            continue;
        }

        if (listen(sockfd, 5) < 0) {
            close(sockfd);
            throw_err_notif("Error listening on socket");
        }

        int clientfd = accept(sockfd, NULL, NULL);
        if (clientfd < 0) {
            close(sockfd);
            throw_err_notif("Error accepting connection");
        }

        socket_connect(clientfd);
        close(clientfd);
    }
    close(sockfd);
    auto _ = std::filesystem::remove(sock_path);
}

void safe_socket_serve() {
    try {
        socket_serve();
    } catch (const std::exception& e) {
        err_notif(e.what());
        // well. i hope something nice happens.
    }
}

inline CFunctionHook* g_pWorkAnimHook = nullptr;
typedef void (*origStartAnim)(CWorkspace*, bool, bool, bool);
void hk_workspace_anim(CWorkspace* thisptr, bool in, bool left, bool instant) {
    SAnimationPropertyConfig* conf = (thisptr->m_fAlpha.getConfig());
    std::string style = conf->pValues->internalStyle;

    switch (anim_dir) {
        case Animation::None: {
            instant = true;
        } break;
        case Animation::Left: {
            left = false;
            conf->pValues->internalStyle = "slide";
        } break;
        case Animation::Right: {
            left = true;
            conf->pValues->internalStyle = "slide";
        } break;
        case Animation::Up: {
            left = false;
            conf->pValues->internalStyle = "slidevert";
        } break;
        case Animation::Down: {
            left = true;
            conf->pValues->internalStyle = "slidevert";
        } break;
        case Animation::Fade: {
            conf->pValues->internalStyle = "fade";
        } break;
        default: {
            instant = true;
        } break;
    }

    (*(origStartAnim)g_pWorkAnimHook->m_pOriginal)(thisptr, in, left, instant);

    conf->pValues->internalStyle = style;
}

inline CFunctionHook* g_pRenderLayer = nullptr;
typedef void (*origRenderLayer)(void*, SLayerSurface*, CMonitor*, timespec*, bool);
void hk_render_layer(void* thisptr, SLayerSurface* layer, CMonitor* monitor, timespec* time, bool popups) {
    if (!overview_enabled) {
        (*(origRenderLayer)(g_pRenderLayer->m_pOriginal))(thisptr, layer, monitor, time, popups);
    }
}

class OverviewWorkspace {
  public:
    std::string name;
    CBox box;
    float scale;

    void render(CBox screen, timespec* time) {
        render_hyprland_wallpaper(screen);
        render_bg_layers(screen, time);

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
            render_window(w.get(), screen, time);
        }

        render_top_layers(screen, time);
    }

    void render_window(CWindow* w, CBox screen, timespec* time) {
        auto& m = g_pCompositor->m_vMonitors[0];

        auto pos = w->m_vRealPosition.value();
        auto size = w->m_vRealSize.value();
        CBox wbox = CBox(pos.x, pos.y, size.x, size.y);

        auto o_ws = w->m_pWorkspace;

        w->m_pWorkspace = m->activeWorkspace;
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
            {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
            {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});

        // TODO: damaging window like this doesn't work very well :/
        //       maybe set the pos and size before damaging
        // g_pHyprRenderer->damageWindow(w);
        (*(FuncRenderWindow)renderWindow)(g_pHyprRenderer.get(), w, m.get(), time, true, RENDER_PASS_MAIN, false,
                                          false);

        w->m_pWorkspace = o_ws;
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
    }

    void render_layer(SLayerSurface* layer, CBox screen, timespec* time) {
        auto& m = g_pCompositor->m_vMonitors[0];

        g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
            {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
            {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});

        (*(FuncRenderLayer)renderLayer)(g_pHyprRenderer.get(), layer, m.get(), time, false);

        g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
    }

    void render_hyprland_wallpaper(CBox screen) {
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
            {SRenderModifData::eRenderModifType::RMOD_TYPE_TRANSLATE, box.pos()});
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.push_back(
            {SRenderModifData::eRenderModifType::RMOD_TYPE_SCALE, scale});

        g_pHyprOpenGL->clearWithTex();

        g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
        g_pHyprOpenGL->m_RenderData.renderModif.modifs.pop_back();
    }

    void render_bg_layers(CBox screen, timespec* time) {
        auto& m = g_pCompositor->m_vMonitors[0];
        for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BACKGROUND]) {
            render_layer(layer.get(), screen, time);
        }
        for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_BOTTOM]) {
            render_layer(layer.get(), screen, time);
        }
    }

    void render_top_layers(CBox screen, timespec* time) {
        auto& m = g_pCompositor->m_vMonitors[0];
        for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_TOP]) {
            render_layer(layer.get(), screen, time);
        }
        // for (auto& layer : m->m_aLayerSurfaceLayers[ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY]) {
        //     render_layer(layer.get(), screen, time);
        // }
    }

    void render_border(CColor col) {
        float bsize = 2.0;
        CBox bbox = box;
        bbox.scale(scale);
        bbox.w -= 2.0 * bsize;
        bbox.h -= 2.0 * bsize;
        bbox.x += bsize;
        bbox.y += bsize;
        CGradientValueData grad = {col};

        g_pHyprOpenGL->renderBorder(&bbox, grad, 0, bsize);
    }
};

std::regex overview_pattern("([a-zA-Z0-9-_]+):\\(([0-9]+) ([0-9]+)\\):overview");
class GridOverview {
  public:
    std::string activity;
    std::vector<OverviewWorkspace> workspaces;
    CBox box;
    CColor cursor_ws_border;
    CColor focus_border;
    int border_size;

    void init() {
        static auto* const* CURSOR_WS_BORDER =
            (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, "plugin:hyprkool:overview:cursor_ws_border")
                ->getDataStaticPtr();
        static auto* const* FOCUS_BORDER =
            (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, "plugin:hyprkool:overview:focus_border")
                ->getDataStaticPtr();
        static auto* const* BORDER_SIZE =
            (Hyprlang::INT* const*)HyprlandAPI::getConfigValue(PHANDLE, "plugin:hyprkool:overview:border_size")
                ->getDataStaticPtr();
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

    void render() {
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

        for (auto& ow : workspaces) {
            ow.render(box, &time);
        }

        auto& m = g_pCompositor->m_vMonitors[0];
        auto& w = m->activeWorkspace;
        auto mouse = g_pInputManager->getMouseCoordsInternal();
        mouse.x *= g_KoolConfig.workspaces_x;
        mouse.y *= g_KoolConfig.workspaces_y;
        for (auto& ow : workspaces) {
            if (w->m_szName.starts_with(ow.name)) {
                ow.render_border(cursor_ws_border);
            }
            if (ow.box.containsPoint(mouse)) {
                ow.render_border(focus_border);
            }
        }

        g_pHyprOpenGL->m_RenderData.pCurrentMonData->blurFBShouldRender = br;
        g_pHyprOpenGL->m_RenderData.clipBox = CBox();
        g_pHyprOpenGL->m_RenderData.renderModif.enabled = o_modif;
    }
};
GridOverview g_go;

void on_render(void* thisptr, SCallbackInfo& info, std::any args) {
    if (!overview_enabled) {
        return;
    }
    const auto render_stage = std::any_cast<eRenderStage>(args);

    switch (render_stage) {
        case eRenderStage::RENDER_PRE: {
        } break;
        case eRenderStage::RENDER_PRE_WINDOWS: {
            // CBox box = CBox(50, 50, 100.0, 100.0);
            // g_pHyprOpenGL->renderRectWithBlur(&box, CColor(0.3, 0.0, 0.0, 0.3));
            overview_enabled = false;
            g_go.render();
            overview_enabled = true;
            // TODO: damaging entire window fixes the weird areas - but is inefficient
            g_pHyprRenderer->damageBox(&g_go.box);
        } break;
        case eRenderStage::RENDER_POST_WINDOWS: {
        } break;
        case eRenderStage::RENDER_LAST_MOMENT: {
        } break;
        case eRenderStage::RENDER_POST: {
        } break;
        default: {
        } break;
    }
}

void safe_on_render(void* thisptr, SCallbackInfo& info, std::any args) {
    // it should not throw, but better to not crash hyprland.
    try {
        on_render(thisptr, info, args);
    } catch (const std::exception& e) {
        err_notif(std::string("ERROR while rendering overview: ") + e.what());
    }
}

void on_workspace(void* thisptr, SCallbackInfo& info, std::any args) {
    auto const ws = std::any_cast<std::shared_ptr<CWorkspace>>(args);
    if (ws->m_szName.ends_with(":overview")) {
        overview_enabled = true;
        g_go = {};
        g_go.init();
    } else {
        overview_enabled = false;
    }
}

void safe_on_workspace(void* thisptr, SCallbackInfo& info, std::any args) {
    try {
        on_workspace(thisptr, info, args);
    } catch (const std::exception& e) {
        err_notif(e.what());
        overview_enabled = false;
    }
}

void on_window(void* thisptr, SCallbackInfo& info, std::any args) {
    auto* const w = std::any_cast<CWindow*>(args);
    if (!w) {
        return;
    }
    if (overview_enabled) {
        auto& m = g_pCompositor->m_vMonitors[0];
        auto& w = m->activeWorkspace;
        if (std::regex_match(w->m_szName, overview_pattern)) {
            auto ss = std::istringstream(w->m_szName);
            std::string activity;
            std::string pos;
            std::getline(ss, activity, ':');
            std::getline(ss, pos, ':');
            HyprlandAPI::invokeHyprctlCommand("dispatch", "movetoworkspace name:" + activity + ":" + pos);
        }
        overview_enabled = false;
    }
}

void on_mouse_button(void* thisptr, SCallbackInfo& info, std::any args) {
    if (!overview_enabled) {
        return;
    }
    const auto e = std::any_cast<wlr_pointer_button_event*>(args);
    if (!e) {
        return;
    }

    if (e->button != BTN_LEFT) {
        return;
    }
    auto pos = g_pInputManager->getMouseCoordsInternal();
    pos.x *= g_KoolConfig.workspaces_x;
    pos.y *= g_KoolConfig.workspaces_y;
    for (auto& w : g_go.workspaces) {
        if (w.box.containsPoint(pos)) {
            HyprlandAPI::invokeHyprctlCommand("dispatch", "workspace name:" + w.name);
            return;
        }
    }
}

void init_hooks() {
    static const auto START_ANIM = HyprlandAPI::findFunctionsByName(PHANDLE, "startAnim");
    g_pWorkAnimHook = HyprlandAPI::createFunctionHook(PHANDLE, START_ANIM[0].address, (void*)&hk_workspace_anim);
    g_pWorkAnimHook->hook();

    static const auto RENDER_LAYER = HyprlandAPI::findFunctionsByName(PHANDLE, "renderLayer");
    g_pRenderLayer = HyprlandAPI::createFunctionHook(PHANDLE, RENDER_LAYER[0].address, (void*)&hk_render_layer);
    g_pRenderLayer->hook();

    HyprlandAPI::registerCallbackDynamic(PHANDLE, "render", safe_on_render);
    HyprlandAPI::registerCallbackDynamic(PHANDLE, "workspace", safe_on_workspace);
    HyprlandAPI::registerCallbackDynamic(PHANDLE, "activeWindow", on_window);
    HyprlandAPI::registerCallbackDynamic(PHANDLE, "mouseButton", on_mouse_button);

    auto funcSearch = HyprlandAPI::findFunctionsByName(PHANDLE, "renderWindow");
    renderWindow = funcSearch[0].address;

    funcSearch = HyprlandAPI::findFunctionsByName(PHANDLE, "renderLayer");
    renderLayer = funcSearch[0].address;
}

void init_hypr_config() {
    HyprlandAPI::addConfigValue(PHANDLE, "plugin:hyprkool:overview:cursor_ws_border", Hyprlang::INT{0xee33ccff});
    HyprlandAPI::addConfigValue(PHANDLE, "plugin:hyprkool:overview:focus_border", Hyprlang::INT{0xee00ff99});
    HyprlandAPI::addConfigValue(PHANDLE, "plugin:hyprkool:overview:border_size", Hyprlang::INT{2});
}

// Do NOT change this function.
APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    PHANDLE = handle;

    const std::string HASH = __hyprland_api_get_hash();

    // ALWAYS add this to your plugins. It will prevent random crashes coming
    // from mismatched header versions.
    if (HASH != GIT_COMMIT_HASH) {
        // throwing is allowed in init function
        throw_err_notif("Mismatched headers! Can't proceed.");
    }

    sock_path = get_socket_path();

    init_hooks();
    init_hypr_config();
    set_config();
    // NOTE: throwing not allowed in another thread
    sock_thread = std::thread(safe_socket_serve);

    return {"hyprkool", "hyprkool yea", "thrombe", "0.0.1"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    exit_flag = true;
    sock_thread.join();
}
