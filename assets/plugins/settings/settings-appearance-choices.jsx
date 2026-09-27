// @jsx h
// Appearance controls. Persistence, platform lookup, and color policy stay in the host.
function App() {
    const data = nickel.data;
    return <settings-appearance-choices>
        <settings-appearance-modes label={data.title} value={data.description}>
            <settings-choice id="appearance-mode-light" label={data.light}
                selected={data.selected === 'light'}
                onClick={() => nickel.request({type: 'mode', value: 'light'})} />
            <settings-choice id="appearance-mode-dark" label={data.dark}
                selected={data.selected === 'dark'}
                onClick={() => nickel.request({type: 'mode', value: 'dark'})} />
            <settings-choice id="appearance-mode-system" label={data.automatic}
                selected={data.selected === 'system'}
                onClick={() => nickel.request({type: 'mode', value: 'system'})} />
        </settings-appearance-modes>
        <settings-accent-choices label={data.accentTitle} value={data.accentDescription}>
            {data.swatches.map(swatch => <settings-swatch key={swatch.hue}
                id={`appearance-accent-${swatch.hue}`} hue={swatch.hue}
                selected={swatch.selected}
                onClick={() => nickel.request({type: 'accent-hue', hue: swatch.hue})} />)}
            <settings-swatch id="appearance-accent-custom" hue={data.hue} custom={true}
                onClick={() => nickel.request({type: 'open-custom-hue'})} />
        </settings-accent-choices>
        <settings-hue-dialog id="appearance-custom-hue-dialog" label={data.customHueTitle}
            value={data.customHueDescription} open={data.customHueOpen}>
            <settings-input id="appearance-custom-hue-input" label={data.customHueField}
                placeholder={data.customHuePlaceholder} value={data.customHueDraft}
                onChange={value => nickel.request({type: 'custom-hue-draft', value})} />
            <settings-button id="appearance-custom-hue-apply" label={data.customHueApply}
                value="primary" onClick={() => nickel.request({type: 'apply-custom-hue', value: data.customHueDraft})} />
            <settings-button id="appearance-custom-hue-cancel" label={data.customHueCancel}
                value="secondary" onClick={() => nickel.request({type: 'cancel-custom-hue'})} />
        </settings-hue-dialog>
        <settings-interface id="appearance-interface-card" label={data.interfaceTitle}>
            <settings-slider id="appearance-hue" label={data.hueTitle}
                value={data.hueValue} placeholder={data.hueDescription}
                percent={data.hue / 359}
                onChange={fraction => nickel.request({type: 'appearance-hue', fraction})} />
            <settings-slider id="appearance-intensity" label={data.intensityTitle}
                value={data.intensityValue} placeholder={data.intensityDescription}
                percent={data.intensity / 100}
                onChange={fraction => nickel.request({type: 'appearance-intensity', fraction})} />
            <settings-transparency id="appearance-transparency" label={data.transparencyTitle}
                value={data.transparencyDescription} selected={data.reduceTransparency}
                onClick={() => nickel.request({type: 'reduce-transparency', value: !data.reduceTransparency})} />
            <settings-select id="appearance-animations" label={data.animationTitle}
                placeholder={data.animationDescription} value={data.animationValue}
                open={data.animationExpanded}
                onClick={() => nickel.request({type: 'toggle-animation-select'})}>
                {data.animations.map(option => <settings-option key={option.id}
                    id={`appearance-animation-${option.id}`} label={option.label}
                    selected={option.id === data.animationId}
                    onClick={() => nickel.request({type: 'animation', value: option.id})} />)}
            </settings-select>
            <settings-select id="appearance-file-artwork" label={data.fileArtworkTitle}
                placeholder={data.fileArtworkDescription} value={data.fileArtworkValue}
                open={data.fileArtworkExpanded}
                onClick={() => nickel.request({type: 'toggle-file-artwork-select'})}>
                {data.fileArtworkOptions.map((option, index) => <settings-option key={option.id}
                    id={`appearance-file-artwork-option-${index}`} label={option.label}
                    selected={option.id === data.fileArtworkId}
                    onClick={() => nickel.request({type: 'file-artwork', value: option.id})} />)}
            </settings-select>
        </settings-interface>
    </settings-appearance-choices>;
}
