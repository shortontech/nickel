// @jsx h
// The host projects the current level and redacts output names while locked.
function App() {
    const audio = nickel.data;
    return h(Panel, { height: 96, background: 0xf12b303c },
        h(Column, null,
            h(Text, null, audio.label),
            h(Progress, { percent: audio.percent, width: 372, height: 8 })));
}
