// @jsx h
// Appearance controls. Persistence, platform lookup, and color policy stay in the host.
function App() {
    const data = nickel.data;
    return h("settings-appearance-page", null,
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
        h("settings-wallpaper", { id: "appearance-wallpaper-card", label: data.wallpaperTitle, value: data.wallpaperDescription },
            h("settings-wallpaper-preview", { id: "appearance-wallpaper-preview", label: data.wallpaperName, value: data.wallpaperDimensions, placeholder: data.wallpaperNone, state: data.wallpaperStatus }),
            h("settings-button", { id: "appearance-wallpaper-choose", label: data.wallpaperChoose, value: "primary", onClick: () => nickel.request({ type: 'wallpaper-choose' }) }),
            h("settings-button", { id: "appearance-wallpaper-remove", label: data.wallpaperRemove, value: "secondary", onClick: () => nickel.request({ type: 'wallpaper-remove' }) }),
            h("settings-select", { id: "appearance-wallpaper-position", label: data.wallpaperFitTitle, placeholder: data.wallpaperFitDescription, value: data.wallpaperPositionValue, open: data.wallpaperPositionExpanded, onClick: () => nickel.request({ type: 'toggle-wallpaper-position' }) }, data.wallpaperPositions.map(option => h("settings-option", { key: option.id, id: `appearance-wallpaper-position-${option.id}`, label: option.label, selected: option.id === data.wallpaperPositionId, onClick: () => nickel.request({ type: 'wallpaper-position', value: option.id }) })))),
        h("settings-interface", { id: "appearance-interface-card", label: data.interfaceTitle },
            h("settings-slider", { id: "appearance-hue", label: data.hueTitle, value: data.hueValue, placeholder: data.hueDescription, percent: data.hue / 359, onChange: fraction => nickel.request({ type: 'appearance-hue', fraction }) }),
            h("settings-slider", { id: "appearance-intensity", label: data.intensityTitle, value: data.intensityValue, placeholder: data.intensityDescription, percent: data.intensity / 100, onChange: fraction => nickel.request({ type: 'appearance-intensity', fraction }) }),
            h("settings-transparency", { id: "appearance-transparency", label: data.transparencyTitle, value: data.transparencyDescription, selected: data.reduceTransparency, onClick: () => nickel.request({ type: 'reduce-transparency', value: !data.reduceTransparency }) }),
            h("settings-select", { id: "appearance-animations", label: data.animationTitle, placeholder: data.animationDescription, value: data.animationValue, open: data.animationExpanded, onClick: () => nickel.request({ type: 'toggle-animation-select' }) }, data.animations.map(option => h("settings-option", { key: option.id, id: `appearance-animation-${option.id}`, label: option.label, selected: option.id === data.animationId, onClick: () => nickel.request({ type: 'animation', value: option.id }) }))),
            h("settings-select", { id: "appearance-file-artwork", label: data.fileArtworkTitle, placeholder: data.fileArtworkDescription, value: data.fileArtworkValue, open: data.fileArtworkExpanded, onClick: () => nickel.request({ type: 'toggle-file-artwork-select' }) }, data.fileArtworkOptions.map((option, index) => h("settings-option", { key: option.id, id: `appearance-file-artwork-option-${index}`, label: option.label, selected: option.id === data.fileArtworkId, onClick: () => nickel.request({ type: 'file-artwork', value: option.id }) })))),
        h("settings-reset", { id: "appearance-reset", label: data.resetLabel, onClick: () => nickel.request({ type: 'appearance-reset' }) }));
}
