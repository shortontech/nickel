// @jsx h
// The host projects the current level and redacts output names while locked.
function App() {
    const audio = nickel.data;
    return <FixedWindow width={420} height={96} className="volume-osd">
        <Column className="volume-content">
            <Text>{audio.label}</Text>
            <Progress percent={audio.percent} width={372} height={8} />
        </Column>
    </FixedWindow>;
}
