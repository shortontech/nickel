// @jsx h
import { parseHexColor, toHexColor, toHsv, fromHsv } from "./colors.js";
import "./styles/color-picker.css";
export function ColorPicker({ id, label, value, allowAlpha = false, onChange }) {
    const [draft, setDraft] = useState({ source: value, text: value });
    const text = draft.source === value ? draft.text : value;
    const channels = parseHexColor(value);
    const hsv = channels && toHsv(channels);
    const adjust = (index, next) => {
        const updated = [...hsv];
        updated[index] = next;
        onChange(toHexColor(fromHsv(updated), allowAlpha));
    };
    const presets = ["#e64553", "#ed8c32", "#edcf41", "#52bd78", "#35becb", "#5187ef", "#9160dc", "#e35dad"];
    return h(Column, { className: "settings-color-picker" },
        h(Row, { className: "settings-color-presets" }, presets.map(color => h(ColorSwatch, { key: color, id: id + "/" + color.slice(1), color: color, selected: color === value, accessibilityLabel: label + ": " + color, onClick: () => onChange(allowAlpha && channels ? color + toHexColor(channels, true).slice(-2) : color) }))),
        hsv ? h(Column, { className: "settings-color-channels" },
            h(Text, null, "Hue"),
            h(Slider, { id: id + "/hue", accessibilityLabel: label + " hue", min: 0, max: 360, step: 1, value: hsv[0], onChange: next => adjust(0, next) }),
            h(Text, null, "Saturation"),
            h(Slider, { id: id + "/saturation", accessibilityLabel: label + " saturation", value: hsv[1], onChange: next => adjust(1, next) }),
            h(Text, null, "Brightness"),
            h(Slider, { id: id + "/brightness", accessibilityLabel: label + " brightness", value: hsv[2], onChange: next => adjust(2, next) }),
            allowAlpha ? h(Column, null,
                h(Text, null, "Opacity"),
                h(Slider, { id: id + "/alpha", accessibilityLabel: label + " opacity", value: hsv[3], onChange: next => adjust(3, next) })) : null) : null,
        h(Row, { className: "settings-color-entry" },
            h(TextField, { id: id + "/value", accessibilityLabel: label + " color value", value: text, onChange: next => setDraft({ source: value, text: next }) }),
            h(Button, { id: id + "/apply", disabled: !text || !text.trim(), onClick: () => onChange(text.trim()) }, "Apply")));
}
