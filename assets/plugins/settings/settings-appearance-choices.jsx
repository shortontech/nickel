// @jsx h
// Appearance mode and accent choices. Persistence and color policy stay in the host.
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
        <settings-transparency id="appearance-transparency" label={data.transparencyTitle}
            value={data.transparencyDescription} selected={data.reduceTransparency}
            onClick={() => nickel.request({type: 'reduce-transparency', value: !data.reduceTransparency})} />
    </settings-appearance-choices>;
}
