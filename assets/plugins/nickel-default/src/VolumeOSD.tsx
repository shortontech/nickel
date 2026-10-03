// @jsx h
import "./styles/volume-osd.css";

export function VolumeOSD() {
    const audio = nickel.audio.get();
    const output = audio.devices.find(device => device.isDefault);
    const label = (audio.muted ? "Muted" : "Volume " + audio.percent + "%")
        + (output ? " · " + output.name : "");
    return <FixedWindow id="volume-osd" width={420} height={96} edge="bottom" anchor="bottom-center" passive={true} className="volume-osd">
        <Column className="volume-content">
            <Text>{label}</Text>
            <Progress className="volume-progress" percent={audio.percent} width={372} height={8} />
        </Column>
    </FixedWindow>;
}
