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
                onClick={() => nickel.request({type: 'accent-hue', hue: data.hue})} />
        </settings-accent-choices>
    </settings-appearance-choices>;
}
