// @jsx h
// Candidate controls inside the host-owned association picker popover.
function App() {
    const data = nickel.data;
    return <div className="app-picker-page">
        <div className="app-picker-header">
            <Text className="app-picker-status" wrap={true}>{data.status}</Text>
            <TextField id={`default-app-handler-search-${data.row}`}
                className="app-picker-search" value={data.query}
                placeholder={data.searchPlaceholder}
                onChange={value => nickel.request({type: 'search-handlers', value})} />
        </div>
        <ScrollView id={`default-app-handler-scroll-${data.row}`} height={280}>
        <div className="app-picker-candidates">
            {data.handlerState ? <Text className="app-picker-detail">{data.handlerState}</Text> : null}
            {data.handlerHasPages ? <div className="app-picker-pages">
                <Button id="default-app-handler-previous" className="app-picker-action" disabled={!data.handlerCanPrevious}
                    onClick={() => nickel.request({type: 'page-handlers', direction: 'previous'})}>Previous</Button>
                <Text className="app-picker-detail">{data.handlerPageLabel}</Text>
                <Button id="default-app-handler-next" className="app-picker-action" disabled={!data.handlerCanNext}
                    onClick={() => nickel.request({type: 'page-handlers', direction: 'next'})}>Next</Button>
            </div> : null}
            {data.handlers.map(handler => <div key={handler.id} className="app-picker-row">
                <div className="app-picker-label">
                    <Text className="app-picker-name">{handler.name}</Text>
                    <Text className="app-picker-detail">{handler.detail}</Text>
                </div>
                <Button id={`default-app-handler-${handler.id}`}
                    className={handler.editable ? 'app-picker-action' : 'app-picker-action disabled'}
                    disabled={!handler.editable}
                    onClick={() => nickel.request({
                        type: 'choose-handler', row: data.row,
                        target: data.target, handler: handler.id
                    })}>{handler.current ? data.currentLabel : data.chooseLabel}</Button>
            </div>)}

        </div>
        </ScrollView>
    </div>;
}
