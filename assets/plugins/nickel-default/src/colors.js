// Color math belongs to presentation, not a native Settings adapter.
export function parseHexColor(value) {
    if (typeof value !== "string" || !/^#(?:[\da-f]{3}|[\da-f]{4}|[\da-f]{6}|[\da-f]{8})$/i.test(value)) return null;
    let hex = value.slice(1);
    if (hex.length < 5) hex = [...hex].map(channel => channel + channel).join("");
    if (hex.length === 6) hex += "ff";
    return [0, 2, 4, 6].map(offset => parseInt(hex.slice(offset, offset + 2), 16) / 255);
}

export function toHexColor(channels, alpha = false) {
    return "#" + channels.slice(0, alpha ? 4 : 3).map(channel => Math.round(Math.max(0, Math.min(1, channel)) * 255).toString(16).padStart(2, "0")).join("");
}

export function toHsv([red, green, blue, alpha]) {
    const high = Math.max(red, green, blue), low = Math.min(red, green, blue), delta = high - low;
    let hue = 0;
    if (delta) hue = high === red ? ((green - blue) / delta + 6) % 6 : high === green ? (blue - red) / delta + 2 : (red - green) / delta + 4;
    return [hue * 60, high ? delta / high : 0, high, alpha];
}

export function fromHsv([hue, saturation, value, alpha]) {
    const sector = ((hue % 360) + 360) % 360 / 60;
    const chroma = value * saturation, secondary = chroma * (1 - Math.abs(sector % 2 - 1)), base = value - chroma;
    const rgb = sector < 1 ? [chroma, secondary, 0] : sector < 2 ? [secondary, chroma, 0] : sector < 3 ? [0, chroma, secondary] : sector < 4 ? [0, secondary, chroma] : sector < 5 ? [secondary, 0, chroma] : [chroma, 0, secondary];
    return [...rgb.map(channel => channel + base), alpha];
}
