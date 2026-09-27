// @jsx h
// Ordinary Bar settings. The host owns persistence and shell topology checks.
function App() {
    const data = nickel.data;
    return <settings-bar>
        <settings-text label={data.showOn} />
        <settings-radio-group id="bar-display-scope">
            <settings-radio id="bar-primary-display" label={data.primaryDisplay}
                selected={!data.barOnAllDisplays}
                onClick={() => nickel.request({type: 'display-scope', scope: 'primary'})} />
            <settings-radio id="bar-all-displays" label={data.allDisplays}
                selected={data.barOnAllDisplays}
                onClick={() => nickel.request({type: 'display-scope', scope: 'all'})} />
        </settings-radio-group>
        <settings-text label={data.windowScope} />
        <settings-radio-group id="bar-window-scope">
            <settings-radio id="bar-display-windows" label={data.thisDisplay}
                selected={!data.allWindowsOnEveryBar}
                onClick={() => nickel.request({type: 'window-scope', scope: 'display'})} />
            <settings-radio id="bar-all-windows" label={data.allWindows}
                selected={data.allWindowsOnEveryBar}
                onClick={() => nickel.request({type: 'window-scope', scope: 'all'})} />
        </settings-radio-group>
        <settings-slider id="bar-desktop-count" label={data.desktopsLabel}
            value={data.desktopCountLabel}
            percent={(data.desktopCount - 1) / (data.maxDesktops - 1)}
            onChange={fraction => nickel.request({type: 'desktop-count', fraction})}>
            <settings-description label="The number of persistent workspaces available to the session." />
        </settings-slider>
        <settings-desktops>
            {Array.from({length: data.desktopCount}, (_, index) =>
                <settings-desktop key={index} count={index + 1}
                    selected={index === data.activeDesktop} />)}
        </settings-desktops>
    </settings-bar>;
}
