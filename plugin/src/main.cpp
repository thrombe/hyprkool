
#include <hyprland/src/config/ConfigManager.hpp>
#include <hyprland/src/debug/Log.hpp>
#include <hyprland/src/desktop/Workspace.hpp>
#include <hyprland/src/helpers/AnimatedVariable.hpp>
#include <hyprland/src/plugins/PluginAPI.hpp>
#include <string>

inline HANDLE PHANDLE = nullptr;

inline CFunctionHook* g_pWorkAnimHook = nullptr;
typedef void (*origStartAnim)(CWorkspace*, bool, bool, bool);
void hk_workspace_anim(CWorkspace* thisptr, bool in, bool left, bool instant) {
    SAnimationPropertyConfig* conf = (thisptr->m_fAlpha.getConfig());
    std::string style = conf->pValues->internalStyle;
    conf->pValues->internalStyle = "slidevert";

    (*(origStartAnim)g_pWorkAnimHook->m_pOriginal)(thisptr, in, left, instant);

    conf->pValues->internalStyle = style;
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
        HyprlandAPI::addNotification(
            PHANDLE, "[hyprkool] Mismatched headers! Can't proceed.",
            CColor{1.0, 0.2, 0.2, 1.0}, 5000);
        throw std::runtime_error("[hyprkool] Version mismatch");
    }

    // ...
    static const auto METHODS =
        HyprlandAPI::findFunctionsByName(PHANDLE, "startAnim");
    g_pWorkAnimHook = HyprlandAPI::createFunctionHook(
        handle, METHODS[0].address, (void*)&hk_workspace_anim);
    g_pWorkAnimHook->hook();

    return {"hyprkool", "hyprkool yea", "thrombe", "0.0.1"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    // ...
}
