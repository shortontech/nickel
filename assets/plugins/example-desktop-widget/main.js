// @jsx h
function App() {
    const unread = nickel.data.settings?.unread ?? 12;
    return h(Widget, {
        label: "Unread mail",
        value: `${unread} messages`,
        percent: Math.min(100, unread * 5),
        color: 0xff80c7ff
    });
}
