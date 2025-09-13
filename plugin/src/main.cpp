
#include <exception>
#include <poll.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <filesystem>

#include "utils.hpp"

#ifndef VERSION
#define VERSION ""
#endif

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
using origStartAnim = void(*)(CDesktopAnimationManager*, PHLWORKSPACE, CDesktopAnimationManager::eAnimationType, bool, bool);
using origStartAnimMemberFnType = void (CDesktopAnimationManager::*)(PHLWORKSPACE, CDesktopAnimationManager::eAnimationType, bool, bool);
static_assert(
    std::is_same_v<
        origStartAnimMemberFnType,
        decltype(static_cast<origStartAnimMemberFnType>(&CDesktopAnimationManager::startAnimation))
    >,
    "animation hook function signature mismatch"
);

void hk_workspace_anim(CDesktopAnimationManager* thisptr, PHLWORKSPACE ws, CDesktopAnimationManager::eAnimationType type, bool left, bool instant) {
    Hyprutils::Memory::CWeakPointer<Hyprutils::Animation::SAnimationPropertyConfig> conf = ws->m_alpha->getConfig();

    bool did_the_thing = false;

    if (const auto pconfig = conf.lock()) {
        const auto pvalues = pconfig->pValues.lock();
        if (pvalues) {
            std::string style = pvalues->internalStyle;

            switch (anim_dir) {
                case Animation::None: {
                    instant = true;
                } break;
                case Animation::Left: {
                    left = false;
                    pvalues->internalStyle = "slide";
                } break;
                case Animation::Right: {
                    left = true;
                    pvalues->internalStyle = "slide";
                } break;
                case Animation::Up: {
                    left = false;
                    pvalues->internalStyle = "slidevert";
                } break;
                case Animation::Down: {
                    left = true;
                    pvalues->internalStyle = "slidevert";
                } break;
                case Animation::Fade: {
                    pvalues->internalStyle = "fade";
                } break;
                default: {
                    instant = true;
                } break;
            }

            (*(origStartAnim)g_pWorkAnimHook->m_original)(thisptr, ws, type, left, instant);

            pvalues->internalStyle = style;
            did_the_thing = true;
        }
    }

    if (!did_the_thing) {
        (*(origStartAnim)g_pWorkAnimHook->m_original)(thisptr, ws, type, left, instant);
    }
}

void init_hooks() {
    // objdump -t $(which Hyprland) | rg "F .text" | rg startAnimation | rg CDesktopAnimationManager | rg CWorkspace
    static const auto START_ANIM = HyprlandAPI::findFunctionsByName(PHANDLE, "_ZN24CDesktopAnimationManager14startAnimationEN9Hyprutils6Memory14CSharedPointerI10CWorkspaceEENS_14eAnimationTypeEbb");
    g_pWorkAnimHook = HyprlandAPI::createFunctionHook(PHANDLE, START_ANIM[0].address, (void*)&hk_workspace_anim);
    g_pWorkAnimHook->hook();
}

void init_hypr_config() {
    // HyprlandAPI::addConfigValue(PHANDLE, _?_, Hyprlang::INT{?});
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
