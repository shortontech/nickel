/// <reference path="../nickel-plugin.d.ts" />
// @jsx h
function App() {
    return <FixedWindow width="100%" height={36} output="all" edge="bottom"
        reserveWorkArea={true} className="reserved-panel">
        <Text>Reserved panel example</Text>
    </FixedWindow>;
}
