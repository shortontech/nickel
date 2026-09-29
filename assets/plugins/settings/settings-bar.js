// @jsx h
// Workspace and panel preferences rendered from ordinary Nickel components.
function App() {
    const data = nickel.data;
    return h("div", { className: "bar-card" },
        h(Text, { className: "bar-heading" }, data.showOn),
        h("div", { className: "bar-options" },
            h(Button, { id: "bar-primary-display", className: data.barOnAllDisplays ? 'bar-option' : 'bar-option selected', accessibilityLabel: data.primaryDisplay, onClick: () => nickel.request({ type: 'display-scope', scope: 'primary' }) }, `${data.barOnAllDisplays ? '○' : '◉'}  ${data.primaryDisplay}`),
            h(Button, { id: "bar-all-displays", className: data.barOnAllDisplays ? 'bar-option selected' : 'bar-option', accessibilityLabel: data.allDisplays, onClick: () => nickel.request({ type: 'display-scope', scope: 'all' }) }, `${data.barOnAllDisplays ? '◉' : '○'}  ${data.allDisplays}`)),
        h(Text, { className: "bar-heading" }, data.windowScope),
        h("div", { className: "bar-options" },
            h(Button, { id: "bar-display-windows", className: data.allWindowsOnEveryBar ? 'bar-option' : 'bar-option selected', accessibilityLabel: data.thisDisplay, onClick: () => nickel.request({ type: 'window-scope', scope: 'display' }) }, `${data.allWindowsOnEveryBar ? '○' : '◉'}  ${data.thisDisplay}`),
            h(Button, { id: "bar-all-windows", className: data.allWindowsOnEveryBar ? 'bar-option selected' : 'bar-option', accessibilityLabel: data.allWindows, onClick: () => nickel.request({ type: 'window-scope', scope: 'all' }) }, `${data.allWindowsOnEveryBar ? '◉' : '○'}  ${data.allWindows}`)),
        h("div", { className: "bar-slider-section" },
            h("div", { className: "bar-slider-heading" },
                h(Text, { className: "bar-heading" }, data.desktopsLabel),
                h(Text, { className: "bar-count" }, data.desktopCountLabel)),
            h(Slider, { id: "bar-desktop-count", className: "bar-slider", accessibilityLabel: data.desktopsLabel, value: (data.desktopCount - 1) / (data.maxDesktops - 1), onChange: fraction => nickel.request({ type: 'desktop-count', fraction }) }),
            h(Text, { className: "bar-description", wrap: true }, "The number of persistent workspaces available to the session.")),
        h("div", { className: "bar-desktops" }, Array.from({ length: data.desktopCount }, (_, index) => h("div", { key: index, className: index === data.activeDesktop ? 'bar-desktop selected' : 'bar-desktop' },
            h(Text, { className: "bar-desktop-number" }, index + 1)))));
}
