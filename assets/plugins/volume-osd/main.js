// @jsx h
// The host projects the current level and redacts output names while locked.
function App() {
    const audio = nickel.data;
    return h(FixedWindow, { width: 420, height: 96, className: "volume-osd" },
        h(Column, { className: "volume-content" },
            h(Text, null, audio.label),
            h(Progress, { percent: audio.percent, width: 372, height: 8 })));
}
