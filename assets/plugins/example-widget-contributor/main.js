// @jsx h
function App() {
    return h(Widget, { label: "Unread mail", value: String(nickel.data.settings['unread-count']), percent: 50, color: 0xff80c0ff });
}
