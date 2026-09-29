// @jsx h
// Appearance layout and controls. The host validates and persists requests.
function App() {
    const data = nickel.data;
    const request = (type, fields = {}) => nickel.request({type, ...fields});
    const modes = [
        {id: 'light', label: data.light},
        {id: 'dark', label: data.dark},
        {id: 'system', label: data.automatic},
    ];
    return <div className="appearance-page">
        <div id="appearance-mode-card" className="appearance-card" role="group" aria-label={data.title}>
            <Text className="appearance-heading">{data.title}</Text>
            <Text className="appearance-detail" wrap={true}>{data.description}</Text>
            <div className="appearance-modes" role="radiogroup" aria-label={data.title}>
                {modes.map(mode => <div key={mode.id} id={`appearance-mode-${mode.id}`}
                    className={data.selected === mode.id ? 'appearance-mode selected' : 'appearance-mode'}
                    role="radio" aria-label={mode.label} aria-checked={data.selected === mode.id}
                    onClick={() => request('mode', {value: mode.id})}>
                    <div className={`appearance-mode-preview ${mode.id}`}>
                        <div className="appearance-preview-sidebar" />
                        <div className="appearance-preview-content">
                            <div className="appearance-preview-line" />
                            <div className="appearance-preview-line short" />
                        </div>
                    </div>
                    <Text className="appearance-mode-label">{mode.label}</Text>
                </div>)}
            </div>
        </div>
        <div id="appearance-accent-card" className="appearance-card" role="group" aria-label={data.accentTitle}>
            <Text className="appearance-heading">{data.accentTitle}</Text>
            <Text className="appearance-detail" wrap={true}>{data.accentDescription}</Text>
            <div className="appearance-swatches" role="radiogroup" aria-label={data.accentTitle}>
                {data.swatches.map(swatch => <ColorSwatch key={swatch.hue}
                    id={`appearance-accent-${swatch.hue}`} color={swatch.color}
                    selected={swatch.selected} accessibilityLabel={`${data.accentTitle}: ${swatch.hue}°`}
                    onClick={() => request('accent-hue', {hue: swatch.hue})} />)}
                <ColorSwatch id="appearance-accent-custom"
                    accessibilityLabel={data.customHueTitle}
                    onClick={() => request('open-custom-hue')} />
            </div>
        </div>
        <Dialog id="appearance-custom-hue-dialog" anchor="appearance-accent-custom"
            open={data.customHueOpen} width={360} height={216}>
            <div className="appearance-dialog">
                <Text className="appearance-heading">{data.customHueTitle}</Text>
                <Text className="appearance-detail" wrap={true}>{data.customHueDescription}</Text>
                <Text>{data.customHueField}</Text>
                <TextField id="appearance-custom-hue-input" className="appearance-hue-input"
                    value={data.customHueDraft} placeholder={data.customHuePlaceholder}
                    onChange={value => request('custom-hue-draft', {value})} />
                <div className="appearance-dialog-actions">
                    <Button id="appearance-custom-hue-apply" className="appearance-primary"
                        onClick={() => request('apply-custom-hue', {value: data.customHueDraft})}>{data.customHueApply}</Button>
                    <Button id="appearance-custom-hue-cancel"
                        onClick={() => request('cancel-custom-hue')}>{data.customHueCancel}</Button>
                </div>
            </div>
        </Dialog>
        <div id="appearance-wallpaper-card" className="appearance-card" role="group" aria-label={data.wallpaperTitle}>
            <Text className="appearance-heading">{data.wallpaperTitle}</Text>
            <Text className="appearance-detail" wrap={true}>{data.wallpaperDescription}</Text>
            <div className="appearance-wallpaper-row">
                <div id="appearance-wallpaper-preview" className="appearance-wallpaper-preview">
                    {data.wallpaperHasPreview ? <Image asset="wallpaper-preview" width={124} height={96} fit="cover" />
                        : <Text className="appearance-detail" wrap={true}>{data.wallpaperNone}</Text>}
                </div>
                <div className="appearance-wallpaper-controls">
                    <Text>{data.wallpaperName}</Text>
                    {data.wallpaperDimensions ? <Text className="appearance-detail" wrap={true}>{data.wallpaperDimensions}</Text> : null}
                    {data.wallpaperStatus ? <Text className="appearance-detail" wrap={true}>{data.wallpaperStatus}</Text> : null}
                    <div className="appearance-button-row">
                        <Button id="appearance-wallpaper-choose" className="appearance-primary"
                            onClick={() => request('wallpaper-choose')}>{data.wallpaperChoose}</Button>
                        <Button id="appearance-wallpaper-remove"
                            onClick={() => request('wallpaper-remove')}>{data.wallpaperRemove}</Button>
                    </div>
                </div>
            </div>
            <Text>{data.wallpaperFitTitle}</Text>
            <Select id="appearance-wallpaper-position" accessibilityLabel={data.wallpaperFitTitle}
                value={data.wallpaperPositionValue} open={data.wallpaperPositionExpanded}
                onClick={() => request('toggle-wallpaper-position')}>
                {data.wallpaperPositions.map(option => <Option key={option.id}
                    id={`appearance-wallpaper-position-${option.id}`}
                    onClick={() => request('wallpaper-position', {value: option.id})}>{option.label}</Option>)}
            </Select>
        </div>
        <div id="appearance-interface-card" className="appearance-card" role="group" aria-label={data.interfaceTitle}>
            <Text className="appearance-heading">{data.interfaceTitle}</Text>
            <div className="appearance-control-row">
                <div className="appearance-control-label">
                    <Text>{data.hueTitle}</Text><Text className="appearance-detail" wrap={true}>{data.hueDescription}</Text>
                </div>
                <Slider id="appearance-hue" accessibilityLabel={data.hueTitle}
                    value={data.hue / 359}
                    onChange={fraction => request('appearance-hue', {fraction})} />
                <Text>{data.hueValue}</Text>
            </div>
            <div className="appearance-control-row">
                <div className="appearance-control-label">
                    <Text>{data.intensityTitle}</Text><Text className="appearance-detail" wrap={true}>{data.intensityDescription}</Text>
                </div>
                <Slider id="appearance-intensity" accessibilityLabel={data.intensityTitle}
                    value={data.intensity / 100}
                    onChange={fraction => request('appearance-intensity', {fraction})} />
                <Text>{data.intensityValue}</Text>
            </div>
            <div className="appearance-control-row">
                <div className="appearance-control-label">
                    <Text>{data.transparencyTitle}</Text><Text className="appearance-detail" wrap={true}>{data.transparencyDescription}</Text>
                </div>
                <Switch id="appearance-transparency" accessibilityLabel={data.transparencyTitle}
                    state={data.reduceTransparency ? 'on' : 'off'}
                    onClick={() => request('reduce-transparency', {value: !data.reduceTransparency})} />
            </div>
            <div className="appearance-control-row">
                <div className="appearance-control-label">
                    <Text>{data.animationTitle}</Text><Text className="appearance-detail" wrap={true}>{data.animationDescription}</Text>
                </div>
                <Select id="appearance-animations" accessibilityLabel={data.animationTitle}
                    value={data.animationValue} open={data.animationExpanded}
                    onClick={() => request('toggle-animation-select')}>
                    {data.animations.map(option => <Option key={option.id}
                        id={`appearance-animation-${option.id}`}
                        onClick={() => request('animation', {value: option.id})}>{option.label}</Option>)}
                </Select>
            </div>
            <div className="appearance-control-row">
                <div className="appearance-control-label">
                    <Text>{data.fileArtworkTitle}</Text><Text className="appearance-detail" wrap={true}>{data.fileArtworkDescription}</Text>
                </div>
                <Select id="appearance-file-artwork" accessibilityLabel={data.fileArtworkTitle}
                    value={data.fileArtworkValue} open={data.fileArtworkExpanded}
                    onClick={() => request('toggle-file-artwork-select')}>
                    {data.fileArtworkOptions.map((option, index) => <Option key={option.id}
                        id={`appearance-file-artwork-option-${index}`}
                        onClick={() => request('file-artwork', {value: option.id})}>{option.label}</Option>)}
                </Select>
            </div>
        </div>
        <Button id="appearance-reset" onClick={() => request('appearance-reset')}>{data.resetLabel}</Button>
    </div>;
}
