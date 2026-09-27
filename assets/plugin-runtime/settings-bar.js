// @jsx h
// Ordinary Bar settings. The host owns persistence and shell topology checks.
function App() {
    const data = nickel.data;
    return h("settings-bar", null,
        h("settings-text", { label: data.showOn }),
        h("settings-radio-group", { id: "bar-display-scope" },
            h("settings-radio", { id: "bar-primary-display", label: data.primaryDisplay, selected: !data.barOnAllDisplays, onClick: () => nickel.request({ type: 'display-scope', scope: 'primary' }) }),
            h("settings-radio", { id: "bar-all-displays", label: data.allDisplays, selected: data.barOnAllDisplays, onClick: () => nickel.request({ type: 'display-scope', scope: 'all' }) })),
        h("settings-text", { label: data.windowScope }),
        h("settings-radio-group", { id: "bar-window-scope" },
            h("settings-radio", { id: "bar-display-windows", label: data.thisDisplay, selected: !data.allWindowsOnEveryBar, onClick: () => nickel.request({ type: 'window-scope', scope: 'display' }) }),
            h("settings-radio", { id: "bar-all-windows", label: data.allWindows, selected: data.allWindowsOnEveryBar, onClick: () => nickel.request({ type: 'window-scope', scope: 'all' }) })),
        h("settings-slider", { id: "bar-desktop-count", label: data.desktopsLabel, value: data.desktopCountLabel, percent: (data.desktopCount - 1) / (data.maxDesktops - 1), onChange: fraction => nickel.request({ type: 'desktop-count', fraction }) },
            h("settings-description", { label: "The number of persistent workspaces available to the session." })),
        h("settings-desktops", null, Array.from({ length: data.desktopCount }, (_, index) => h("settings-desktop", { key: index, count: index + 1, selected: index === data.activeDesktop }))));
}
