// @jsx h
import { parseHexColor, toHexColor, toHsv, fromHsv } from "./colors.js";
import "./styles/color-picker.css";

export function ColorPicker({id, label, value, allowAlpha = false, onChange}) {
    const [draft, setDraft] = useState({source:value, text:value});
    const text = draft.source === value ? draft.text : value;
    const channels = parseHexColor(value);
    const hsv = channels && toHsv(channels);
    const adjust = (index, next) => {
        const updated = [...hsv];
        updated[index] = next;
        onChange(toHexColor(fromHsv(updated), allowAlpha));
    };
    const presets = ["#e64553", "#ed8c32", "#edcf41", "#52bd78", "#35becb", "#5187ef", "#9160dc", "#e35dad"];
    return <Column className="settings-color-picker">
        <Row className="settings-color-presets">
            {presets.map(color => <ColorSwatch key={color} id={id + "/" + color.slice(1)}
                color={color} selected={color === value} accessibilityLabel={label + ": " + color}
                onClick={() => onChange(allowAlpha && channels ? color + toHexColor(channels, true).slice(-2) : color)} />)}
        </Row>
        {hsv ? <Column className="settings-color-channels">
            <Text>Hue</Text>
            <Slider id={id + "/hue"} accessibilityLabel={label + " hue"} min={0} max={360} step={1} value={hsv[0]} onChange={next => adjust(0, next)} />
            <Text>Saturation</Text>
            <Slider id={id + "/saturation"} accessibilityLabel={label + " saturation"} value={hsv[1]} onChange={next => adjust(1, next)} />
            <Text>Brightness</Text>
            <Slider id={id + "/brightness"} accessibilityLabel={label + " brightness"} value={hsv[2]} onChange={next => adjust(2, next)} />
            {allowAlpha ? <Column>
                <Text>Opacity</Text>
                <Slider id={id + "/alpha"} accessibilityLabel={label + " opacity"} value={hsv[3]} onChange={next => adjust(3, next)} />
            </Column> : null}
        </Column> : null}
        <Row className="settings-color-entry">
            <TextField id={id + "/value"} accessibilityLabel={label + " color value"} value={text}
                onChange={next => setDraft({source:value, text:next})} />
            <Button id={id + "/apply"} disabled={!text || !text.trim()}
                onClick={() => onChange(text.trim())}>Apply</Button>
        </Row>
    </Column>;
}
