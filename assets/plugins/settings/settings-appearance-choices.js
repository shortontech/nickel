// @jsx h
// Appearance mode and accent choices. Persistence and color policy stay in the host.
function App() {
    const data = nickel.data;
    return h("settings-appearance-choices", null,
        h("settings-appearance-modes", { label: data.title, value: data.description },
            h("settings-choice", { id: "appearance-mode-light", label: data.light, selected: data.selected === 'light', onClick: () => nickel.request({ type: 'mode', value: 'light' }) }),
            h("settings-choice", { id: "appearance-mode-dark", label: data.dark, selected: data.selected === 'dark', onClick: () => nickel.request({ type: 'mode', value: 'dark' }) }),
            h("settings-choice", { id: "appearance-mode-system", label: data.automatic, selected: data.selected === 'system', onClick: () => nickel.request({ type: 'mode', value: 'system' }) })),
        h("settings-accent-choices", { label: data.accentTitle, value: data.accentDescription },
            data.swatches.map(swatch => h("settings-swatch", { key: swatch.hue, id: `appearance-accent-${swatch.hue}`, hue: swatch.hue, selected: swatch.selected, onClick: () => nickel.request({ type: 'accent-hue', hue: swatch.hue }) })),
            h("settings-swatch", { id: "appearance-accent-custom", hue: data.hue, custom: true, onClick: () => nickel.request({ type: 'open-custom-hue' }) })),
        h("settings-hue-dialog", { id: "appearance-custom-hue-dialog", label: data.customHueTitle, value: data.customHueDescription, open: data.customHueOpen },
            h("settings-input", { id: "appearance-custom-hue-input", label: data.customHueField, placeholder: data.customHuePlaceholder, value: data.customHueDraft, onChange: value => nickel.request({ type: 'custom-hue-draft', value }) }),
            h("settings-button", { id: "appearance-custom-hue-apply", label: data.customHueApply, value: "primary", onClick: () => nickel.request({ type: 'apply-custom-hue', value: data.customHueDraft }) }),
            h("settings-button", { id: "appearance-custom-hue-cancel", label: data.customHueCancel, value: "secondary", onClick: () => nickel.request({ type: 'cancel-custom-hue' }) })),
        h("settings-transparency", { id: "appearance-transparency", label: data.transparencyTitle, value: data.transparencyDescription, selected: data.reduceTransparency, onClick: () => nickel.request({ type: 'reduce-transparency', value: !data.reduceTransparency }) }));
}
