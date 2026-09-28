// @jsx h
function App() {
    return <Action id="open-launcher" label="Open launcher"
        onClick={() => nickel.request('show-launcher')} />;
}
