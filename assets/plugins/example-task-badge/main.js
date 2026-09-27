// @jsx h
// Replace this ID with the taskbar application ID you want to annotate.
function App() {
    return h(Badge, {
        item: "org.example.mail",
        label: "Unread mail",
        count: 3
    });
}
