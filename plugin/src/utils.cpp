#include <toml++/toml.hpp>
#include <filesystem>

#include "utils.hpp"

void* renderWindow;
void* renderLayer;

Animation anim_dir = Animation::None;

inline HANDLE PHANDLE = nullptr;
std::string sock_path;
bool exit_flag = false;
int sockfd = -1;
std::thread sock_thread;

// - [How to capture stdin, stdout and stderr of child program! | Now or Never](https://jineshkj.wordpress.com/2006/12/22/how-to-capture-stdin-stdout-and-stderr-of-child-program/)
// - [mod_gearman/common/popenRWE.c at master · sni/mod_gearman · GitHub](https://github.com/sni/mod_gearman/blob/master/common/popenRWE.c)
// hyprland uses the exact same code twice :skull
// std::string exec(std::string cmd) {
//     cmd += " 2>&1";
//     std::array<char, 128> buffer;
//     std::string result;
//     std::unique_ptr<FILE, decltype(&pclose)> pipe(popen(cmd.c_str(), "r"), pclose);
//     if (!pipe) {
//         throw std::runtime_error("popen() failed!");
//     }
//     while (fgets(buffer.data(), static_cast<int>(buffer.size()), pipe.get()) != nullptr) {
//         result += buffer.data();
//     }
//     return result;
// }

// void _set_config() {
//     auto out = exec("/home/issac/0Git/hyprkool/target/debug/hyprkool internal get-workspace-nums");
//     std::cout << out << std::endl;
//     auto outs = std::istringstream(out);
//     outs >> g_KoolConfig.workspaces_x >> g_KoolConfig.workspaces_y >> g_KoolConfig.workspaces_x;
// }

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

