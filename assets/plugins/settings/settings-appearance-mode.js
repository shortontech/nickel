// @jsx h
// Appearance mode choices. Persistence and system appearance stay in the host.
function App() {
    const data = nickel.data;
    return h("settings-appearance-modes", { label: data.title, value: data.description },
        h("settings-choice", { id: "appearance-mode-light", label: data.light, selected: data.selected === 'light', onClick: () => nickel.request({ type: 'mode', value: 'light' }) }),
        h("settings-choice", { id: "appearance-mode-dark", label: data.dark, selected: data.selected === 'dark', onClick: () => nickel.request({ type: 'mode', value: 'dark' }) }),
        h("settings-choice", { id: "appearance-mode-system", label: data.automatic, selected: data.selected === 'system', onClick: () => nickel.request({ type: 'mode', value: 'system' }) }));
}
