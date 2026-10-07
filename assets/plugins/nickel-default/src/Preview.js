// @jsx h
import "./styles/preview.css";
export function Preview() {
    const data = useWindowPreviews();
    const windows = data.windows || [];
    const switcher = data.taskSwitcher;
    return h(Window, { id: "window-preview", placement: "fixed", width: "100%", height: "100%", className: "window-preview" },
        h("div", { className: "preview-content" }, windows.map((window, index) => {
            const title = window.title || window.applicationName || "Untitled window";
            const titleLimit = switcher ? 28 : 38;
            const displayTitle = title.length > titleLimit ? title.slice(0, titleLimit - 1) + "…" : title;
            const label = switcher ? `${title}, ${index + 1} of ${windows.length}${window.selected ? ", selected" : ""}` : title;
            const cardClass = (switcher ? "preview-card switcher-card" : "preview-card") + (window.selected ? " selected" : "");
            return h(Column, { key: window.id, className: cardClass },
                h(Row, { className: "preview-header" },
                    h(Text, { className: "preview-title" }, displayTitle),
                    window.canClose ? h(Button, { id: "preview-close-" + window.id, accessibilityLabel: "Close " + title, onClick: () => nickel.windowPreviews.close(window.id, data.revision) }, "\u00D7") : null),
                h(ImageButton, { id: "preview-window-" + window.id, asset: window.image, width: switcher ? 188 : 244, height: 116, fit: "contain", accessibilityLabel: label, onClick: () => nickel.windowPreviews.activate(window.id, data.revision), onContextMenu: () => nickel.windowPreviews.openMenu(window.id, data.revision) }));
        })));
}
