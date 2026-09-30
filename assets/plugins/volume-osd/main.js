// @jsx h
// The host projects the current level and redacts output names while locked.
function App() {
    const audio = nickel.data.audio;
    return h(FixedWindow, { width: 420, height: 96, edge: "bottom", anchor: "bottom-center", className: "volume-osd" },
        h(Column, { className: "volume-content" },
            h(Text, null, audio.label),
            h(Progress, { className: "volume-progress", percent: audio.percent, width: 372, height: 8 })));
}
