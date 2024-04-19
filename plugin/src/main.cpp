
#include <any>
#include <cerrno>
#include <cstdio>
#include <ctime>
#include <exception>
#include <filesystem>
#include <hyprland/src/Compositor.hpp>
#include <hyprland/src/SharedDefs.hpp>
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
    ToggleOverview = 6,
};
Animation anim_dir = Animation::None;

inline HANDLE PHANDLE = nullptr;
std::string sock_path;
bool exit_flag = false;
int sockfd = -1;
std::thread sock_thread;

// I DON'T KNOW HOW TO DO CPP ERROR HANDLING
void throw_err_notif(std::string msg) {
    msg = "[hyprkool] " + msg;
    std::cerr << msg << std::endl;
    HyprlandAPI::addNotification(PHANDLE, msg,
                                 CColor{1.0, 0.2, 0.2, 1.0}, 5000);
    throw std::runtime_error(msg);
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
        case PluginEvent::ToggleOverview: {
            overview_enabled = !overview_enabled;
            auto& m = g_pCompositor->m_vMonitors[0];
            g_pCompositor->scheduleFrameForMonitor(m.get());
        } break;
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
};

class GridOverview {
  public:
    std::string activity;
    std::vector<OverviewWorkspace> workspaces;
    CBox box;

    GridOverview() {
        auto& m = g_pCompositor->m_vMonitors[0];
        auto& w = m->activeWorkspace;
        std::regex pattern("([a-zA-Z0-9]+):\\(([0-9]+) ([0-9]+)\\)");

        if (std::regex_match(w->m_szName, pattern)) {
            std::getline(std::istringstream(w->m_szName), activity, ':');
        } else {
            throw_err_notif("can't display overview when not in a hyprkool activity");
        }
        box.x = m->vecPosition.x;
        box.y = m->vecPosition.y;
        box.w = m->vecSize.x;
        box.h = m->vecSize.y;

        float scale = 1.0 / 3.0;

        for (int y = 0; y < 3; y++) {
            for (int x = 0; x < 3; x++) {
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

        g_pHyprOpenGL->m_RenderData.pCurrentMonData->blurFBShouldRender = br;
        g_pHyprOpenGL->m_RenderData.clipBox = CBox();
        g_pHyprOpenGL->m_RenderData.renderModif.enabled = o_modif;
    }
};

void on_render(void* thisptr, SCallbackInfo& info, std::any args) {
    if (!overview_enabled) {
        return;
    }
    const auto render_stage = std::any_cast<eRenderStage>(args);
    GridOverview go;
    try {
        go = GridOverview();
    } catch (const std::exception& e) {
        std::cerr << e.what() << std::endl;
        overview_enabled = false;
        return;
    }

    switch (render_stage) {
        case eRenderStage::RENDER_PRE: {
        } break;
        case eRenderStage::RENDER_PRE_WINDOWS: {
        } break;
        case eRenderStage::RENDER_POST_WINDOWS: {
            // CBox box = CBox(50, 50, 100.0, 100.0);
            // g_pHyprOpenGL->renderRectWithBlur(&box, CColor(0.3, 0.0, 0.0, 0.3));
            overview_enabled = false;
            go.render();
            overview_enabled = true;
            // TODO: damaging entire window fixes the weird areas - but is inefficient
            g_pHyprRenderer->damageBox(&go.box);
        } break;
        case eRenderStage::RENDER_LAST_MOMENT: {
        } break;
        case eRenderStage::RENDER_POST: {
        } break;
        default: {
        } break;
    }
}

void init_hooks() {
    static const auto START_ANIM = HyprlandAPI::findFunctionsByName(PHANDLE, "startAnim");
    g_pWorkAnimHook = HyprlandAPI::createFunctionHook(PHANDLE, START_ANIM[0].address, (void*)&hk_workspace_anim);
    g_pWorkAnimHook->hook();

    static const auto RENDER_LAYER = HyprlandAPI::findFunctionsByName(PHANDLE, "renderLayer");
    g_pRenderLayer = HyprlandAPI::createFunctionHook(PHANDLE, RENDER_LAYER[0].address, (void*)&hk_render_layer);
    g_pRenderLayer->hook();

    HyprlandAPI::registerCallbackDynamic(PHANDLE, "render", on_render);

    auto funcSearch = HyprlandAPI::findFunctionsByName(PHANDLE, "renderWindow");
    renderWindow = funcSearch[0].address;

    funcSearch = HyprlandAPI::findFunctionsByName(PHANDLE, "renderLayer");
    renderLayer = funcSearch[0].address;
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
        throw_err_notif("Mismatched headers! Can't proceed.");
    }

    sock_path = get_socket_path();

    init_hooks();
    sock_thread = std::thread(socket_serve);

    return {"hyprkool", "hyprkool yea", "thrombe", "0.0.1"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    exit_flag = true;
    sock_thread.join();
}
