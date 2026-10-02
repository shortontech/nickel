/// <reference path="../nickel-plugin.d.ts" />
// @jsx h
function App() {
    return h(FixedWindow, { width: "100%", height: 36, output: "all", edge: "bottom", reserveWorkArea: true, className: "reserved-panel" },
        h(Text, null, "Reserved panel example"));
}
