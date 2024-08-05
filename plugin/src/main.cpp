
#include <poll.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <hyprland/src/devices/IPointer.hpp>
#include <filesystem>

#include "overview.hpp"
#include "utils.hpp"

#ifndef VERSION
#define VERSION ""
#endif

bool overview_enabled = false;
void handle_plugin_event(PluginEvent e) {
    switch (e) {
        default: {
            anim_dir = static_cast<Animation>(e);
        } break;
    }
}

void sendstr(int sockfd, const char* buf) {
    ssize_t len = strlen(buf);
    ssize_t total_sent = 0;
    while (total_sent < len) {
        ssize_t sent = send(sockfd, buf + total_sent, len - total_sent, 0);
        if (sent == -1) {
            throw_err_notif("Could not send all bytes across socket");
            return;
        }
        total_sent += sent;
    }
    return;
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
                sendstr(clientfd, "\"IpcOk\"\n");
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
typedef void (*origRenderLayer)(void*, CLayerSurface*, CMonitor*, timespec*, bool);
void hk_render_layer(void* thisptr, CLayerSurface* layer, CMonitor* monitor, timespec* time, bool popups) {
    if (!overview_enabled) {
        (*(origRenderLayer)(g_pRenderLayer->m_pOriginal))(thisptr, layer, monitor, time, popups);
    }
}

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
    auto const ws = std::any_cast<CSharedPointer<CWorkspace>>(args);
    if (ws->m_szName.ends_with(":overview")) {
        g_go = {};
        g_go.init();
        overview_enabled = true;
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
    auto const w = std::any_cast<PHLWINDOW>(args);
    if (!w) {
        return;
    }
    if (overview_enabled) {
        auto m = g_pCompositor->getMonitorFromCursor();
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
void safe_on_window(void* thisptr, SCallbackInfo& info, std::any args) {
    try {
        on_window(thisptr, info, args);
    } catch (const std::exception& e) {
        err_notif(e.what());
        overview_enabled = false;
    }
}

void on_mouse_button(void* thisptr, SCallbackInfo& info, std::any args) {
    if (!overview_enabled) {
        return;
    }
    const auto e = std::any_cast<IPointer::SButtonEvent>(args);
    if (e.button != BTN_LEFT) {
        return;
    }
    auto pos = g_pInputManager->getMouseCoordsInternal();
    for (auto& w : g_pCompositor->m_vWindows) {
        auto wbox = w->getFullWindowBoundingBox();
        for (auto& ow : g_go.workspaces) {
            if (!w->m_pWorkspace) {
                continue;
            }
            if (w->m_pWorkspace->m_szName.starts_with(ow.name)) {
                wbox.scale(ow.scale);
                wbox.translate(ow.box.pos());
                wbox.round();
                if (wbox.containsPoint(pos)) {
                    // Hyprland/src/desktop/Window.hpp:467
                    HyprlandAPI::invokeHyprctlCommand("dispatch", std::string("focuswindow address:") +
                                                                      std::format("0x{:x}", (uintptr_t)w.get()));
                    return;
                }
            }
        }
    }
    for (auto& ow : g_go.workspaces) {
        if (ow.box.containsPoint(pos)) {
            HyprlandAPI::invokeHyprctlCommand("dispatch", "workspace name:" + ow.name);
            return;
        }
    }
}
void safe_on_mouse_button(void* thisptr, SCallbackInfo& info, std::any args) {
    try {
        on_mouse_button(thisptr, info, args);
    } catch (const std::exception& e) {
        err_notif(e.what());
        overview_enabled = false;
    }
}

CSharedPointer<HOOK_CALLBACK_FN> render_callback;
CSharedPointer<HOOK_CALLBACK_FN> workspace_callback;
CSharedPointer<HOOK_CALLBACK_FN> activewindow_callback;
CSharedPointer<HOOK_CALLBACK_FN> mousebutton_callback;
void init_hooks() {
    static const auto START_ANIM = HyprlandAPI::findFunctionsByName(PHANDLE, "startAnim");
    g_pWorkAnimHook = HyprlandAPI::createFunctionHook(PHANDLE, START_ANIM[0].address, (void*)&hk_workspace_anim);
    g_pWorkAnimHook->hook();

    static const auto RENDER_LAYER = HyprlandAPI::findFunctionsByName(PHANDLE, "renderLayer");
    g_pRenderLayer = HyprlandAPI::createFunctionHook(PHANDLE, RENDER_LAYER[0].address, (void*)&hk_render_layer);
    g_pRenderLayer->hook();

    render_callback = HyprlandAPI::registerCallbackDynamic(PHANDLE, "render", safe_on_render);
    workspace_callback = HyprlandAPI::registerCallbackDynamic(PHANDLE, "workspace", safe_on_workspace);
    activewindow_callback = HyprlandAPI::registerCallbackDynamic(PHANDLE, "activeWindow", safe_on_window);
    mousebutton_callback = HyprlandAPI::registerCallbackDynamic(PHANDLE, "mouseButton", safe_on_mouse_button);

    auto funcSearch = HyprlandAPI::findFunctionsByName(PHANDLE, "renderWindow");
    renderWindow = funcSearch[0].address;

    funcSearch = HyprlandAPI::findFunctionsByName(PHANDLE, "renderLayer");
    renderLayer = funcSearch[0].address;
}

void init_hypr_config() {
    HyprlandAPI::addConfigValue(PHANDLE, HOVER_BORDER_CONFIG_NAME, Hyprlang::INT{0xee33ccff});
    HyprlandAPI::addConfigValue(PHANDLE, FOCUS_BORDER_CONFIG_NAME, Hyprlang::INT{0xee00ff99});
    HyprlandAPI::addConfigValue(PHANDLE, GAP_SIZE_CONFIG_NAME, Hyprlang::INT{10});
}

// Do NOT change this function.
APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

// TODO: check and make sure that hyprkool cli is capatable before starting the plugin
// TODO: when plugin starts, send an internal command to hyprkool daemon. and replace that running
//    process with a newer instance of daemon if there is a version change.
//    (ig have both the daemon and plugin contain commit hash)
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

    return {"hyprkool", "Grid workspaces for hyprland", "thrombe", VERSION};
}

APICALL EXPORT void PLUGIN_EXIT() {
    exit_flag = true;
    sock_thread.join();
}
