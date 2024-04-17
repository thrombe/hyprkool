
#include <cerrno>
#include <cstdio>
#include <exception>
#include <filesystem>
#include <hyprland/src/config/ConfigManager.hpp>
#include <hyprland/src/debug/Log.hpp>
#include <hyprland/src/desktop/Workspace.hpp>
#include <hyprland/src/helpers/AnimatedVariable.hpp>
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <netinet/in.h>
#include <poll.h>
#include <pthread.h>
#include <stdexcept>
#include <string>
#include <sys/socket.h>
#include <sys/un.h>
#include <thread>

enum Animation {
    None = 0,
    Left = 1,
    Right = 2,
    Up = 3,
    Down = 4,
    Fade = 5,
};
Animation anim_dir = Animation::None;

inline HANDLE PHANDLE = nullptr;
std::string sock_path;
bool exit_flag = false;
int sockfd = -1;
std::thread sock_thread;

std::string get_socket_path() {
    const auto ISIG = getenv("HYPRLAND_INSTANCE_SIGNATURE");
    if (!ISIG) {
        std::cout << "HYPRLAND_INSTANCE_SIGNATURE not set! (is hyprland running?)\n";
        throw std::runtime_error("[hyprkool] could not get HYPRLAND_INSTANCE_SIGNATURE");
    }
    auto sock_path = "/tmp/hyprkool/" + std::string(ISIG);
    if (!std::filesystem::exists(sock_path)) {
        if (!std::filesystem::create_directories(sock_path)) {
            std::cout << "could not create directory";
            throw std::runtime_error("could not create directory");
        }
    }
    sock_path += "/plugin.sock";
    return sock_path;
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
                Animation d = static_cast<Animation>(std::stoi(line));
                anim_dir = d;
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
        std::cerr << "Error creating socket" << std::endl;
        throw std::runtime_error("[hyprkool] Error creating socket");
    }

    if (std::filesystem::exists(sock_path)) {
        auto _ = std::filesystem::remove(sock_path);
    }

    // Bind the socket to a file path (socket file)
    struct sockaddr_un addr;
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, sock_path.c_str(), sizeof(addr.sun_path) - 1);
    if (bind(sockfd, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        std::cerr << "Error binding socket" << std::endl;
        close(sockfd);
        throw std::runtime_error("[hyprkool] Error binding socket");
    }

    pollfd fd;
    fd.fd = sockfd;
    fd.events = POLLIN;
    while (!exit_flag) {
        int ret = poll(&fd, 1, 50);
        if (ret < 0) {
            std::cerr << "Error polling on socket" << std::endl;
            throw std::runtime_error("[hyprkool] Error polling on socket");
        } else if (ret == 0) {
            // timeout
            continue;
        }

        if (listen(sockfd, 5) < 0) {
            std::cerr << "Error listening on socket" << std::endl;
            close(sockfd);
            throw std::runtime_error("[hyprkool] Error listening on socket");
        }

        int clientfd = accept(sockfd, NULL, NULL);
        if (clientfd < 0) {
            std::cerr << "Error accepting connection" << std::endl;
            std::cerr << strerror(errno) << std::endl;
            close(sockfd);
            throw std::runtime_error("[hyprkool] Error accepting connection");
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

void init_hooks() {
    static const auto METHODS = HyprlandAPI::findFunctionsByName(PHANDLE, "startAnim");
    g_pWorkAnimHook = HyprlandAPI::createFunctionHook(PHANDLE, METHODS[0].address, (void*)&hk_workspace_anim);
    g_pWorkAnimHook->hook();
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
        HyprlandAPI::addNotification(PHANDLE, "[hyprkool] Mismatched headers! Can't proceed.",
                                     CColor{1.0, 0.2, 0.2, 1.0}, 5000);
        throw std::runtime_error("[hyprkool] Version mismatch");
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
