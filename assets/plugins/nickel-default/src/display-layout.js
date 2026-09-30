export function arrangement(outputs) {
    const active = outputs.filter(output => output.enabled);
    if (!active.length) return {cards: [], scale: 1};
    const left = Math.min(...active.map(output => output.geometry.x));
    const top = Math.min(...active.map(output => output.geometry.y));
    const right = Math.max(...active.map(output => output.geometry.x + output.geometry.width));
    const bottom = Math.max(...active.map(output => output.geometry.y + output.geometry.height));
    const scale = Math.min(1, 680 / Math.max(1, right - left), 216 / Math.max(1, bottom - top));
    return {scale, cards: outputs.map(output => ({
        ...output,
        x: Math.round((output.geometry.x - left) * scale),
        y: Math.round((output.geometry.y - top) * scale),
        width: Math.max(120, Math.round(output.geometry.width * scale)),
        height: Math.max(80, Math.round(output.geometry.height * scale)),
    }))};
}
export function snapPlacement(moved, outputs) {
    const snap = 32;
    let x = moved.geometry.x, y = moved.geometry.y;
    let bestX = snap + 1, bestY = snap + 1;
    for (const other of outputs) {
        if (other.name === moved.name || !other.enabled) continue;
        const a = moved.geometry, b = other.geometry;
        if (y < b.y + b.height && y + a.height > b.y) {
            for (const candidate of [b.x + b.width, b.x - a.width]) {
                const distance = Math.abs(candidate - x);
                if (distance < bestX) { bestX = distance; x = candidate; }
            }
        }
        if (x < b.x + b.width && x + a.width > b.x) {
            for (const candidate of [b.y + b.height, b.y - a.height]) {
                const distance = Math.abs(candidate - y);
                if (distance < bestY) { bestY = distance; y = candidate; }
            }
        }
    }
    return {x: Math.round(x), y: Math.round(y)};
}
