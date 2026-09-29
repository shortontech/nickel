// @jsx h
// Workspace and panel preferences rendered from ordinary Nickel components.
function App() {
    const data = nickel.data;
    return <div className="bar-card">
        <Text className="bar-heading">{data.showOn}</Text>
        <div className="bar-options">
            <Button id="bar-primary-display" className={data.barOnAllDisplays ? 'bar-option' : 'bar-option selected'}
                accessibilityLabel={data.primaryDisplay}
                onClick={() => nickel.request({type: 'display-scope', scope: 'primary'})}>{`${data.barOnAllDisplays ? '○' : '◉'}  ${data.primaryDisplay}`}</Button>
            <Button id="bar-all-displays" className={data.barOnAllDisplays ? 'bar-option selected' : 'bar-option'}
                accessibilityLabel={data.allDisplays}
                onClick={() => nickel.request({type: 'display-scope', scope: 'all'})}>{`${data.barOnAllDisplays ? '◉' : '○'}  ${data.allDisplays}`}</Button>
        </div>
        <Text className="bar-heading">{data.windowScope}</Text>
        <div className="bar-options">
            <Button id="bar-display-windows" className={data.allWindowsOnEveryBar ? 'bar-option' : 'bar-option selected'}
                accessibilityLabel={data.thisDisplay}
                onClick={() => nickel.request({type: 'window-scope', scope: 'display'})}>{`${data.allWindowsOnEveryBar ? '○' : '◉'}  ${data.thisDisplay}`}</Button>
            <Button id="bar-all-windows" className={data.allWindowsOnEveryBar ? 'bar-option selected' : 'bar-option'}
                accessibilityLabel={data.allWindows}
                onClick={() => nickel.request({type: 'window-scope', scope: 'all'})}>{`${data.allWindowsOnEveryBar ? '◉' : '○'}  ${data.allWindows}`}</Button>
        </div>
        <div className="bar-slider-section">
            <div className="bar-slider-heading">
                <Text className="bar-heading">{data.desktopsLabel}</Text>
                <Text className="bar-count">{data.desktopCountLabel}</Text>
            </div>
            <Slider id="bar-desktop-count" className="bar-slider"
                accessibilityLabel={data.desktopsLabel}
                value={(data.desktopCount - 1) / (data.maxDesktops - 1)}
                onChange={fraction => nickel.request({type: 'desktop-count', fraction})} />
            <Text className="bar-description" wrap={true}>The number of persistent workspaces available to the session.</Text>
        </div>
        <div className="bar-desktops">
            {Array.from({length: data.desktopCount}, (_, index) =>
                <div key={index} className={index === data.activeDesktop ? 'bar-desktop selected' : 'bar-desktop'}>
                    <Text className="bar-desktop-number">{index + 1}</Text>
                </div>)}
        </div>
    </div>;
}
